//! issue 级自动化 enrollment 的独立持久存储（P0 1.2，REQ-WIGA-01/02）。
//!
//! 文件锁 + 原子 JSON 写实现 CAS：`compare_and_set` 在同一 issue 的
//! `automation-enrollment.json` 上线性化；同键同 payload 幂等返回原值，
//! 异 payload/旧 revision 冲突。P0 不创建 plan/session，也不启动 provider。

use std::path::{Path, PathBuf};

use chrono::Utc;
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::automation::{
    EnrollmentBindingHistory, EnrollmentBindingIdentity, EnrollmentCommandResult, EnrollmentError,
    EnrollmentRebindRequest, EnrollmentRebindResult, EnrollmentWriteCommand,
    IssueAutomationEnrollment, OperationState, PlanGenerationIntent, PlanGenerationPhase,
    PreparedPlanIntent,
};

/// issue 级 enrollment 的唯一持久入口；所有写路径都在目标 JSON 的伴生文件锁内
/// 完成「读、比 revision、写」，跨进程互斥。
#[derive(Debug, Clone)]
pub struct IssueAutomationStore {
    app_paths: ProductAppPaths,
}

/// 锁内 CAS 的判定结果；锁外统一映射为 `EnrollmentError`/成功值。
enum CasResolution {
    /// 同键同 payload（或已禁用时的重复 Disable）：返回原记录，revision 不变。
    Unchanged(IssueAutomationEnrollment),
    /// 首次写入/重开/绑定：已原子落盘。
    Applied(IssueAutomationEnrollment),
    /// revision 不匹配或绑定互斥；携带当前 revision 供 HTTP details。
    Conflict { current_revision: Option<u64> },
    /// 目标 enrollment 不存在（Disable/绑定的前置失败）。
    Missing,
}

impl IssueAutomationStore {
    pub fn new(app_paths: ProductAppPaths) -> Self {
        Self { app_paths }
    }

    fn enrollment_path(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(self
            .app_paths
            .issue_root(project_id, issue_id)
            .join("automation-enrollment.json"))
    }

    /// 读投影：无文件=`None`；文件存在但损坏/不可读 fail-closed 报错。
    pub fn get(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Option<IssueAutomationEnrollment>, ProductStoreError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        // read_json 的 Io 错误是格式化字符串（无可判 ErrorKind），缺失判定必须
        // 用 metadata 预检；读窗口与写并发的竞态由原子 rename 消解。
        if path.metadata().is_err() {
            return Ok(None);
        }
        let saved: IssueAutomationEnrollment = read_json(&path)?;
        Ok(Some(saved))
    }

    /// 以 `expected_revision` 线性化 enrollment 修订：`None` 仅对「无记录时的首次
    /// Enable」或「同键同 payload 幂等重试」合法；其余与当前 revision 不符即冲突。
    pub fn compare_and_set(
        &self,
        project_id: &str,
        issue_id: &str,
        expected_revision: Option<u64>,
        command: EnrollmentWriteCommand,
    ) -> Result<IssueAutomationEnrollment, EnrollmentError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        let resolution = with_exclusive_lock(&path, || {
            let existing = read_optional_enrollment(&path)?;
            let resolution = match (&existing, &command) {
                (
                    Some(saved),
                    EnrollmentWriteCommand::Enable {
                        selection_key,
                        source,
                        options,
                        logical_repository_id,
                        target,
                    },
                ) if saved.enabled
                    && saved.selection_key == *selection_key
                    && saved.source == *source
                    && saved.options == *options
                    && saved.logical_repository_id == *logical_repository_id
                    && saved.target == *target
                    && (expected_revision.is_none()
                        || expected_revision == Some(saved.policy_revision)) =>
                {
                    CasResolution::Unchanged(saved.clone())
                }
                // REQ-WIGA-02 fail-closed：enabled 状态下异 payload 的 Enable
                // 一律 Conflict（与是否携带当前 revision 无关）——绑定授权的
                // source/options/target 不可被原地改写；换 payload 必须先
                // Disable 重开（重开走 revision+1 换新授权并保留身份/绑定）。
                (
                    Some(saved),
                    EnrollmentWriteCommand::Enable {
                        selection_key,
                        source,
                        options,
                        logical_repository_id,
                        target,
                    },
                ) if saved.enabled
                    && (saved.selection_key != *selection_key
                        || saved.source != *source
                        || saved.options != *options
                        || saved.logical_repository_id != *logical_repository_id
                        || saved.target != *target) =>
                {
                    CasResolution::Conflict {
                        current_revision: Some(saved.policy_revision),
                    }
                }
                (Some(saved), _) if expected_revision != Some(saved.policy_revision) => {
                    CasResolution::Conflict {
                        current_revision: Some(saved.policy_revision),
                    }
                }
                (Some(saved), EnrollmentWriteCommand::Disable) if !saved.enabled => {
                    CasResolution::Unchanged(saved.clone())
                }
                (None, EnrollmentWriteCommand::Disable) => CasResolution::Missing,
                _ => apply_revision_and_write(&path, existing, command)?,
            };
            Ok(resolution)
        })?;
        resolve(resolution)
    }

    /// 绑定明确的 plan/session；同键幂等，异 plan/session 不可改写，旧 revision
    /// 拒绝。不按「最新 plan」推断——绑定只能由显式 POST 指定。
    pub fn bind_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        expected_revision: u64,
        plan_id: &str,
        session_id: &str,
    ) -> Result<IssueAutomationEnrollment, EnrollmentError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        validate_relative_id(plan_id)?;
        validate_relative_id(session_id)?;
        let resolution = with_exclusive_lock(&path, || {
            let Some(saved) = read_optional_enrollment(&path)? else {
                return Ok(CasResolution::Missing);
            };
            if expected_revision != saved.policy_revision {
                return Ok(CasResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                });
            }
            if !saved.enabled {
                return Ok(CasResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                });
            }
            bind_plan_ids_locked(&path, saved, plan_id, session_id)
        })?;
        resolve(resolution)
    }

    /// P2 Task 4：enrollment 文件锁内重读当前 enrollment 并执行同步闭包
    /// `f`（仅同步检查与 attempt 认领，禁止 await/provider 启动）。与 attempt
    /// 文件锁的固定顺序为 enrollment lock → attempt lock；「读 enrollment 后
    /// 放锁再写 claim」的窗口由此闭合（disable 与自动首启许可消费的单一
    /// 线性化点）。
    pub fn with_current_enrollment_locked<T>(
        &self,
        project_id: &str,
        issue_id: &str,
        f: impl FnOnce(&IssueAutomationEnrollment) -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        with_exclusive_lock(&path, || {
            let enrollment = read_optional_enrollment(&path)?.ok_or_else(|| {
                ProductStoreError::NotFound {
                    kind: "automation_enrollment",
                    id: format!("{project_id}/{issue_id}"),
                }
            })?;
            f(&enrollment)
        })
    }

    /// P1 WIGA Task 4：enrollment-bound 唯一创建与绑定补偿。
    ///
    /// 在 `automation-enrollment.json` 的同一文件锁内：重读 current（enabled、
    /// enrollment_id 一致）、读/写不可变 `automation-plan-intent.json`（既存
    /// 值与当前 enrollment 派生的新意图不一致即 Conflict，绝不覆盖——换源
    /// 重开天然 fail-closed）；完整绑定 → 幂等 Unchanged；单边/异绑定视为
    /// 损坏 Conflict 不修补；未绑定时由注入的 `create` 回调（共用 prepare
    /// 数据面）核对/创建 plan+session，成功后锁内绑定（revision+1）。create
    /// 失败原样穿出锁外，意图文件保留为下次补偿锚点。
    pub fn ensure_plan_binding<F>(
        &self,
        project_id: &str,
        issue_id: &str,
        enrollment_id: &str,
        create: F,
    ) -> Result<IssueAutomationEnrollment, EnrollmentError>
    where
        F: FnOnce(&IssueAutomationEnrollment, &PreparedPlanIntent) -> Result<(), EnrollmentError>,
    {
        let path = self.enrollment_path(project_id, issue_id)?;
        // 锁内闭包错误类型固定为 ProductStoreError：create 的 EnrollmentError
        // 以 Failed 包装穿出锁外，再由 resolve_ensure 还原。
        let resolution = with_exclusive_lock(&path, || {
            let Some(saved) = read_optional_enrollment(&path)? else {
                return Ok(EnsurePlanResolution::Missing);
            };
            if !saved.enabled || saved.enrollment_id != enrollment_id {
                return Ok(EnsurePlanResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                });
            }
            let intent = PreparedPlanIntent::from_enrollment(&saved);
            let intent_path = path.with_file_name("automation-plan-intent.json");
            if intent_path.metadata().is_ok() {
                let existing: PreparedPlanIntent = read_json(&intent_path)?;
                if existing != intent {
                    return Ok(EnsurePlanResolution::Conflict {
                        current_revision: Some(saved.policy_revision),
                    });
                }
            } else {
                write_json(&intent_path, &intent)?;
            }
            // P1 WIGA Task 10：测试注入的 intent 落盘中窗（创建回调之前）。
            #[cfg(test)]
            if automation_crash_window::fire_once(automation_crash_window::CrashWindow::AfterIntentSaved) {
                return Err(ProductStoreError::Io(
                    "automation_crash_window: interrupted after intent saved".to_string(),
                ));
            }
            match (saved.plan_id.as_deref(), saved.session_id.as_deref()) {
                (Some(plan), Some(session)) if plan == intent.plan_id && session == intent.session_id => {
                    return Ok(EnsurePlanResolution::Unchanged(saved));
                }
                // 半提交（单边）或异来源绑定：损坏 fail-closed，禁止覆盖/修补。
                (Some(_), _) | (_, Some(_)) => {
                    return Ok(EnsurePlanResolution::Conflict {
                        current_revision: Some(saved.policy_revision),
                    });
                }
                (None, None) => {}
            }
            if let Err(error) = create(&saved, &intent) {
                return Ok(EnsurePlanResolution::Failed(error));
            }
            // create 只写 plan/session；防御性重读并核对身份后锁内绑定。
            let Some(reloaded) = read_optional_enrollment(&path)? else {
                return Ok(EnsurePlanResolution::Conflict {
                    current_revision: None,
                });
            };
            if reloaded.enrollment_id != saved.enrollment_id {
                return Ok(EnsurePlanResolution::Conflict {
                    current_revision: Some(reloaded.policy_revision),
                });
            }
            // P1 WIGA Task 10：测试注入的绑定 session 落盘中窗（enrollment
            // 绑定写回之前）。
            #[cfg(test)]
            if automation_crash_window::fire_once(automation_crash_window::CrashWindow::AfterSessionSaved) {
                return Err(ProductStoreError::Io(
                    "automation_crash_window: interrupted after bound session saved".to_string(),
                ));
            }
            bind_plan_ids_locked(&path, reloaded, &intent.plan_id, &intent.session_id)
                .map(ensure_resolution_from_cas)
        })?;
        resolve_ensure(resolution)
    }

    /// P1 WIGA Task 5：认领 plan 生成动作检查点。在 enrollment 文件锁内从
    /// 当前 enrollment 冻结身份（含显式绑定 plan/session），首次落盘
    /// `automation-generation-intent.json`（phase=Claimed）；既存检查点身份
    /// 一致按其 phase 返回，身份漂移（换源重开等）fail-closed Conflict。
    pub fn claim_plan_generation(
        &self,
        project_id: &str,
        issue_id: &str,
        enrollment_id: &str,
    ) -> Result<PlanGenerationIntent, EnrollmentError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        let resolution = with_exclusive_lock(&path, || {
            let Some(saved) = read_optional_enrollment(&path)? else {
                return Ok(GenerationResolution::Missing);
            };
            if !saved.enabled || saved.enrollment_id != enrollment_id {
                return Ok(GenerationResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                });
            }
            let (Some(plan_id), Some(session_id)) =
                (saved.plan_id.clone(), saved.session_id.clone())
            else {
                return Ok(GenerationResolution::InvalidScope(
                    "plan generation requires a fully bound enrollment".to_string(),
                ));
            };
            let derived = PlanGenerationIntent {
                enrollment_id: saved.enrollment_id.clone(),
                action_key: PlanGenerationIntent::action_key_for(
                    &saved.enrollment_id,
                    &plan_id,
                ),
                plan_id,
                session_id,
                source: saved.source.clone(),
                options: saved.options.clone(),
                logical_repository_id: saved.logical_repository_id,
                phase: PlanGenerationPhase::Claimed,
            };
            let intent_path = path.with_file_name("automation-generation-intent.json");
            if intent_path.metadata().is_ok() {
                let existing: PlanGenerationIntent = read_json(&intent_path)?;
                if !existing.same_identity(&derived) {
                    return Ok(GenerationResolution::Conflict {
                        current_revision: Some(saved.policy_revision),
                    });
                }
                return Ok(GenerationResolution::Ready(existing));
            }
            write_json(&intent_path, &derived)?;
            Ok(GenerationResolution::Ready(derived))
        })?;
        resolve_generation(resolution)
    }

    /// P1 WIGA Task 5：推进检查点 phase（只能在身份一致的前置检查点上推进）。
    pub fn mark_plan_generation_phase(
        &self,
        project_id: &str,
        issue_id: &str,
        enrollment_id: &str,
        phase: PlanGenerationPhase,
    ) -> Result<PlanGenerationIntent, EnrollmentError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        let resolution = with_exclusive_lock(&path, || {
            let Some(saved) = read_optional_enrollment(&path)? else {
                return Ok(GenerationResolution::Missing);
            };
            if !saved.enabled || saved.enrollment_id != enrollment_id {
                return Ok(GenerationResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                });
            }
            let intent_path = path.with_file_name("automation-generation-intent.json");
            if intent_path.metadata().is_err() {
                return Ok(GenerationResolution::InvalidScope(
                    "plan generation checkpoint is missing".to_string(),
                ));
            }
            let mut existing: PlanGenerationIntent = read_json(&intent_path)?;
            let derived = PlanGenerationIntent {
                enrollment_id: saved.enrollment_id.clone(),
                action_key: PlanGenerationIntent::action_key_for(
                    &saved.enrollment_id,
                    existing.plan_id.as_str(),
                ),
                plan_id: existing.plan_id.clone(),
                session_id: existing.session_id.clone(),
                source: saved.source.clone(),
                options: saved.options.clone(),
                logical_repository_id: saved.logical_repository_id,
                phase: existing.phase,
            };
            if !existing.same_identity(&derived) {
                return Ok(GenerationResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                });
            }
            existing.phase = phase;
            write_json(&intent_path, &existing)?;
            Ok(GenerationResolution::Ready(existing))
        })?;
        resolve_generation(resolution)
    }

    /// P0 1.2（REQ-WIGA-08）：按同一精确绑定从 durable 事实计算会话归属——
    /// 仅 enrollment 显式绑定的 session 才是 server；无 enrollment/未绑定/
    /// 已关闭一律 client，关闭的绑定保留 enrollment_id/revision 供前端退位。
    pub fn ownership_for_session(
        &self,
        record: &crate::product::models::WorkspaceSessionRecord,
    ) -> Result<crate::product::models::automation::AutomationOwnership, ProductStoreError> {
        self.ownership_for_ids(&record.project_id, &record.issue_id, &record.id)
    }

    /// summary 投影入口（HTTP lifecycle 列表消费）：同一核心按
    /// (project_id, issue_id, session_id) 精确绑定计算，不另建归属口径。
    pub fn ownership_for_ids(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<crate::product::models::automation::AutomationOwnership, ProductStoreError> {
        Ok(match self.get(project_id, issue_id)? {
            None => crate::product::models::automation::AutomationOwnership::client_default(),
            Some(enrollment) => match enrollment.session_id.as_deref() {
                Some(bound_session) if bound_session == session_id => {
                    crate::product::models::automation::AutomationOwnership {
                        owner: if enrollment.enabled {
                            crate::product::models::automation::AutomationOwner::Server
                        } else {
                            crate::product::models::automation::AutomationOwner::Client
                        },
                        enrollment_id: Some(enrollment.enrollment_id),
                        policy_revision: Some(enrollment.policy_revision),
                        enabled: enrollment.enabled,
                    }
                }
                _ => crate::product::models::automation::AutomationOwnership::client_default(),
            },
        })
    }

    /// C1 Task 1（REQ-WIGA-01、REQ-C1-TARGET-01）：显式重绑/换代。在同一
    /// enrollment 文件锁内：先查命令账本（同 command 同 payload 幂等重放
    /// 首次 durable 结果、异 payload Conflict），再校验 enabled、expected
    /// policy/binding 版本与 target 授权域（跨载体/身份漂移 fail-closed，
    /// 旧 enrollment 无声明 target 不猜），最后追加 previous、写入新
    /// current（binding_version+1）并把 enrollment 投影同步到新代——
    /// 旧代身份只读保留，迟到旧回执按版本拒绝。
    pub fn rebind(
        &self,
        project_id: &str,
        issue_id: &str,
        request: EnrollmentRebindRequest,
    ) -> Result<EnrollmentRebindResult, EnrollmentError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        validate_relative_id(&request.binding.plan_id)?;
        validate_relative_id(&request.binding.session_id)?;
        let digest = request.payload_digest();
        let resolution = with_exclusive_lock(&path, || {
            let Some(mut saved) = read_optional_enrollment(&path)? else {
                return Ok(RebindResolution::Missing);
            };
            // 命令账本先判：同 command 同 payload 返回首次 durable 结果，
            // 异 payload 一律 Conflict（fail-closed，不猜哪个是"真的"）。
            if let Some(ledger) = saved
                .command_ledger
                .iter()
                .find(|entry| entry.command_id == request.command_id)
            {
                if ledger.payload_digest == digest {
                    return Ok(RebindResolution::Replayed {
                        command_id: request.command_id.clone(),
                        enrollment: saved,
                    });
                }
                return Ok(RebindResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                });
            }
            if !saved.enabled || request.expected_policy_revision != saved.policy_revision {
                return Ok(RebindResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                });
            }
            // target 授权域：无声明（旧 enrollment）不猜；跨载体/身份漂移拒绝。
            let Some(declared) = saved.target.clone() else {
                return Ok(RebindResolution::InvalidScope(
                    "rebind requires an enrollment with an explicitly declared target; \
                     re-enable with an explicit target first"
                        .to_string(),
                ));
            };
            if request.binding.target != declared {
                return Ok(RebindResolution::InvalidScope(format!(
                    "rebind target must match the enrollment's declared target \
                     carrier and identity: expected {declared:?}, got {:?}",
                    request.binding.target
                )));
            }
            let Some(history) = saved.binding_history.clone() else {
                return Ok(RebindResolution::InvalidScope(
                    "rebind requires durable binding history; re-enable with an \
                     explicit target first"
                        .to_string(),
                ));
            };
            if request.expected_binding_version != history.current.binding_version {
                return Ok(RebindResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                });
            }
            // 追加换代：previous 只读保留旧代完整身份；current 写新代；
            // enrollment 投影（plan/session/source/providers）同步到新代。
            let mut previous = history.previous;
            let binding_version = history.current.binding_version + 1;
            previous.push(history.current);
            saved.binding_history = Some(EnrollmentBindingHistory {
                current: EnrollmentBindingIdentity {
                    binding_version,
                    enrollment_id: saved.enrollment_id.clone(),
                    plan_id: request.binding.plan_id.clone(),
                    session_id: request.binding.session_id.clone(),
                    source: request.binding.source.clone(),
                    target: request.binding.target.clone(),
                    author_provider: request.binding.author_provider.clone(),
                    reviewer_provider: request.binding.reviewer_provider.clone(),
                },
                previous,
            });
            saved.plan_id = Some(request.binding.plan_id.clone());
            saved.session_id = Some(request.binding.session_id.clone());
            saved.source = request.binding.source.clone();
            saved.options.author_provider = request.binding.author_provider;
            saved.options.reviewer_provider = request.binding.reviewer_provider;
            saved.policy_revision += 1;
            saved.updated_at = now_rfc3339();
            saved.command_ledger.push(EnrollmentCommandResult {
                command_id: request.command_id.clone(),
                payload_digest: digest,
                state: OperationState::Accepted,
                binding_version,
            });
            write_json(&path, &saved)?;
            Ok(RebindResolution::Accepted {
                command_id: request.command_id.clone(),
                enrollment: saved,
            })
        })?;
        resolve_rebind(resolution)
    }
}

fn resolve(resolution: CasResolution) -> Result<IssueAutomationEnrollment, EnrollmentError> {
    match resolution {
        CasResolution::Unchanged(saved) | CasResolution::Applied(saved) => Ok(saved),
        CasResolution::Conflict { current_revision } => {
            Err(EnrollmentError::Conflict { current_revision })
        }
        CasResolution::Missing => Err(EnrollmentError::NotFound),
    }
}

/// 锁内绑定分支：同键幂等返回原值；异/半绑定 Conflict；(None,None) 才写入
/// 并 revision+1（P0 `bind_plan` 与 P1 `ensure_plan_binding` 共用，避免嵌套锁）。
fn bind_plan_ids_locked(
    path: &Path,
    saved: IssueAutomationEnrollment,
    plan_id: &str,
    session_id: &str,
) -> Result<CasResolution, ProductStoreError> {
    match (saved.plan_id.as_deref(), saved.session_id.as_deref()) {
        (Some(plan), Some(session)) if plan == plan_id && session == session_id => {
            Ok(CasResolution::Unchanged(saved))
        }
        (Some(_), _) | (_, Some(_)) => Ok(CasResolution::Conflict {
            current_revision: Some(saved.policy_revision),
        }),
        (None, None) => {
            let mut next = saved;
            next.plan_id = Some(plan_id.to_string());
            next.session_id = Some(session_id.to_string());
            // C1 Task 1：绑定补全 durable binding current 的 plan/session
            // 身份（enable 时以空串占位；版本不递增——绑定不是换代）。
            if let Some(history) = next.binding_history.as_mut() {
                history.current.plan_id = plan_id.to_string();
                history.current.session_id = session_id.to_string();
            }
            next.policy_revision += 1;
            next.updated_at = now_rfc3339();
            write_json(path, &next)?;
            Ok(CasResolution::Applied(next))
        }
    }
}

/// `ensure_plan_binding` 锁内判定；`Failed` 把 create 回调的 EnrollmentError
/// 原样穿出锁外（锁闭包错误类型固定为 ProductStoreError）。
enum EnsurePlanResolution {
    Unchanged(IssueAutomationEnrollment),
    Applied(IssueAutomationEnrollment),
    Conflict { current_revision: Option<u64> },
    Missing,
    Failed(EnrollmentError),
}

fn ensure_resolution_from_cas(resolution: CasResolution) -> EnsurePlanResolution {
    match resolution {
        CasResolution::Unchanged(saved) => EnsurePlanResolution::Unchanged(saved),
        CasResolution::Applied(saved) => EnsurePlanResolution::Applied(saved),
        CasResolution::Conflict { current_revision } => EnsurePlanResolution::Conflict {
            current_revision,
        },
        CasResolution::Missing => EnsurePlanResolution::Missing,
    }
}

fn resolve_ensure(
    resolution: EnsurePlanResolution,
) -> Result<IssueAutomationEnrollment, EnrollmentError> {
    match resolution {
        EnsurePlanResolution::Unchanged(saved) | EnsurePlanResolution::Applied(saved) => Ok(saved),
        EnsurePlanResolution::Conflict { current_revision } => {
            Err(EnrollmentError::Conflict { current_revision })
        }
        EnsurePlanResolution::Missing => Err(EnrollmentError::NotFound),
        EnsurePlanResolution::Failed(error) => Err(error),
    }
}

/// `claim_plan_generation`/`mark_plan_generation_phase` 的锁内判定。
enum GenerationResolution {
    Ready(PlanGenerationIntent),
    Conflict { current_revision: Option<u64> },
    Missing,
    InvalidScope(String),
}

fn resolve_generation(
    resolution: GenerationResolution,
) -> Result<PlanGenerationIntent, EnrollmentError> {
    match resolution {
        GenerationResolution::Ready(intent) => Ok(intent),
        GenerationResolution::Conflict { current_revision } => {
            Err(EnrollmentError::Conflict { current_revision })
        }
        GenerationResolution::Missing => Err(EnrollmentError::NotFound),
        GenerationResolution::InvalidScope(reason) => Err(EnrollmentError::InvalidScope(reason)),
    }
}

/// `rebind` 锁内判定；锁外映射为 `EnrollmentRebindResult`/`EnrollmentError`。
enum RebindResolution {
    Accepted {
        command_id: String,
        enrollment: IssueAutomationEnrollment,
    },
    Replayed {
        command_id: String,
        enrollment: IssueAutomationEnrollment,
    },
    Conflict {
        current_revision: Option<u64>,
    },
    InvalidScope(String),
    Missing,
}

fn resolve_rebind(
    resolution: RebindResolution,
) -> Result<EnrollmentRebindResult, EnrollmentError> {
    match resolution {
        RebindResolution::Accepted {
            command_id,
            enrollment,
        } => Ok(EnrollmentRebindResult {
            command_id,
            state: OperationState::Accepted,
            enrollment,
        }),
        RebindResolution::Replayed {
            command_id,
            enrollment,
        } => Ok(EnrollmentRebindResult {
            command_id,
            state: OperationState::Replayed,
            enrollment,
        }),
        RebindResolution::Conflict { current_revision } => {
            Err(EnrollmentError::Conflict { current_revision })
        }
        RebindResolution::InvalidScope(reason) => Err(EnrollmentError::InvalidScope(reason)),
        RebindResolution::Missing => Err(EnrollmentError::NotFound),
    }
}

/// 将 durable 归属注入 SessionState 帧（所有 manager 对外出口统一调用）。
/// 读取失败由调用方决定暴露方式：HTTP 明确报错；WS 帧置 None 保持未知。
pub fn project_session_automation(
    frame: &mut crate::web::workspace_ws_types::WsOutMessage,
    record: &crate::product::models::WorkspaceSessionRecord,
    store: &IssueAutomationStore,
) -> Result<(), ProductStoreError> {
    if let crate::web::workspace_ws_types::WsOutMessage::SessionState { automation, .. } = frame {
        *automation = Some(store.ownership_for_session(record)?);
    }
    Ok(())
}

fn read_optional_enrollment(
    path: &Path,
) -> Result<Option<IssueAutomationEnrollment>, ProductStoreError> {
    if path.metadata().is_err() {
        return Ok(None);
    }
    let saved: IssueAutomationEnrollment = read_json(path)?;
    Ok(Some(saved))
}

fn apply_revision_and_write(
    path: &Path,
    existing: Option<IssueAutomationEnrollment>,
    command: EnrollmentWriteCommand,
) -> Result<CasResolution, ProductStoreError> {
    let now = now_rfc3339();
    let next = match (existing, command) {
        (
            None,
            EnrollmentWriteCommand::Enable {
                selection_key,
                source,
                options,
                logical_repository_id,
                target,
            },
        ) => {
            let enrollment_id = Uuid::new_v4().to_string();
            // prepare_intent_id 与 enrollment 同源；P0 不消费该意图。
            let prepare_intent_id = enrollment_id.clone();
            // C1 Task 1：显式声明 target 时初始化 durable binding v1；
            // plan/session 尚未绑定（绑定写入时补全 current 身份）。
            let binding_history = target.as_ref().map(|declared| {
                initial_binding_history(
                    &enrollment_id,
                    None,
                    None,
                    &source,
                    declared,
                    &options,
                )
            });
            IssueAutomationEnrollment {
                enrollment_id,
                selection_key,
                project_id: path_project_id(path),
                issue_id: path_issue_id(path),
                enabled: true,
                policy_revision: 1,
                source,
                options,
                logical_repository_id,
                prepare_intent_id,
                plan_id: None,
                session_id: None,
                created_at: now.clone(),
                updated_at: now,
                target,
                binding_history,
                command_ledger: Vec::new(),
            }
        }
        // 重开（disabled 后换 payload 的启用）：保留 enrollment 身份与既有
        // 绑定，仅 revision+1。C1 Task 1：target 是授权域冻结事实——已声明
        // target 的漂移/撤销一律 fail-closed（换 target 只能走显式 rebind）。
        (
            Some(mut saved),
            EnrollmentWriteCommand::Enable {
                selection_key,
                source,
                options,
                logical_repository_id,
                target,
            },
        ) => {
            let target_drifted = match (&saved.target, &target) {
                // 旧 enrollment（无声明）重开时允许显式升级 target。
                (None, _) => false,
                // 撤销已声明的 target 或换 target：fail-closed（走显式 rebind）。
                (_, None) => true,
                (Some(declared), Some(next)) => declared != next,
            };
            if target_drifted {
                return Ok(CasResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                });
            }
            // 旧 enrollment（无声明）重开时显式升级 target → 初始化 v1。
            if saved.binding_history.is_none() {
                if let Some(declared) = target.as_ref() {
                    saved.binding_history = Some(initial_binding_history(
                        &saved.enrollment_id,
                        saved.plan_id.as_deref(),
                        saved.session_id.as_deref(),
                        &source,
                        declared,
                        &options,
                    ));
                }
            }
            saved.selection_key = selection_key;
            saved.source = source;
            saved.options = options;
            saved.logical_repository_id = logical_repository_id;
            saved.target = target;
            saved.enabled = true;
            saved.policy_revision += 1;
            saved.updated_at = now;
            saved
        }
        (Some(mut saved), EnrollmentWriteCommand::Disable) => {
            saved.enabled = false;
            saved.policy_revision += 1;
            saved.updated_at = now;
            saved
        }
        (None, EnrollmentWriteCommand::Disable) => return Ok(CasResolution::Missing),
    };
    write_json(path, &next)?;
    Ok(CasResolution::Applied(next))
}

/// C1 Task 1：从 durable enrollment 事实构造初始 binding v1（enable 显式
/// 声明 target 时）。plan/session 未绑定时以空串占位，绑定写入时补全。
fn initial_binding_history(
    enrollment_id: &str,
    plan_id: Option<&str>,
    session_id: Option<&str>,
    source: &crate::product::models::automation::EnrollmentSource,
    target: &crate::product::logical_codebase::EnrollmentTarget,
    options: &crate::product::models::automation::EnrollmentOptions,
) -> EnrollmentBindingHistory {
    EnrollmentBindingHistory {
        current: EnrollmentBindingIdentity {
            binding_version: 1,
            enrollment_id: enrollment_id.to_string(),
            plan_id: plan_id.unwrap_or_default().to_string(),
            session_id: session_id.unwrap_or_default().to_string(),
            source: source.clone(),
            target: target.clone(),
            author_provider: options.author_provider.clone(),
            reviewer_provider: options.reviewer_provider.clone(),
        },
        previous: Vec::new(),
    }
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn path_project_id(path: &Path) -> String {
    path.ancestors()
        .nth(3)
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn path_issue_id(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}
#[cfg(test)]
mod tests;

/// P1 WIGA Task 10（2.4）：自动化 plan 链四中窗的一次性中断替身。
///
/// 注册某窗口后，真实链路在该持久写落盘之后、下一步之前以错误中止
/// （进程崩溃替身）；「重启」由测试以全新 `WebAppState` 只凭 durable
/// 事实补偿来验证。仅测试构建编译，非测试二进制零代码。
#[cfg(test)]
pub(crate) mod automation_crash_window {
    use std::collections::BTreeSet;
    use std::sync::{LazyLock, Mutex};

    /// 自动化链持久写边界：intent 落盘后（创建回调之前）/ 绑定 plan 落盘
    /// 后（session 创建之前）/ 绑定 session 落盘后（enrollment 绑定写回
    /// 之前）/ 生成检查点 EngineStarted 落盘后（provider 派发之前）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub(crate) enum CrashWindow {
        AfterIntentSaved,
        AfterPlanSaved,
        AfterSessionSaved,
        AfterEngineStarted,
    }

    static WINDOWS: LazyLock<Mutex<BTreeSet<CrashWindow>>> =
        LazyLock::new(|| Mutex::new(BTreeSet::new()));

    fn windows() -> &'static Mutex<BTreeSet<CrashWindow>> {
        &WINDOWS
    }

    /// 注册守卫：drop（测试结束或「崩溃」）时清除未触发的注册，不跨测试泄漏。
    pub(crate) struct CrashWindowGuard(CrashWindow);

    impl Drop for CrashWindowGuard {
        fn drop(&mut self) {
            windows()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&self.0);
        }
    }

    pub(crate) fn register(window: CrashWindow) -> CrashWindowGuard {
        let inserted = windows()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(window);
        assert!(inserted, "crash window already registered: {window:?}");
        CrashWindowGuard(window)
    }

    /// 触发点：已注册则消费并返回 true（模拟进程在该持久写之后立即崩溃）。
    pub(crate) fn fire_once(window: CrashWindow) -> bool {
        windows()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&window)
    }
}
