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
    EnrollmentError, EnrollmentWriteCommand, IssueAutomationEnrollment, PlanGenerationIntent,
    PlanGenerationPhase, PreparedPlanIntent,
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
                    },
                ) if saved.enabled
                    && saved.selection_key == *selection_key
                    && saved.source == *source
                    && saved.options == *options
                    && saved.logical_repository_id == *logical_repository_id
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
                    },
                ) if saved.enabled
                    && (saved.selection_key != *selection_key
                        || saved.source != *source
                        || saved.options != *options
                        || saved.logical_repository_id != *logical_repository_id) =>
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
            },
        ) => {
            let enrollment_id = Uuid::new_v4().to_string();
            // prepare_intent_id 与 enrollment 同源；P0 不消费该意图。
            let prepare_intent_id = enrollment_id.clone();
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
            }
        }
        // 重开（或换 payload 的启用）：保留 enrollment 身份与既有绑定，仅 revision+1。
        (
            Some(mut saved),
            EnrollmentWriteCommand::Enable {
                selection_key,
                source,
                options,
                logical_repository_id,
            },
        ) => {
            saved.selection_key = selection_key;
            saved.source = source;
            saved.options = options;
            saved.logical_repository_id = logical_repository_id;
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
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use super::IssueAutomationStore;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::issue_automation_store::EnrollmentError;
    use crate::product::json_store::{read_json, write_json};
    use crate::product::logical_codebase::LogicalRepositoryId;
    use crate::product::models::automation::{
        EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, IssueAutomationEnrollment,
        SourceRevisionRef,
    };
    use crate::product::models::lifecycle::IssueWorkItemPlanOptions;
    use crate::product::models::provider::ProviderName;

    fn logical_repo(id: u16) -> LogicalRepositoryId {
        let uuid = format!("{id:08x}-0000-0000-0000-000000000000");
        LogicalRepositoryId(Uuid::parse_str(&uuid).unwrap())
    }

    fn source() -> EnrollmentSource {
        EnrollmentSource {
            stories: vec![SourceRevisionRef {
                id: "story_spec_0001".into(),
                version: 1,
            }],
            designs: vec![SourceRevisionRef {
                id: "design_spec_0001".into(),
                version: 1,
            }],
        }
    }

    fn options() -> EnrollmentOptions {
        EnrollmentOptions {
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            plan_options: IssueWorkItemPlanOptions {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
        }
    }

    fn enable(selection_key: &str, repository: LogicalRepositoryId) -> EnrollmentWriteCommand {
        EnrollmentWriteCommand::Enable {
            selection_key: selection_key.into(),
            source: source(),
            options: options(),
            logical_repository_id: repository,
        }
    }

    fn enable_with_options(
        selection_key: &str,
        repository: LogicalRepositoryId,
        options: EnrollmentOptions,
    ) -> EnrollmentWriteCommand {
        EnrollmentWriteCommand::Enable {
            selection_key: selection_key.into(),
            source: source(),
            options,
            logical_repository_id: repository,
        }
    }

    fn assert_conflict(error: crate::product::models::automation::EnrollmentError, expected: u64) {
        assert!(
            matches!(
                error,
                crate::product::models::automation::EnrollmentError::Conflict {
                    current_revision: Some(revision)
                } if revision == expected
            ),
            "expected Conflict at revision {expected}, got {error:?}"
        );
    }

    #[test]
    fn issue_automation_store_starts_empty_for_new_issue() {
        let tmp = tempfile::tempdir().unwrap();
        let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
        assert_eq!(store.get("project_1", "issue_1").unwrap(), None);
    }

    #[test]
    fn issue_automation_store_concurrent_identical_enable_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(ProductAppPaths::new(tmp.path()));
        let (tx, rx) = std::sync::mpsc::channel();

        for _ in 0..2 {
            let paths = Arc::clone(&paths);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let store = IssueAutomationStore::new((*paths).clone());
                let result = store.compare_and_set(
                    "project_1",
                    "issue_1",
                    None,
                    enable("human-choice-1", logical_repo(1)),
                );
                let _ = tx.send(result);
            });
        }
        drop(tx);
        let results: Vec<_> = rx.iter().collect();
        assert_eq!(results.len(), 2);
        let first = results[0].as_ref().unwrap();
        let second = results[1].as_ref().unwrap();
        assert_eq!(first.enrollment_id, second.enrollment_id);
        assert_eq!(first.policy_revision, 1);
        assert_eq!(second.policy_revision, 1);

        // durable 事实仅一条 JSON，且与内存返回一致。
        let on_disk: IssueAutomationEnrollment = read_json(
            &paths
                .issue_root("project_1", "issue_1")
                .join("automation-enrollment.json"),
        )
        .unwrap();
        assert_eq!(on_disk.enrollment_id, first.enrollment_id);
        assert_eq!(on_disk.prepare_intent_id, first.enrollment_id);
        assert!(on_disk.plan_id.is_none());
        assert!(on_disk.session_id.is_none());
        assert!(on_disk.enabled);
    }

    #[test]
    fn issue_automation_store_conflicts_on_divergent_payload() {
        let tmp = tempfile::tempdir().unwrap();
        let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
        let first = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();
        assert_eq!(first.policy_revision, 1);

        let mut other_options = options();
        other_options.review_rounds = 2;
        let error = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable_with_options("human-choice-1", logical_repo(1), other_options),
            )
            .unwrap_err();
        assert_conflict(error, 1);

        // 异 selection_key / 异 logical repository 同样冲突。
        let error = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-2", logical_repo(1)),
            )
            .unwrap_err();
        assert!(matches!(error, EnrollmentError::Conflict { .. }));
        let error = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-1", logical_repo(2)),
            )
            .unwrap_err();
        assert!(matches!(error, EnrollmentError::Conflict { .. }));

        // durable 记录未被覆盖。
        assert_eq!(store.get("project_1", "issue_1").unwrap().unwrap(), first);
    }

    #[test]
    fn issue_automation_store_disable_reenable_conflicts_and_preserves_binding() {
        let tmp = tempfile::tempdir().unwrap();
        let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));

        let created = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();
        assert_eq!(created.policy_revision, 1);

        let bound = store
            .bind_plan("project_1", "issue_1", 1, "plan_0001", "session_0001")
            .unwrap();
        assert_eq!(bound.plan_id.as_deref(), Some("plan_0001"));
        assert_eq!(bound.session_id.as_deref(), Some("session_0001"));

        let disabled = store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(bound.policy_revision),
                EnrollmentWriteCommand::Disable,
            )
            .unwrap();
        assert!(!disabled.enabled);
        assert_eq!(disabled.policy_revision, bound.policy_revision + 1);

        // 旧 revision 启用 → Conflict（关闭后未认领许可无效）。
        let error = store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(bound.policy_revision),
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap_err();
        assert_conflict(error, disabled.policy_revision);

        // 以当前 revision 重开：revision+1 且不抹绑定/prepare_intent。
        let reopened = store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(disabled.policy_revision),
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();
        assert!(reopened.enabled);
        assert_eq!(reopened.policy_revision, disabled.policy_revision + 1);
        assert_eq!(reopened.enrollment_id, created.enrollment_id);
        assert_eq!(reopened.prepare_intent_id, created.prepare_intent_id);
        assert_eq!(reopened.plan_id.as_deref(), Some("plan_0001"));
        assert_eq!(reopened.session_id.as_deref(), Some("session_0001"));
    }

    /// REQ-WIGA-02 契约（T2 Step 3）：enabled 状态下的 Enable 只允许同键同
    /// payload 幂等；异内容（source/options/target 任一不同）一律 Conflict——
    /// 即使携带当前 revision 也不得改写授权内容（改 payload 必须先 Disable
    /// 重开，重开才走 revision+1 换新授权）。
    #[test]
    fn issue_automation_store_rejects_enabled_payload_rewrite_with_current_revision() {
        let tmp = tempfile::tempdir().unwrap();
        let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
        let created = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();
        assert_eq!(created.policy_revision, 1);

        // 携当前 revision 的异 options 重写必须 Conflict，且 durable 不被改写。
        let mut other_options = options();
        other_options.review_rounds = 2;
        let error = store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(created.policy_revision),
                enable_with_options("human-choice-1", logical_repo(1), other_options),
            )
            .unwrap_err();
        assert_conflict(error, created.policy_revision);
        assert_eq!(store.get("project_1", "issue_1").unwrap().unwrap(), created);

        // 异 selection_key / 异 target 携当前 revision 同样冲突。
        let error = store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(created.policy_revision),
                enable("human-choice-2", logical_repo(1)),
            )
            .unwrap_err();
        assert_conflict(error, created.policy_revision);
        let error = store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(created.policy_revision),
                enable("human-choice-1", logical_repo(2)),
            )
            .unwrap_err();
        assert_conflict(error, created.policy_revision);

        // 对照：同键同 payload 携当前 revision 仍幂等返回原值。
        let idempotent = store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(created.policy_revision),
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();
        assert_eq!(idempotent, created);

        // 对照：Disable 后重开（disabled 态）允许新 payload，revision+1 且保留身份。
        let disabled = store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(created.policy_revision),
                EnrollmentWriteCommand::Disable,
            )
            .unwrap();
        assert!(!disabled.enabled);
        let mut reopened_options = options();
        reopened_options.review_rounds = 3;
        let reopened = store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(disabled.policy_revision),
                enable_with_options("human-choice-1", logical_repo(1), reopened_options),
            )
            .unwrap();
        assert!(reopened.enabled);
        assert_eq!(reopened.policy_revision, disabled.policy_revision + 1);
        assert_eq!(reopened.enrollment_id, created.enrollment_id);
        assert_eq!(reopened.options.review_rounds, 3);
    }

    #[test]
    fn issue_automation_store_bind_plan_is_idempotent_and_exclusive() {
        let tmp = tempfile::tempdir().unwrap();
        let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
        let created = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();

        let bound = store
            .bind_plan(
                "project_1",
                "issue_1",
                created.policy_revision,
                "plan_0001",
                "session_0001",
            )
            .unwrap();
        assert!(bound.policy_revision > created.policy_revision);
        let bound_revision = bound.policy_revision;

        // 同键重试：revision 不变，返回原值。
        let retried = store
            .bind_plan(
                "project_1",
                "issue_1",
                bound_revision,
                "plan_0001",
                "session_0001",
            )
            .unwrap();
        assert_eq!(retried.policy_revision, bound_revision);
        assert_eq!(retried, bound);

        // 重绑他者 plan 失败，绑定仍唯一。
        let error = store
            .bind_plan(
                "project_1",
                "issue_1",
                bound_revision,
                "plan_0002",
                "session_0002",
            )
            .unwrap_err();
        assert_conflict(error, bound_revision);
        let current = store.get("project_1", "issue_1").unwrap().unwrap();
        assert_eq!(current.plan_id.as_deref(), Some("plan_0001"));
        assert_eq!(current.session_id.as_deref(), Some("session_0001"));

        // 缺失 enrollment / 旧 revision 拒绝绑定。
        let error = store
            .bind_plan("project_1", "issue_9", 1, "plan_0001", "session_0001")
            .unwrap_err();
        assert!(matches!(error, EnrollmentError::NotFound));
        let error = store
            .bind_plan(
                "project_1",
                "issue_1",
                created.policy_revision,
                "plan_0001",
                "session_0001",
            )
            .unwrap_err();
        assert_conflict(error, bound_revision);
    }

    #[test]
    fn issue_automation_store_disable_requires_existing_enrollment() {
        let tmp = tempfile::tempdir().unwrap();
        let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
        let error = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                EnrollmentWriteCommand::Disable,
            )
            .unwrap_err();
        assert!(matches!(error, EnrollmentError::NotFound));
    }
    // ---- P1 WIGA Task 4：enrollment-bound 创建意图、唯一绑定与半提交恢复 ----

    fn intent_path(paths: &ProductAppPaths) -> std::path::PathBuf {
        paths
            .issue_root("project_1", "issue_1")
            .join("automation-plan-intent.json")
    }

    fn read_intent(
        paths: &ProductAppPaths,
    ) -> crate::product::models::automation::PreparedPlanIntent {
        read_json(&intent_path(paths)).unwrap()
    }

    #[test]
    fn issue_automation_store_ensure_plan_binding_binds_once_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(tmp.path());
        let store = IssueAutomationStore::new(paths.clone());
        let enrolled = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();

        let creates = std::sync::atomic::AtomicUsize::new(0);
        let bound = store
            .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |current, intent| {
                creates.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                assert_eq!(current.enrollment_id, enrolled.enrollment_id);
                assert_eq!(
                    intent.plan_id,
                    format!("issue_work_item_plan_auto_{}", current.prepare_intent_id)
                );
                assert_eq!(
                    intent.session_id,
                    format!("workspace_session_auto_{}", current.prepare_intent_id)
                );
                Ok(())
            })
            .unwrap();
        assert_eq!(bound.policy_revision, enrolled.policy_revision + 1);
        assert_eq!(
            bound.plan_id.as_deref(),
            Some(
                format!("issue_work_item_plan_auto_{}", enrolled.prepare_intent_id).as_str()
            )
        );
        assert_eq!(
            bound.session_id.as_deref(),
            Some(
                format!("workspace_session_auto_{}", enrolled.prepare_intent_id).as_str()
            )
        );

        // 重复补偿：完整绑定 → Unchanged 幂等，不再调用 create，revision 不再 +1。
        let again = store
            .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
                panic!("bound enrollment must not re-create");
            })
            .unwrap();
        assert_eq!(again.policy_revision, bound.policy_revision);
        assert_eq!(again.plan_id, bound.plan_id);
        assert_eq!(creates.load(std::sync::atomic::Ordering::SeqCst), 1);

        // 意图文件冻结且与 enrollment 派生一致。
        let intent = read_intent(&paths);
        assert_eq!(intent.enrollment_id, enrolled.enrollment_id);
        assert_eq!(intent.source, enrolled.source);
        assert_eq!(intent.options, enrolled.options);
    }

    /// 中窗恢复：create 失败（如 preflight/源校验）→ 失败原样穿出且不落绑定；
    /// 意图已先于 create 落盘，下一次补偿按同一冻结身份重建。
    #[test]
    fn issue_automation_store_ensure_plan_binding_recovers_after_create_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(tmp.path());
        let store = IssueAutomationStore::new(paths.clone());
        let enrolled = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();

        let error = store
            .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
                Err(EnrollmentError::InvalidScope("create failed".into()))
            })
            .unwrap_err();
        assert!(matches!(error, EnrollmentError::InvalidScope(_)));
        let current = store.get("project_1", "issue_1").unwrap().unwrap();
        assert!(current.plan_id.is_none());
        assert!(current.session_id.is_none());
        assert!(intent_path(&paths).exists());

        let bound = store
            .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| Ok(()))
            .unwrap();
        assert_eq!(bound.policy_revision, enrolled.policy_revision + 1);
    }

    /// Disable → 异 payload 重开（保留绑定/prepare_intent_id）：intent 文件
    /// 仍是旧冻结快照，与当前 enrollment 派生的新意图不一致 → fail-closed，
    /// 绝不把旧 plan/session 隐式重授权给新 payload。
    #[test]
    fn issue_automation_store_ensure_plan_binding_fails_closed_on_divergent_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(tmp.path());
        let store = IssueAutomationStore::new(paths.clone());
        let enrolled = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();
        let bound = store
            .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| Ok(()))
            .unwrap();
        assert_eq!(bound.policy_revision, enrolled.policy_revision + 1);

        let disabled = store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(enrolled.policy_revision + 1),
                EnrollmentWriteCommand::Disable,
            )
            .unwrap();
        let mut other_options = options();
        other_options.review_rounds = 2;
        let reopened = store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(disabled.policy_revision),
                enable_with_options("human-choice-1", logical_repo(1), other_options),
            )
            .unwrap();
        assert!(reopened.enabled);

        let error = store
            .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
                panic!("divergent reopen must not re-create");
            })
            .unwrap_err();
        assert_conflict(error, reopened.policy_revision);
        let current = store.get("project_1", "issue_1").unwrap().unwrap();
        assert_eq!(current.plan_id, bound.plan_id);
        assert_eq!(read_intent(&paths).options, options());
    }

    /// 半提交损坏（单边 plan_id/session_id）与禁用/缺失/异 enrollment_id fail-closed。
    #[test]
    fn issue_automation_store_ensure_plan_binding_rejects_corrupt_disabled_and_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(tmp.path());
        let store = IssueAutomationStore::new(paths.clone());

        let error = store
            .ensure_plan_binding("project_1", "issue_1", "missing", |_, _| Ok(()))
            .unwrap_err();
        assert!(matches!(error, EnrollmentError::NotFound));

        let enrolled = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();

        let disabled = store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(enrolled.policy_revision),
                EnrollmentWriteCommand::Disable,
            )
            .unwrap();
        let error = store
            .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
                panic!("disabled enrollment must not create");
            })
            .unwrap_err();
        assert_conflict(error, disabled.policy_revision);

        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(disabled.policy_revision),
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();
        let enrollment_file = paths
            .issue_root("project_1", "issue_1")
            .join("automation-enrollment.json");
        let mut corrupted = store.get("project_1", "issue_1").unwrap().unwrap();
        corrupted.plan_id = Some("plan_leftover".into());
        write_json(&enrollment_file, &corrupted).unwrap();
        let error = store
            .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
                panic!("half-bound enrollment must not create");
            })
            .unwrap_err();
        assert!(matches!(error, EnrollmentError::Conflict { .. }));

        let mut other = corrupted.clone();
        other.plan_id = None;
        write_json(&enrollment_file, &other).unwrap();
        let error = store
            .ensure_plan_binding("project_1", "issue_1", "another-enrollment", |_, _| Ok(()))
            .unwrap_err();
        assert!(matches!(error, EnrollmentError::Conflict { .. }));
    }

    /// P1 Task 5：生成检查点——首次认领 Claimed；EngineStarted 后重复认领
    /// 原样返回（进程重建按 phase 分诊）；换源重开身份漂移 fail-closed。
    #[test]
    fn issue_automation_store_plan_generation_checkpoint_phases_and_conflicts() {
        use crate::product::models::automation::{PlanGenerationPhase, PlanGenerationIntent};

        let tmp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(tmp.path());
        let store = IssueAutomationStore::new(paths.clone());
        let enrolled = store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();
        store
            .bind_plan("project_1", "issue_1", 1, "plan_bound", "session_bound")
            .unwrap();

        // 未绑定时拒绝认领。
        let unbound_dir = tempfile::tempdir().unwrap();
        let unbound_store = IssueAutomationStore::new(ProductAppPaths::new(unbound_dir.path()));
        let unbound = unbound_store
            .compare_and_set("project_2", "issue_1", None, enable("k", logical_repo(1)))
            .unwrap();
        let error = unbound_store
            .claim_plan_generation("project_2", "issue_1", &unbound.enrollment_id)
            .unwrap_err();
        assert!(matches!(error, EnrollmentError::InvalidScope(_)));

        let claimed = store
            .claim_plan_generation("project_1", "issue_1", &enrolled.enrollment_id)
            .unwrap();
        assert_eq!(claimed.phase, PlanGenerationPhase::Claimed);
        assert_eq!(claimed.plan_id, "plan_bound");
        assert_eq!(claimed.session_id, "session_bound");
        assert_eq!(
            claimed.action_key,
            PlanGenerationIntent::action_key_for(&enrolled.enrollment_id, "plan_bound")
        );

        // 重复认领幂等返回既存检查点（含已推进 phase）。
        store
            .mark_plan_generation_phase(
                "project_1",
                "issue_1",
                &enrolled.enrollment_id,
                PlanGenerationPhase::EngineStarted,
            )
            .unwrap();
        let resumed = store
            .claim_plan_generation("project_1", "issue_1", &enrolled.enrollment_id)
            .unwrap();
        assert_eq!(resumed.phase, PlanGenerationPhase::EngineStarted);

        // 禁用/异 enrollment_id 拒绝推进。
        let error = store
            .mark_plan_generation_phase(
                "project_1",
                "issue_1",
                "another-enrollment",
                PlanGenerationPhase::ProviderDispatched,
            )
            .unwrap_err();
        assert!(matches!(error, EnrollmentError::Conflict { .. }));

        // Disable → 异 payload 重开：派生身份漂移 → 认领 fail-closed。
        let bound_revision = store.get("project_1", "issue_1").unwrap().unwrap().policy_revision;
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(bound_revision),
                EnrollmentWriteCommand::Disable,
            )
            .unwrap();
        let disabled_revision = store.get("project_1", "issue_1").unwrap().unwrap().policy_revision;
        let mut other_options = options();
        other_options.review_rounds = 2;
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(disabled_revision),
                enable_with_options("human-choice-1", logical_repo(1), other_options),
            )
            .unwrap();
        let error = store
            .claim_plan_generation("project_1", "issue_1", &enrolled.enrollment_id)
            .unwrap_err();
        assert!(matches!(error, EnrollmentError::Conflict { .. }));
    }

    /// 双实例/双线程并发补偿同一 enrollment：恰一次 create+bind，另一侧 Unchanged。
    #[test]
    fn issue_automation_store_ensure_plan_binding_concurrent_workers_bind_once() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(ProductAppPaths::new(tmp.path()));
        let enrolled = IssueAutomationStore::new((*paths).clone())
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap();
        let enrollment_id = Arc::new(enrolled.enrollment_id.clone());
        let creates = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let (tx, rx) = std::sync::mpsc::channel();

        for _ in 0..2 {
            let paths = Arc::clone(&paths);
            let enrollment_id = Arc::clone(&enrollment_id);
            let creates = Arc::clone(&creates);
            let barrier = Arc::clone(&barrier);
            let tx = tx.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let store = IssueAutomationStore::new((*paths).clone());
                let result = store.ensure_plan_binding(
                    "project_1",
                    "issue_1",
                    &enrollment_id,
                    |_, _| {
                        creates.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(())
                    },
                );
                let _ = tx.send(result);
            });
        }
        drop(tx);
        let results: Vec<_> = rx.iter().collect();
        assert_eq!(results.len(), 2);
        let first = results[0].as_ref().unwrap();
        let second = results[1].as_ref().unwrap();
        assert_eq!(first.enrollment_id, second.enrollment_id);
        assert_eq!(first.plan_id, second.plan_id);
        assert_eq!(first.session_id, second.session_id);
        assert_eq!(first.policy_revision, second.policy_revision);
        assert_eq!(creates.load(std::sync::atomic::Ordering::SeqCst), 1);
        let on_disk: IssueAutomationEnrollment = read_json(
            &paths
                .issue_root("project_1", "issue_1")
                .join("automation-enrollment.json"),
        )
        .unwrap();
        assert_eq!(on_disk.plan_id, first.plan_id);
        assert_eq!(on_disk.session_id, first.session_id);
    }
}
