use crate::product::coding_models::{CodingAttemptStatus, PushStatus};
use crate::product::issue_store::IssueStore;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::LifecycleWorkItemRecord;
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;

/// Issue 级交付状态的三种整体判定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueDeliveryOverall {
    /// 每个 Work Item 的最新 attempt 均 `Completed` 且最新 ReviewRequest 均 `Pushed`。
    AllPushed,
    /// 至少一个 Work Item 未满足「已推送」条件（含无 attempt / attempt 未完成 / push 失败）。
    Partial,
    /// Issue 下没有任何 Work Item。
    None,
}

/// 单个 Work Item 的交付状态投影。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryEntry {
    pub repository_name: String,
    pub work_item_id: String,
    /// `None` 表示该 Work Item 没有任何 attempt。
    pub attempt_status: Option<CodingAttemptStatus>,
    /// 取自最新 attempt 的 `branch_name`；无 attempt 时为 `None`。
    pub branch_name: Option<String>,
    /// 取自最新 attempt 的 `head_commit`；无 attempt 或尚未落盘时为 `None`。
    pub commit_sha: Option<String>,
    /// 取自最新 ReviewRequest 的 `push_status`；`None` 表示无 ReviewRequest。
    pub push_status: Option<PushStatus>,
    pub push_error: Option<String>,
}

/// Issue 级交付状态聚合结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDeliverySummary {
    pub project_id: String,
    pub issue_id: String,
    pub entries: Vec<DeliveryEntry>,
    pub overall: IssueDeliveryOverall,
}

impl super::CodingAttemptStore {
    /// 计算某个 issue 的交付状态聚合。
    ///
    /// 判定语义（每 Work Item 取 `list_attempts_for_work_item` 末元素为最新 attempt；
    /// ReviewRequest 取 `list_review_requests` 末元素的 `push_status`）：
    /// `attempt_status == Completed` 且 `push_status == Some(Pushed)` 才算已交付。
    /// 全部条目满足 → `AllPushed`；有条目不满足 → `Partial`；无 Work Item → `None`。
    pub fn compute_issue_delivery_summary(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<IssueDeliverySummary, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        // 读 issue 以确认其存在，避免对不存在的 issue 返回空聚合（与 T4 组装点一致）。
        IssueStore::new(self.paths()).get(project_id, issue_id)?;

        let work_items = LifecycleStore::new(self.paths()).list_work_items(project_id, issue_id)?;

        let mut entries = Vec::with_capacity(work_items.len());
        for work_item in &work_items {
            let latest_attempt = self
                .list_attempts_for_work_item(project_id, issue_id, &work_item.id)?
                .into_iter()
                .last();

            let attempt_status = latest_attempt
                .as_ref()
                .map(|attempt| attempt.status.clone());
            let branch_name = latest_attempt
                .as_ref()
                .map(|attempt| attempt.branch_name.clone());
            let commit_sha = latest_attempt
                .as_ref()
                .and_then(|attempt| attempt.head_commit.clone());

            let (push_status, push_error) = match latest_attempt.as_ref() {
                Some(attempt) => {
                    let latest_review = self
                        .list_review_requests(project_id, issue_id, &attempt.id)?
                        .into_iter()
                        .last();
                    match latest_review {
                        Some(review) => (Some(review.push_status), review.push_error),
                        None => (None, None),
                    }
                }
                None => (None, None),
            };

            let repository_name = self.resolve_repository_name(project_id, work_item)?;

            entries.push(DeliveryEntry {
                repository_name,
                work_item_id: work_item.id.clone(),
                attempt_status,
                branch_name,
                commit_sha,
                push_status,
                push_error,
            });
        }

        let overall = if entries.is_empty() {
            IssueDeliveryOverall::None
        } else if entries.iter().all(|entry| {
            entry.attempt_status == Some(CodingAttemptStatus::Completed)
                && entry.push_status == Some(PushStatus::Pushed)
        }) {
            IssueDeliveryOverall::AllPushed
        } else {
            IssueDeliveryOverall::Partial
        };

        Ok(IssueDeliverySummary {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            entries,
            overall,
        })
    }

    /// 解析 Work Item 的仓库展示名。
    ///
    /// 优先 `target_repository_id`（`Option<LogicalRepositoryId>`）经
    /// `RepositoryStore::resolve_logical_repository_strict` 解析出 checkout 路径末段目录名；
    /// `target_repository_id` 缺省或 checkout 路径无末段时回退 `repository_id` 字符串本身。
    fn resolve_repository_name(
        &self,
        project_id: &str,
        work_item: &LifecycleWorkItemRecord,
    ) -> Result<String, ProductStoreError> {
        if let Some(logical_id) = work_item.target_repository_id {
            let paths = self.paths();
            let project = ProjectStore::new(paths.clone()).get(project_id)?;
            let (_, checkout, _) = RepositoryStore::for_project(paths, &project)
                .resolve_logical_repository_strict(project_id, logical_id)?;
            if let Some(name) = checkout
                .canonical_path
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .filter(|value| !value.is_empty())
            {
                return Ok(name);
            }
        }
        Ok(work_item.repository_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::IssueDeliveryOverall;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
    use crate::product::coding_models::{
        CodingAttemptStatus, CodingExecutionAttempt, PushStatus, RemoteKind, ReviewRequest,
        ReviewRequestKind, ReviewRequestOwnerKind,
    };
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::json_store::{read_json, write_json};
    use crate::product::lifecycle_store::{CreateWorkItemInput, LifecycleStore};
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
        LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
        RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::models::{LifecycleWorkItemRecord, ProviderName, RepositoryRecord};
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";

    fn setup_store() -> (TempDir, CodingAttemptStore) {
        let tmp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
                multi_repo: false,
            })
            .unwrap();
        IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: PROJECT_ID.to_string(),
                repo_id: Some("repository_0001".to_string()),
                title: "issue".to_string(),
                description: None,
                change_id: None,
            })
            .unwrap();
        let store = CodingAttemptStore::new(paths);
        (tmp, store)
    }

    fn seed_work_item(store: &CodingAttemptStore, work_item_id: &str, repository_id: &str) {
        LifecycleStore::new(store.paths())
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: repository_id.to_string(),
                title: format!("work item {work_item_id}"),
                plan_status: crate::product::models::WorkItemPlanStatus::Confirmed,
                ..Default::default()
            })
            .unwrap();
    }

    fn provider_snapshot() -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        }
    }

    fn seed_completed_attempt(
        store: &CodingAttemptStore,
        work_item_id: &str,
        branch_name: &str,
        commit_sha: &str,
    ) -> CodingExecutionAttempt {
        let created = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: work_item_id.to_string(),
                base_branch: "main".to_string(),
                branch_name: branch_name.to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 2,
            })
            .unwrap();
        let attempt = CodingExecutionAttempt {
            status: CodingAttemptStatus::Completed,
            head_commit: Some(commit_sha.to_string()),
            ..created
        };
        store.write_coding_attempt_for_test(&attempt).unwrap();
        attempt
    }

    fn seed_review_request(
        store: &CodingAttemptStore,
        attempt: &CodingExecutionAttempt,
        push_status: PushStatus,
        push_error: Option<String>,
    ) {
        let request = ReviewRequest {
            id: format!("review_request_{}", Uuid::new_v4().simple()),
            attempt_id: attempt.id.clone(),
            kind: ReviewRequestKind::GitBranchOnly,
            remote_kind: RemoteKind::GenericGit,
            remote: "origin".to_string(),
            base_branch: "main".to_string(),
            branch_name: attempt.branch_name.clone(),
            commit_sha: attempt.head_commit.clone().unwrap_or_default(),
            push_status,
            external_url: None,
            manual_instructions: Vec::new(),
            push_error,
            owner_kind: ReviewRequestOwnerKind::Attempt,
            pointer_publication_id: None,
            revoked: false,
            created_at: "2026-08-13T00:00:00Z".to_string(),
            updated_at: "2026-08-13T00:00:00Z".to_string(),
        };
        store.save_review_request(attempt, &request).unwrap();
    }

    fn set_work_item_target_repository(
        store: &CodingAttemptStore,
        work_item_id: &str,
        logical_id: LogicalRepositoryId,
    ) {
        let path = store
            .paths()
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("work-items")
            .join(format!("{work_item_id}.json"));
        let mut record: LifecycleWorkItemRecord = read_json(&path).unwrap();
        record.target_repository_id = Some(logical_id);
        write_json(&path, &record).unwrap();
    }

    /// 播种 logical codebase 权威记录 + 兼容投影，使
    /// `resolve_logical_repository_strict` 能把 `target_repository_id` 解析到
    /// checkout 路径末段目录名 `checkout_repo_alpha`。
    fn seed_logical_codebase_for_repository(
        root: &Path,
        store: &CodingAttemptStore,
        physical_repository_id: &str,
    ) -> LogicalRepositoryId {
        let logical_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let repository_path = root.join("checkout_repo_alpha");
        let source_identity = RepositorySourceIdentity::from_git_parts(
            &repository_path,
            repository_path.join(".git"),
            None,
        );

        let authority = LogicalCodebaseStore::new(store.paths());
        authority
            .save_manifest(
                PROJECT_ID,
                &LogicalCodebaseManifest::new(
                    PROJECT_ID,
                    root.join("aggregate-root"),
                    vec![logical_id],
                ),
            )
            .unwrap();
        authority
            .save_member(
                PROJECT_ID,
                &CodebaseMemberRecord {
                    logical_repository_id: logical_id,
                    physical_repository_id: physical_repository_id.to_string(),
                    alias: "checkout_repo_alpha".to_string(),
                    role: "repository".to_string(),
                    ordinal: 1,
                    source_identity: source_identity.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: "2026-08-13T00:00:00Z".to_string(),
                    updated_at: "2026-08-13T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        authority
            .save_checkout(
                PROJECT_ID,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: logical_id,
                    physical_repository_id: physical_repository_id.to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: repository_path.clone(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: source_identity.git_dir_identity(),
                    revision: Some("abcdef".to_string()),
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-08-13T00:00:00Z".to_string(),
                    created_at: "2026-08-13T00:00:00Z".to_string(),
                    updated_at: "2026-08-13T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        write_json(
            &store.paths().project_root(PROJECT_ID).join("repos.json"),
            &[RepositoryRecord {
                id: physical_repository_id.to_string(),
                project_id: PROJECT_ID.to_string(),
                name: physical_repository_id.to_string(),
                path: repository_path.clone(),
                repo_hash: "sha256:repository".to_string(),
                runtime_root: repository_path.join(".aria/runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: "2026-08-13T00:00:00Z".to_string(),
                logical_repository_id: Some(logical_id),
                primary_checkout_id: Some(checkout_id),
                identity_schema_version: 1,
                updated_at: "2026-08-13T00:00:00Z".to_string(),
            }],
        )
        .unwrap();

        logical_id
    }

    #[test]
    fn all_pushed_returns_all_pushed() {
        let (_tmp, store) = setup_store();
        seed_work_item(&store, "work_item_0001", "repo_alpha");
        seed_work_item(&store, "work_item_0002", "repo_beta");

        let attempt1 =
            seed_completed_attempt(&store, "work_item_0001", "aria/w1/attempt-1", "sha111");
        seed_review_request(&store, &attempt1, PushStatus::Pushed, None);
        let attempt2 =
            seed_completed_attempt(&store, "work_item_0002", "aria/w2/attempt-1", "sha222");
        seed_review_request(&store, &attempt2, PushStatus::Pushed, None);

        let summary = store
            .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
            .unwrap();

        assert_eq!(summary.overall, IssueDeliveryOverall::AllPushed);
        assert_eq!(summary.entries.len(), 2);
        for entry in &summary.entries {
            assert_eq!(entry.attempt_status, Some(CodingAttemptStatus::Completed));
            assert_eq!(entry.push_status, Some(PushStatus::Pushed));
        }
        // 无 target_repository_id 时回退 repository_id 字符串。
        let repository_names: Vec<&str> = summary
            .entries
            .iter()
            .map(|entry| entry.repository_name.as_str())
            .collect();
        assert!(repository_names.contains(&"repo_alpha"));
        assert!(repository_names.contains(&"repo_beta"));
    }

    #[test]
    fn partial_when_one_push_failed() {
        let (_tmp, store) = setup_store();
        seed_work_item(&store, "work_item_0001", "repo_alpha");
        seed_work_item(&store, "work_item_0002", "repo_beta");

        let attempt1 =
            seed_completed_attempt(&store, "work_item_0001", "aria/w1/attempt-1", "sha111");
        seed_review_request(&store, &attempt1, PushStatus::Pushed, None);
        let attempt2 =
            seed_completed_attempt(&store, "work_item_0002", "aria/w2/attempt-1", "sha222");
        seed_review_request(
            &store,
            &attempt2,
            PushStatus::Failed,
            Some("push rejected".to_string()),
        );

        let summary = store
            .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
            .unwrap();

        assert_eq!(summary.overall, IssueDeliveryOverall::Partial);
        let failed = summary
            .entries
            .iter()
            .find(|entry| entry.work_item_id == "work_item_0002")
            .expect("failed work item entry");
        assert_eq!(failed.attempt_status, Some(CodingAttemptStatus::Completed));
        assert_eq!(failed.push_status, Some(PushStatus::Failed));
        assert_eq!(failed.push_error.as_deref(), Some("push rejected"));
    }

    #[test]
    fn partial_when_attempt_missing_or_not_completed() {
        let (_tmp, store) = setup_store();
        seed_work_item(&store, "work_item_0001", "repo_alpha");
        seed_work_item(&store, "work_item_0002", "repo_beta");

        // work_item_0001 无 attempt；work_item_0002 有 Running（非 Completed）attempt。
        let running = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0002".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/work-items/work_item_0002/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 2,
            })
            .unwrap();
        store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &running.id)
            .unwrap();

        let summary = store
            .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
            .unwrap();

        assert_eq!(summary.overall, IssueDeliveryOverall::Partial);
        assert_eq!(summary.entries.len(), 2);

        let missing = summary
            .entries
            .iter()
            .find(|entry| entry.work_item_id == "work_item_0001")
            .expect("missing attempt entry");
        assert_eq!(missing.attempt_status, None);
        assert_eq!(missing.branch_name, None);
        assert_eq!(missing.commit_sha, None);
        assert_eq!(missing.push_status, None);

        let not_completed = summary
            .entries
            .iter()
            .find(|entry| entry.work_item_id == "work_item_0002")
            .expect("not completed entry");
        assert_eq!(
            not_completed.attempt_status,
            Some(CodingAttemptStatus::Running)
        );
        assert_eq!(
            not_completed.branch_name.as_deref(),
            Some("aria/work-items/work_item_0002/attempt-1")
        );
        assert_eq!(not_completed.push_status, None);
    }

    #[test]
    fn none_when_issue_has_no_work_items() {
        let (_tmp, store) = setup_store();

        let summary = store
            .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
            .unwrap();

        assert_eq!(summary.overall, IssueDeliveryOverall::None);
        assert!(summary.entries.is_empty());
    }

    #[test]
    fn idempotent_recompute() {
        let (_tmp, store) = setup_store();
        seed_work_item(&store, "work_item_0001", "repo_alpha");
        let attempt =
            seed_completed_attempt(&store, "work_item_0001", "aria/w1/attempt-1", "sha111");
        seed_review_request(&store, &attempt, PushStatus::Pushed, None);

        let first = store
            .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
            .unwrap();
        let second = store
            .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
            .unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn repository_name_prefers_target_repository_checkout_path() {
        let (tmp, store) = setup_store();
        seed_work_item(&store, "work_item_0001", "repo_alpha");
        let logical_id =
            seed_logical_codebase_for_repository(tmp.path(), &store, "repository_0001");
        set_work_item_target_repository(&store, "work_item_0001", logical_id);

        let attempt =
            seed_completed_attempt(&store, "work_item_0001", "aria/w1/attempt-1", "sha111");
        seed_review_request(&store, &attempt, PushStatus::Pushed, None);

        let summary = store
            .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
            .unwrap();

        assert_eq!(summary.overall, IssueDeliveryOverall::AllPushed);
        assert_eq!(summary.entries.len(), 1);
        assert_eq!(summary.entries[0].repository_name, "checkout_repo_alpha");
    }

    #[test]
    fn ten_work_items_compute_within_one_second() {
        // 性能基线：10 个 Work Item（各含 Completed attempt + Pushed ReviewRequest）的
        // 聚合计算必须在 1s 内完成，防止 O(n²) 或逐文件全量扫描回归。
        let (_tmp, store) = setup_store();
        for index in 0..10 {
            let work_item_id = format!("work_item_{index:04}");
            let repository_id = format!("repo_{index:04}");
            seed_work_item(&store, &work_item_id, &repository_id);
            let attempt = seed_completed_attempt(
                &store,
                &work_item_id,
                &format!("aria/w{index}/attempt-1"),
                &format!("sha{index:03}"),
            );
            seed_review_request(&store, &attempt, PushStatus::Pushed, None);
        }

        let started = Instant::now();
        let summary = store
            .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
            .unwrap();
        let elapsed = started.elapsed();

        assert_eq!(summary.overall, IssueDeliveryOverall::AllPushed);
        assert_eq!(summary.entries.len(), 10);
        assert!(
            elapsed < Duration::from_secs(1),
            "aggregation took {elapsed:?}"
        );
    }
}
