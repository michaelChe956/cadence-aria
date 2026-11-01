use crate::product::json_store::{ProductStoreError, read_json};
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::IssueSharedWorktree;

mod issue_locks;
mod repo_locks;
mod shared_admission;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueWorktreeLockLease {
    pub worktree: IssueSharedWorktree,
    pub lease_id: String,
    pub acquired: bool,
}

/// 多仓仓维锁 lease：与 [`IssueWorktreeLockLease`] 同构，但作用域为单个逻辑仓库
/// （`(project, issue, logical_repository_id)` 三元键之一），异仓并行、同仓串行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoWorktreeLockLease {
    pub worktree: IssueSharedWorktree,
    pub lease_id: String,
    pub acquired: bool,
}

/// D1.3c 中央 writer 准入的归属 hint：来自入口参数的真实身份
/// （work_item_id / attempt 或 lease owner id），只做归属反推证据，
/// 不构成新授权。
struct SharedWorktreeOwnerHints<'a> {
    work_item_ids: Vec<&'a str>,
    owner_ids: Vec<&'a str>,
}

impl SharedWorktreeOwnerHints<'_> {
    fn empty() -> Self {
        Self {
            work_item_ids: Vec::new(),
            owner_ids: Vec::new(),
        }
    }
}

/// group 初始化 journal 的最小归属视图（只读 join 三字段；serde 默认忽略
/// 其余字段）。
///
/// **损坏内容不作为归属声明**：journal 是「路径界定归属」（`{plan_id}.json`
/// 或 `{plan_id}/{target}.json`）的残留产物，解析失败只说明该文件不再携带
/// 任何 owner-key 证据，按 REQ-WPS-03「缺失即视为已清」跳过——它不能也不需要
/// 阻止删除/准入推进（删除侧由 `purge_attempt_lock_residue` 按路径直接清掉）。
/// 真实 IO 错误（权限拒绝等）仍 fail-closed（[`read_journal_view_if_exists`]）。
#[derive(serde::Deserialize)]
struct SharedWorktreeJournalView {
    #[allow(dead_code)]
    project_id: String,
    #[allow(dead_code)]
    issue_id: String,
    plan_id: String,
    lock_work_item_id: String,
    worktree_lease_id: String,
}

fn read_issue_worktree(
    path: &std::path::Path,
    project_id: &str,
    issue_id: &str,
) -> Result<IssueSharedWorktree, ProductStoreError> {
    read_json(path).map_err(|error| match error {
        ProductStoreError::NotFound { .. } => ProductStoreError::NotFound {
            kind: "issue_shared_worktree",
            id: format!("{project_id}/{issue_id}"),
        },
        other => other,
    })
}

fn lock_owner_mismatch(issue_id: &str, work_item_id: &str) -> ProductStoreError {
    ProductStoreError::Conflict {
        kind: "issue_worktree_lock_owner",
        id: format!("{issue_id}/{work_item_id}"),
    }
}

fn read_repo_worktree(
    path: &std::path::Path,
    project_id: &str,
    issue_id: &str,
    repository_id: LogicalRepositoryId,
) -> Result<IssueSharedWorktree, ProductStoreError> {
    read_json(path).map_err(|error| match error {
        ProductStoreError::NotFound { .. } => ProductStoreError::NotFound {
            kind: "repo_shared_worktree",
            id: format!("{project_id}/{issue_id}/{}", repository_id.0),
        },
        other => other,
    })
}

fn repo_lock_owner_mismatch(
    repository_id: LogicalRepositoryId,
    work_item_id: &str,
) -> ProductStoreError {
    ProductStoreError::Conflict {
        kind: "repo_worktree_lock_owner",
        id: format!("{}/{}", repository_id.0, work_item_id),
    }
}
