//! issue 级自动化 enrollment 的独立持久存储（P0 1.2，REQ-WIGA-01/02）。
//!
//! 文件锁 + 原子 JSON 写实现 CAS：`compare_and_set` 在同一 issue 的
//! `automation-enrollment.json` 上线性化；同键同 payload 幂等返回原值，
//! 异 payload/旧 revision 冲突。P0 不创建 plan/session，也不启动 provider。

use std::path::{Path, PathBuf};

use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::EnrollmentTarget;
use crate::product::models::automation::{
    AutomationAttentionFact, EnrollmentBindingHistory, EnrollmentBindingIdentity,
    EnrollmentCommandResult, EnrollmentError, EnrollmentOptions, EnrollmentRebindRequest,
    EnrollmentRebindResult, EnrollmentSource, EnrollmentTerminationRequest,
    EnrollmentTerminationResult, EnrollmentWriteCommand, IssueAutomationEnrollment, OperationState,
    PlanGenerationIntent, PlanGenerationPhase, PreparedPlanIntent,
};
use crate::product::termination_command_ledger::{
    TerminationLedgerVerdict, record_termination_ledger_accept, termination_ledger_verdict,
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
    /// REQ-WIGA-09 修订：terminated 人工重新开启缺稳定 command_id（422 面，
    /// 其余门禁不满足仍走 Conflict）。
    InvalidScope(String),
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
    /// 语义与 `compare_and_set_reenable(..., original_source_dangling=false)` 等价
    /// （terminated 重新开启维持 source/options/target 逐字段相等）。
    pub fn compare_and_set(
        &self,
        project_id: &str,
        issue_id: &str,
        expected_revision: Option<u64>,
        command: EnrollmentWriteCommand,
    ) -> Result<IssueAutomationEnrollment, EnrollmentError> {
        self.compare_and_set_inner(project_id, issue_id, expected_revision, command, false)
    }

    /// 悬空源重授权专用 CAS（terminated 人工重新开启）：`original_source_dangling`
    /// 由调用方核实「终止记录的原 source 引用实体已被级联删除」后置真——
    /// 此时仅放行 source 与终止记录的漂移（原授权失效，按本次提交的当前
    /// 已确认 source 显式重新授权）；options/target 仍逐字段相等。其余判定
    /// 与 `compare_and_set` 完全一致（账本重放/revision 线性化/幂等）。
    pub fn compare_and_set_reenable(
        &self,
        project_id: &str,
        issue_id: &str,
        expected_revision: Option<u64>,
        command: EnrollmentWriteCommand,
        original_source_dangling: bool,
    ) -> Result<IssueAutomationEnrollment, EnrollmentError> {
        self.compare_and_set_inner(
            project_id,
            issue_id,
            expected_revision,
            command,
            original_source_dangling,
        )
    }

    fn compare_and_set_inner(
        &self,
        project_id: &str,
        issue_id: &str,
        expected_revision: Option<u64>,
        command: EnrollmentWriteCommand,
        original_source_dangling: bool,
    ) -> Result<IssueAutomationEnrollment, EnrollmentError> {
        // 批 2 P1（REQ-WIGA-02 修订）：Enable 冻结前统一归一选项——auto
        // 授权把 require_execution_plan_confirm 压 false。归一先于幂等
        // 比较：同语义 payload（复选框值漂移）重试仍幂等；Disable 无选项
        // 原样通过。
        let command = match command {
            EnrollmentWriteCommand::Enable {
                command_id,
                selection_key,
                source,
                options,
                target,
            } => EnrollmentWriteCommand::Enable {
                command_id,
                selection_key,
                source,
                options: options.normalized_for_enrollment_freeze(),
                target,
            },
            command @ EnrollmentWriteCommand::Disable => command,
        };
        let path = self.enrollment_path(project_id, issue_id)?;
        let resolution = with_exclusive_lock(&path, || {
            let existing = read_optional_enrollment(&path)?;
            let resolution = match (&existing, &command) {
                // REQ-WIGA-09 修订：terminated 的人工显式 Enable 取代——
                // 判定与落盘见 `supersede_terminated_enrollment`（命令账本
                // 先判、稳定 command_id、revision 线性化、授权 payload
                // 逐字段相等——悬空源重授权的例外见其内部）。自动路径无从
                // 构造显式 revision＋command_id 的 Enable，本分支只可能由
                // 人工入口触达。
                (
                    Some(saved),
                    EnrollmentWriteCommand::Enable {
                        command_id,
                        selection_key,
                        source,
                        options,
                        target,
                    },
                ) if saved.terminated_at.is_some() => supersede_terminated_enrollment(
                    &path,
                    saved,
                    SupersedeEnableRequest {
                        command_id: command_id.as_deref(),
                        selection_key,
                        source,
                        options,
                        target,
                        expected_revision,
                        original_source_dangling,
                    },
                )?,
                // 批 2 T1（REQ-WIGA-09）：terminated 对其余写命令（含缺
                // 门禁的 Enable——由取代分支内部拒绝）一律 Conflict 零副
                // 作用；终止命令重放只走 terminate_plan 的账本分支。
                (Some(saved), _) if saved.terminated_at.is_some() => CasResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                },
                // REQ-WIGA-09 修订：Enable 命令身份的账本重放面——同
                // command_id 命中命令账本时先判：同 payload（含
                // expected_revision）返回首次 durable 结果的当前投影
                //（零写入），异 payload fail-closed（不猜哪个是"真的"）。
                // 先于幂等/revision 分支：取代落盘后的重放携带的是取代
                // 时的旧 revision。
                (
                    Some(saved),
                    EnrollmentWriteCommand::Enable {
                        command_id: Some(command_id),
                        selection_key,
                        source,
                        options,
                        target,
                    },
                ) if saved
                    .command_ledger
                    .iter()
                    .any(|entry| entry.command_id == *command_id) =>
                {
                    let digest = enable_command_payload_digest(
                        command_id,
                        selection_key,
                        source,
                        options,
                        target,
                        expected_revision,
                    );
                    let first = saved
                        .command_ledger
                        .iter()
                        .find(|entry| entry.command_id == *command_id)
                        .expect("ledger hit is guaranteed by the arm guard");
                    if first.payload_digest == digest {
                        CasResolution::Unchanged(saved.clone())
                    } else {
                        CasResolution::Conflict {
                            current_revision: Some(saved.policy_revision),
                        }
                    }
                }
                (
                    Some(saved),
                    EnrollmentWriteCommand::Enable {
                        command_id: _,
                        selection_key,
                        source,
                        options,
                        target,
                    },
                ) if saved.enabled
                    && saved.selection_key == *selection_key
                    && saved.source == *source
                    && saved.options == *options
                    && saved.target.as_ref() == Some(target)
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
                        command_id: _,
                        selection_key,
                        source,
                        options,
                        target,
                    },
                ) if saved.enabled
                    && (saved.selection_key != *selection_key
                        || saved.source != *source
                        || saved.options != *options
                        || saved.target.as_ref() != Some(target)) =>
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
                _ => apply_revision_and_write(&path, existing, command, expected_revision)?,
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
            let enrollment =
                read_optional_enrollment(&path)?.ok_or_else(|| ProductStoreError::NotFound {
                    kind: "automation_enrollment",
                    id: format!("{project_id}/{issue_id}"),
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
            // C5 Task 1：旧代 enrollment（无显式 target）不能驱动自动链——
            // 冻结意图的身份唯一来自 enrollment 声明的 target。
            let Some(declared) = saved.target.clone() else {
                return Ok(EnsurePlanResolution::InvalidScope(
                    "enrollment has no explicit target; legacy enrollments cannot drive \
                     automatic chains (re-enable with an explicit target)"
                        .to_string(),
                ));
            };
            let intent = PreparedPlanIntent::from_enrollment(&saved, &declared);
            let intent_path = path.with_file_name("automation-plan-intent.json");
            if intent_path.metadata().is_ok() {
                match read_prepared_plan_intent(&intent_path)? {
                    // 旧格式意图（无 target）与新派生意图永不一致：fail-closed
                    // Conflict，绝不覆盖、不迁移。
                    PreparedPlanIntentRead::LegacyUnbound { .. } => {
                        return Ok(EnsurePlanResolution::Conflict {
                            current_revision: Some(saved.policy_revision),
                        });
                    }
                    PreparedPlanIntentRead::Current(existing) if existing != intent => {
                        return Ok(EnsurePlanResolution::Conflict {
                            current_revision: Some(saved.policy_revision),
                        });
                    }
                    PreparedPlanIntentRead::Current(_) => {}
                }
            } else {
                write_json(&intent_path, &intent)?;
            }
            // P1 WIGA Task 10：测试注入的 intent 落盘中窗（创建回调之前）。
            #[cfg(test)]
            if automation_crash_window::fire_once(
                automation_crash_window::CrashWindow::AfterIntentSaved,
            ) {
                return Err(ProductStoreError::Io(
                    "automation_crash_window: interrupted after intent saved".to_string(),
                ));
            }
            match (saved.plan_id.as_deref(), saved.session_id.as_deref()) {
                (Some(plan), Some(session))
                    if plan == intent.plan_id && session == intent.session_id =>
                {
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
            if automation_crash_window::fire_once(
                automation_crash_window::CrashWindow::AfterSessionSaved,
            ) {
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
            // C5 Task 1：冻结身份唯一来自 enrollment 声明的 target；旧代
            // enrollment 无 target，自动链 fail-closed。
            let Some(declared) = saved.target.clone() else {
                return Ok(GenerationResolution::InvalidScope(
                    "enrollment has no explicit target; legacy enrollments cannot drive \
                     automatic chains (re-enable with an explicit target)"
                        .to_string(),
                ));
            };
            let derived = PlanGenerationIntent {
                enrollment_id: saved.enrollment_id.clone(),
                action_key: PlanGenerationIntent::action_key_for(&saved.enrollment_id, &plan_id),
                plan_id,
                session_id,
                source: saved.source.clone(),
                options: saved.options.clone(),
                target: declared,
                phase: PlanGenerationPhase::Claimed,
            };
            let intent_path = path.with_file_name("automation-generation-intent.json");
            if intent_path.metadata().is_ok() {
                match read_plan_generation_intent(&intent_path)? {
                    // 旧格式检查点（无 target）与新派生身份永不一致：
                    // fail-closed Conflict，绝不覆盖、不迁移。
                    PlanGenerationIntentRead::LegacyUnbound { .. } => {
                        return Ok(GenerationResolution::Conflict {
                            current_revision: Some(saved.policy_revision),
                        });
                    }
                    PlanGenerationIntentRead::Current(existing) => {
                        if !existing.same_identity(&derived) {
                            return Ok(GenerationResolution::Conflict {
                                current_revision: Some(saved.policy_revision),
                            });
                        }
                        return Ok(GenerationResolution::Ready(existing));
                    }
                }
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
            // C5 Task 1：旧格式检查点（无 target）不可推进：fail-closed。
            let mut existing = match read_plan_generation_intent(&intent_path)? {
                PlanGenerationIntentRead::Current(existing) => existing,
                PlanGenerationIntentRead::LegacyUnbound { .. } => {
                    return Ok(GenerationResolution::InvalidScope(
                        "plan generation checkpoint is a legacy intent without a target; \
                         it cannot drive automatic chains"
                            .to_string(),
                    ));
                }
            };
            // C5 Task 1：旧代 enrollment（无显式 target）不能驱动自动链。
            let Some(declared) = saved.target.clone() else {
                return Ok(GenerationResolution::InvalidScope(
                    "enrollment has no explicit target; legacy enrollments cannot drive \
                     automatic chains (re-enable with an explicit target)"
                        .to_string(),
                ));
            };
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
                target: declared,
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
                        // 批 2 T3（REQ-WIGA-08，清单外 additive 披露）：
                        // terminated 后仍 managed（owner 保持 Server）——
                        // 仅 Disable（未终止）退位 client；同源投影补绑定
                        // plan 与终止事实（REST/WS 一致）。
                        owner: if enrollment.enabled || enrollment.terminated_at.is_some() {
                            crate::product::models::automation::AutomationOwner::Server
                        } else {
                            crate::product::models::automation::AutomationOwner::Client
                        },
                        enrollment_id: Some(enrollment.enrollment_id.clone()),
                        policy_revision: Some(enrollment.policy_revision),
                        enabled: enrollment.enabled,
                        plan_id: enrollment.plan_id.clone(),
                        terminated_at: enrollment.terminated_at.clone(),
                    }
                }
                _ => crate::product::models::automation::AutomationOwnership::client_default(),
            },
        })
    }
}

// 物理拆分（large_file_guard 1200 行硬线）：终止/账本/attention 写命令与
// CAS 内部判定分文件，include! 后与本文件同模块域。
include!("issue_automation_store_parts/command_writes_impl.inc.rs");

include!("issue_automation_store_parts/cas_internals.inc.rs");

#[cfg(test)]
pub(crate) mod tests;

/// P1 WIGA Task 10（2.4）：自动化 plan 链四中窗的一次性中断替身。
///
/// 注册某窗口后，真实链路在该持久写落盘之后、下一步之前以错误中止
/// （进程崩溃替身）；「重启」由测试以全新 `WebAppState` 只凭 durable
/// 事实补偿来验证。仅测试构建编译，非测试二进制零代码。
#[cfg(test)]
pub(crate) mod automation_crash_window {
    use std::collections::{BTreeSet, HashMap};
    use std::sync::{LazyLock, Mutex};
    use std::thread::ThreadId;

    /// 自动化链持久写边界：intent 落盘后（创建回调之前）/ 绑定 plan 落盘
    /// 后（session 创建之前）/ 绑定 session 落盘后（enrollment 绑定写回
    /// 之前）/ 生成检查点 EngineStarted 落盘后（provider 派发之前）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    // `After*` 前缀是语义本身（「某持久化落盘之后」的崩溃窗口），20 处
    // 使用点跨 4 文件共享该命名口径，不做去前缀改名。
    #[allow(clippy::enum_variant_names)]
    pub(crate) enum CrashWindow {
        AfterIntentSaved,
        AfterPlanSaved,
        AfterSessionSaved,
        AfterEngineStarted,
    }

    /// 注册表按测试线程隔离：`cargo test` 并行跑多个驱动
    /// `ensure_enrolled_plan`/reconcile 的用例（web:: 全量下大量并行），
    /// 进程级单表会让无关线程的 `fire_once` 偷走本测试的窗口（本测试随后
    /// 得到 Ok 而断言「must abort」失败）。tokio 测试默认 current_thread
    /// 运行时，注册与触发都在测试线程上，按 `ThreadId` 分桶即恢复
    /// 「测试内一次性」语义；`#[cfg(test)]` 支持面，非测试二进制零代码。
    static WINDOWS: LazyLock<Mutex<HashMap<ThreadId, BTreeSet<CrashWindow>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    fn windows() -> &'static Mutex<HashMap<ThreadId, BTreeSet<CrashWindow>>> {
        &WINDOWS
    }

    /// 注册守卫：drop（测试结束或「崩溃」）时清除本线程未触发的注册，
    /// 不跨测试/跨线程泄漏。
    pub(crate) struct CrashWindowGuard(ThreadId, CrashWindow);

    impl Drop for CrashWindowGuard {
        fn drop(&mut self) {
            if let Some(set) = windows()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get_mut(&self.0)
            {
                set.remove(&self.1);
            }
        }
    }

    pub(crate) fn register(window: CrashWindow) -> CrashWindowGuard {
        let thread = std::thread::current().id();
        let inserted = windows()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(thread)
            .or_default()
            .insert(window);
        assert!(inserted, "crash window already registered: {window:?}");
        CrashWindowGuard(thread, window)
    }

    /// 触发点：本线程已注册则消费并返回 true（模拟进程在该持久写之后立即
    /// 崩溃）；其它线程的注册不可见、不可消费。
    pub(crate) fn fire_once(window: CrashWindow) -> bool {
        windows()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_mut(&std::thread::current().id())
            .is_some_and(|set| set.remove(&window))
    }
}
