use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::LogicalRepositoryId;
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
