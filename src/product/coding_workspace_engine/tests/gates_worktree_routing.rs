//! gates 层 shared worktree 分流（Task 10）回归测试。
//!
//! 覆盖 REQ-COD-03（§4.2.3）与迁移契约化（§4.2.6）：
//! - 多仓 attempt（`target_snapshot = Some`）的 get/release 都作用在
//!   `shared-worktrees/{repository_id}.json`（三元键仓维路径）。
//! - 无快照 attempt（`target_snapshot = None`）一律 fail-closed：相对
//!   issue 归属 LC 时 `target_snapshot_missing_for_logical`，不读老
//!   `issue-shared-worktree.json`（单仓 legacy 分流已删除）。
//! - preflight 断言：多仓 issue 下存在旧 `issue-shared-worktree.json` →
//!   fail-closed `legacy_shared_worktree_present`；不存在 → 正常放行。
//! - 防御性分流：Logical routing 但 `target_snapshot = None` →
//!   `target_snapshot_missing_for_logical`。

use super::*;
use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::lifecycle_store::{
    UpsertIssueSharedWorktreeInput, UpsertRepoSharedWorktreeInput,
};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
use uuid::Uuid;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";

/// 构造「issue 归属 LC + manifest + selection」的 Logical routing 现场。
///
/// REQ-ROUTE-01 两态化后分流只读 issue 持久归属的 LC 子树（`for_lc`），
/// 因此 issue 记录必须先在位；`seed_complete` 保证 issue 先于 selection 落盘。
fn logical_routing_fixture() -> (tempfile::TempDir, ProductAppPaths, LogicalRepositoryId) {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let logical = crate::product::logical_codebase::logical_test_fixture::seed_complete(
        &paths,
        root.path(),
        PROJECT_ID,
        ISSUE_ID,
    );
    (root, paths, logical.member_id)
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

fn create_attempt(
    store: &CodingAttemptStore,
    target_snapshot: Option<AttemptTargetSnapshot>,
) -> CodingExecutionAttempt {
    store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            target_snapshot,
            max_auto_rework: 2,
        })
        .expect("create attempt")
}

fn engine(store: &CodingAttemptStore) -> CodingWorkspaceEngine {
    let (tx, _rx) = mpsc::channel(8);
    CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx)
}

#[tokio::test]
async fn attempt_worktree_path_uses_repo_shared_worktree_for_snapshot_attempt() {
    let (root, paths, logical_id) = logical_routing_fixture();
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = create_attempt(&store, Some(snapshot(logical_id)));
    let lifecycle = LifecycleStore::new(paths.clone());

    let worktree = root.path().join("worktree-a");
    fs::create_dir_all(&worktree).expect("worktree dir");
    lifecycle
        .upsert_repo_shared_worktree(UpsertRepoSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: logical_id,
            branch_name: attempt.branch_name.clone(),
            worktree_path: worktree.clone(),
            base_branch: attempt.base_branch.clone(),
        })
        .expect("repo shared worktree");

    let path = engine(&store)
        .attempt_worktree_path(&attempt)
        .await
        .expect("repo worktree path");
    assert_eq!(
        path, worktree,
        "多仓 attempt 必须读 shared-worktrees/{{repository_id}}.json"
    );
    // 单仓老文件未被创建/读取（三元键分流不触碰 issue-shared-worktree.json）。
    assert!(
        lifecycle
            .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
            .expect("legacy read")
            .is_none()
    );
}

#[test]
fn release_issue_shared_worktree_lock_for_attempt_releases_repo_worktree_for_snapshot_attempt() {
    let (_root, paths, logical_id) = logical_routing_fixture();
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = create_attempt(&store, Some(snapshot(logical_id)));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_repo_shared_worktree(UpsertRepoSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: logical_id,
            branch_name: attempt.branch_name.clone(),
            worktree_path: PathBuf::from("/tmp/worktree-a"),
            base_branch: attempt.base_branch.clone(),
        })
        .expect("repo shared worktree");
    lifecycle
        .try_acquire_repo_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            logical_id,
            &attempt.work_item_id,
            &attempt.id,
        )
        .expect("acquire repo lock");

    engine(&store)
        .release_issue_shared_worktree_lock_for_attempt(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )
        .expect("release by attempt");

    let record = lifecycle
        .get_repo_shared_worktree(&attempt.project_id, &attempt.issue_id, logical_id)
        .expect("read repo worktree")
        .expect("repo worktree exists");
    assert!(record.current_lock_owner_id.is_none());
    assert!(record.current_active_work_item_id.is_none());
    // 释放作用在仓维文件上，单仓老文件不受影响（不存在）。
    assert!(
        lifecycle
            .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
            .expect("legacy read")
            .is_none()
    );
}

#[tokio::test]
async fn attempt_worktree_path_fails_closed_for_none_attempt_and_ignores_legacy_worktree() {
    // REQ-ROUTE-01 两态化：单仓 legacy 分流已删除，老 issue-shared-worktree.json
    // 不得被静默读取；无快照 attempt 即使存在旧记录也必须 fail-closed。
    let (root, paths, _logical_id) = logical_routing_fixture();
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = create_attempt(&store, None);
    let lifecycle = LifecycleStore::new(paths.clone());

    let worktree = root.path().join("legacy-worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            branch_name: attempt.branch_name.clone(),
            worktree_path: worktree.clone(),
            base_branch: attempt.base_branch.clone(),
        })
        .expect("legacy shared worktree");

    let error = engine(&store)
        .attempt_worktree_path(&attempt)
        .await
        .expect_err("旧 issue-shared-worktree.json 不得被静默消费");
    assert!(
        error
            .to_string()
            .contains("target_snapshot_missing_for_logical"),
        "unexpected error: {error}"
    );
    // 旧文件保持原样，未被读取/改写。
    assert!(
        lifecycle
            .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
            .expect("legacy read")
            .is_some()
    );
}

#[tokio::test]
async fn preflight_fails_closed_when_legacy_shared_worktree_present() {
    let (root, paths, logical_id) = logical_routing_fixture();
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = create_attempt(&store, Some(snapshot(logical_id)));
    let lifecycle = LifecycleStore::new(paths.clone());

    // 模拟历史遗留：同 issue 下存在旧 issue-shared-worktree.json。
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            branch_name: attempt.branch_name.clone(),
            worktree_path: root.path().join("legacy-worktree"),
            base_branch: attempt.base_branch.clone(),
        })
        .expect("legacy shared worktree");

    let error = engine(&store)
        .attempt_worktree_path(&attempt)
        .await
        .expect_err("preflight must fail closed");
    assert!(
        error.to_string().contains("legacy_shared_worktree_present"),
        "unexpected error: {error}"
    );
    // 旧文件不被静默覆盖，也未被多仓路径删除。
    assert!(
        lifecycle
            .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
            .expect("legacy read")
            .is_some()
    );
}

#[tokio::test]
async fn preflight_passes_when_no_legacy_shared_worktree_for_snapshot_attempt() {
    let (root, paths, logical_id) = logical_routing_fixture();
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = create_attempt(&store, Some(snapshot(logical_id)));
    let lifecycle = LifecycleStore::new(paths.clone());

    let worktree = root.path().join("worktree-b");
    fs::create_dir_all(&worktree).expect("worktree dir");
    lifecycle
        .upsert_repo_shared_worktree(UpsertRepoSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: logical_id,
            branch_name: attempt.branch_name.clone(),
            worktree_path: worktree.clone(),
            base_branch: attempt.base_branch.clone(),
        })
        .expect("repo shared worktree");

    // 旧文件不存在 → preflight 放行，多仓路径正常访问。
    let path = engine(&store)
        .attempt_worktree_path(&attempt)
        .await
        .expect("preflight passes when no legacy file");
    assert_eq!(path, worktree);
}

#[tokio::test]
async fn none_snapshot_with_logical_routing_fails_closed() {
    let (_root, paths, _logical_id) = logical_routing_fixture();
    let store = CodingAttemptStore::new(paths.clone());
    // Logical routing（manifest + selection 已就绪）但 attempt 缺快照：
    // admission 正常已拦截，此处为分流处的防御性 fail-closed。
    let attempt = create_attempt(&store, None);

    let error = engine(&store)
        .attempt_worktree_path(&attempt)
        .await
        .expect_err("None snapshot with Logical routing must fail closed");
    assert!(
        error
            .to_string()
            .contains("target_snapshot_missing_for_logical"),
        "unexpected error: {error}"
    );
}
