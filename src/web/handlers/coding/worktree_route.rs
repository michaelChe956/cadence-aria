use std::path::PathBuf;

use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput;
use crate::product::lifecycle_store::UpsertRepoSharedWorktreeInput;
use crate::product::logical_codebase::LegacySharedWorktreeMigration;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::app_paths::ProductAppPaths;
use crate::web::error::ApiError;
use serde_json::json;

use super::super::support::product_store_api_error;

/// 多仓/单仓 issue shared worktree 分流（REQ-COD-01 分层 c）。
///
/// Legacy（单仓，`target_snapshot=None`）走 `issue-shared-worktree.json`，锁/绑/释放
/// 行为完全不变（红线）；Logical（多仓，`target_snapshot=Some`）走
/// `shared-worktrees/{repository_id}.json`（三元键仓维），锁/绑/释放均作用在
/// `attempt.target_snapshot.logical_repository_id` 解析出的目标仓 checkout 路径上。
pub(crate) enum IssueWorktreeRoute {
    Legacy,
    Repository { repository_id: LogicalRepositoryId },
}

impl IssueWorktreeRoute {
    pub(crate) fn from_target_snapshot(target_snapshot: &Option<AttemptTargetSnapshot>) -> Self {
        match target_snapshot {
            Some(snapshot) => Self::Repository {
                repository_id: snapshot.logical_repository_id,
            },
            None => Self::Legacy,
        }
    }
}

pub(crate) fn issue_worktree_active_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::Io(ref message) if message.contains("issue_worktree_active") => {
            ApiError::runtime(
                "issue_worktree_active",
                "another work item is already active on the issue shared worktree",
                json!({}),
            )
        }
        other => product_store_api_error(other),
    }
}

pub(crate) fn repo_worktree_active_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::Io(ref message) if message.contains("repo_worktree_active") => {
            ApiError::runtime(
                "repo_worktree_active",
                "another work item is already active on the repository shared worktree",
                json!({}),
            )
        }
        other => product_store_api_error(other),
    }
}

/// 按分流释放 worktree 锁；释放失败按原路径语义静默（与原 `let _ =` 一致）。
pub(crate) fn release_worktree_lock(
    lifecycle: &LifecycleStore,
    route: &IssueWorktreeRoute,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
    lease_id: &str,
) {
    let result = match route {
        IssueWorktreeRoute::Legacy => lifecycle
            .release_issue_worktree_lock(project_id, issue_id, work_item_id, lease_id)
            .map(|_| ()),
        IssueWorktreeRoute::Repository { repository_id } => lifecycle
            .release_repo_worktree_lock(
                project_id,
                issue_id,
                *repository_id,
                work_item_id,
                lease_id,
            )
            .map(|_| ()),
    };
    let _ = result;
}

#[derive(Debug)]
pub(crate) struct GroupWorktreeLease {
    pub(crate) acquired: bool,
    pub(crate) owner_attempt_id: Option<String>,
}

/// 按 worktree 路由分流执行「upsert shared worktree + 取 WI 级租约」
/// （REQ-COD-03 §4.2，与单件入口 `create_coding_attempt` 同一模式）。
///
/// Logical 路由（`target_snapshot=Some`）下 issue 维 legacy 布局与运行期
/// `preflight_repo_shared_worktree_absent` 契约相反（恢复 / handoff / 完成门
/// 均以 `legacy_shared_worktree_present` fail-closed），因此组入口 Logical 分支
/// 写仓维 record 并在发现旧布局残留时 fail-closed 422——不静默覆盖、不从旧
/// 文件推导 repository（迁移契约 §4.2.6 红线在组入口同样生效）。
pub(crate) fn upsert_worktree_and_acquire_lease(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    route: &IssueWorktreeRoute,
    project_id: &str,
    issue_id: &str,
    physical_repository_id: &str,
    lock_work_item_id: &str,
    worktree_lease_id: &str,
    branch_name: &str,
    base_branch: &str,
    worktree_path: PathBuf,
) -> Result<GroupWorktreeLease, ApiError> {
    match route {
        IssueWorktreeRoute::Legacy => {
            lifecycle
                .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
                    project_id: project_id.to_string(),
                    issue_id: issue_id.to_string(),
                    repository_id: physical_repository_id.to_string(),
                    branch_name: branch_name.to_string(),
                    worktree_path,
                    base_branch: base_branch.to_string(),
                })
                .map_err(product_store_api_error)?;
            let lease = lifecycle
                .try_acquire_issue_worktree_lock(
                    project_id,
                    issue_id,
                    lock_work_item_id,
                    worktree_lease_id,
                )
                .map_err(issue_worktree_active_api_error)?;
            Ok(GroupWorktreeLease {
                acquired: lease.acquired,
                owner_attempt_id: lease.worktree.current_lock_owner_id,
            })
        }
        IssueWorktreeRoute::Repository { repository_id } => {
            let legacy_error =
                match LegacySharedWorktreeMigration::load_legacy_shared_worktree(
                    app_paths,
                    project_id,
                    issue_id,
                ) {
                    Ok(None) => None,
                    Ok(Some(_)) => Some("legacy_shared_worktree_present"),
                    Err(ProductStoreError::InvalidRecord { reason, .. })
                        if reason.starts_with("legacy_shared_worktree_inconsistent:") =>
                    {
                        Some("legacy_shared_worktree_inconsistent")
                    }
                    Err(error) => return Err(product_store_api_error(error)),
                };
            if let Some(code) = legacy_error {
                return Err(ApiError::validation(
                    code,
                    "legacy issue shared worktree blocks the repository worktree route",
                ));
            }
            lifecycle
                .upsert_repo_shared_worktree(UpsertRepoSharedWorktreeInput {
                    project_id: project_id.to_string(),
                    issue_id: issue_id.to_string(),
                    repository_id: *repository_id,
                    branch_name: branch_name.to_string(),
                    worktree_path,
                    base_branch: base_branch.to_string(),
                })
                .map_err(product_store_api_error)?;
            let lease = lifecycle
                .try_acquire_repo_worktree_lock(
                    project_id,
                    issue_id,
                    *repository_id,
                    lock_work_item_id,
                    worktree_lease_id,
                )
                .map_err(repo_worktree_active_api_error)?;
            Ok(GroupWorktreeLease {
                acquired: lease.acquired,
                owner_attempt_id: lease.worktree.current_lock_owner_id,
            })
        }
    }
}

/// 按分流把 worktree 锁绑定到 attempt 名下（组入口与单件入口同一语义）。
pub(crate) fn bind_worktree_lock_to_attempt_routed(
    lifecycle: &LifecycleStore,
    route: &IssueWorktreeRoute,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
    attempt_id: &str,
) -> Result<(), ProductStoreError> {
    match route {
        IssueWorktreeRoute::Legacy => lifecycle
            .bind_issue_worktree_lock_to_attempt(project_id, issue_id, work_item_id, attempt_id)
            .map(|_| ()),
        IssueWorktreeRoute::Repository { repository_id } => lifecycle
            .bind_repo_worktree_lock_to_attempt(
                project_id,
                issue_id,
                *repository_id,
                work_item_id,
                attempt_id,
            )
            .map(|_| ()),
    }
}
