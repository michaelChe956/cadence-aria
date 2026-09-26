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
    EnrollmentError, EnrollmentWriteCommand, IssueAutomationEnrollment,
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
                    write_json(&path, &next)?;
                    Ok(CasResolution::Applied(next))
                }
            }
        })?;
        resolve(resolution)
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
    use crate::product::json_store::read_json;
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
        assert_eq!(
            store.get("project_1", "issue_1").unwrap().unwrap(),
            created
        );

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
}
