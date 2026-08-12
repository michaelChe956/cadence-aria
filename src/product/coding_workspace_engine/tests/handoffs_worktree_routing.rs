//! handoffs 层 WorkItemGroup abort/delete 分流回归（Task 11 park 兑现）。
//!
//! Task 10 邻接缺口：`handle_abort` / `handle_delete_attempt` 的 WorkItemGroup 分支直调
//! `get_issue_shared_worktree` 未分流。本测试锁定修复后行为：
//! - 多仓 attempt（target_snapshot = Some）读仓维 `shared-worktrees/{repository_id}.json`
//!   的 `active_work_item_id`（不是老 `issue-shared-worktree.json`）。
//! - 当仓维 record 的 active item 与 attempt 自身 item 不同时，abort/delete 必须按仓维
//!   record 的 active item 检查 worktree 干净度并触发 dirty gate（证明读的是仓维 record）。

use super::*;
use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::lifecycle_store::{LifecycleStore, UpsertRepoSharedWorktreeInput};
use crate::product::logical_codebase::{
    IssueCodebaseSelection, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
    LogicalCodebaseStore, LogicalRepositoryId, RepositoryCheckoutId,
};
use uuid::Uuid;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";

fn logical_routing_fixture() -> (tempfile::TempDir, ProductAppPaths, LogicalRepositoryId) {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let logical_id = LogicalRepositoryId(Uuid::new_v4());
    LogicalCodebaseStore::new(paths.clone())
        .save_manifest(
            PROJECT_ID,
            &LogicalCodebaseManifest::new(
                PROJECT_ID,
                root.path().join("aggregate-root"),
                vec![logical_id],
            ),
        )
        .expect("save manifest");
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::all_members(
            PROJECT_ID, ISSUE_ID, None,
        ))
        .expect("save selection");
    (root, paths, logical_id)
}

fn snapshot(logical_id: LogicalRepositoryId) -> AttemptTargetSnapshot {
    AttemptTargetSnapshot {
        logical_repository_id: logical_id,
        checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
        physical_repository_id: "repository_0001".to_string(),
        canonical_path: PathBuf::from("/tmp/repository_0001"),
        git_dir_identity: "sha256:test".to_string(),
        revision: Some("abcdef".to_string()),
        policy_digest: "policy_digest".to_string(),
        membership_revision: 1,
        captured_at: "2026-08-11T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    }
}

fn provider_snapshot() -> ProviderConfigSnapshot {
    ProviderConfigSnapshot {
        author: ProviderName::Fake,
        reviewer: None,
        review_rounds: 0,
        permission_modes: Default::default(),
    }
}

fn group_attempt(
    store: &CodingAttemptStore,
    logical_id: LogicalRepositoryId,
) -> CodingExecutionAttempt {
    store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            target_snapshot: Some(snapshot(logical_id)),
            max_auto_rework: 2,
        })
        .expect("create group attempt")
}

fn engine(store: &CodingAttemptStore) -> CodingWorkspaceEngine {
    let (tx, _rx) = mpsc::channel(8);
    CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx)
}

/// 建一个 dirty git worktree，并把仓维 record 的 active item 设成与 attempt 自身 item
/// 不同的 `work_item_group_active`，owner 绑定到 attempt。修复后 abort 必须读到仓维
/// record 的 active item 并触发 dirty gate；未分流时会回退 attempt 自身 item 而跳过。
fn prepare_dirty_repo_worktree(
    store: &CodingAttemptStore,
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    logical_id: LogicalRepositoryId,
) -> tempfile::TempDir {
    let worktree_root = tempdir().expect("worktree tempdir");
    super::init_test_git_repo(worktree_root.path());
    fs::write(worktree_root.path().join("dirty.txt"), "dirty\n").expect("dirty file");

    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_repo_shared_worktree(UpsertRepoSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: logical_id,
            branch_name: attempt.branch_name.clone(),
            worktree_path: worktree_root.path().to_path_buf(),
            base_branch: attempt.base_branch.clone(),
        })
        .expect("seed repo worktree");
    lifecycle
        .try_acquire_repo_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            logical_id,
            "work_item_group_active",
            "repo_worktree_lease_0001",
        )
        .expect("acquire repo lock");
    lifecycle
        .bind_repo_worktree_lock_to_attempt(
            &attempt.project_id,
            &attempt.issue_id,
            logical_id,
            "work_item_group_active",
            &attempt.id,
        )
        .expect("bind repo lock to attempt");
    let _ = store;
    worktree_root
}

#[tokio::test]
async fn handle_abort_reads_repo_worktree_active_item_for_snapshot_group_attempt() {
    let (_root, paths, logical_id) = logical_routing_fixture();
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = group_attempt(&store, logical_id);
    let _worktree = prepare_dirty_repo_worktree(&store, &paths, &attempt, logical_id);

    let error = engine(&store)
        .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("abort must fail closed on dirty repo worktree");

    assert!(
        error
            .to_string()
            .contains("shared_worktree_dirty_manual_gate"),
        "abort 必须按仓维 record 的 active_work_item_id 检查 worktree 干净度，实际: {error}"
    );
}

#[tokio::test]
async fn handle_delete_attempt_reads_repo_worktree_active_item_for_snapshot_group_attempt() {
    let (_root, paths, logical_id) = logical_routing_fixture();
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = group_attempt(&store, logical_id);
    let _worktree = prepare_dirty_repo_worktree(&store, &paths, &attempt, logical_id);

    let error = engine(&store)
        .handle_delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("delete must fail closed on dirty repo worktree");

    assert!(
        error
            .to_string()
            .contains("shared_worktree_dirty_manual_gate"),
        "delete 必须按仓维 record 的 active_work_item_id 检查 worktree 干净度，实际: {error}"
    );
}
