// Task 11 删除调用链改造测试：拆分到独立文件，经 support.rs 的 `include!` 引入
// （large_file_guard 1200 行红线）。共享 `mod tests` 作用域内 `use super::*` 的导入。

use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::lifecycle_store::UpsertRepoSharedWorktreeInput;
use crate::product::logical_codebase::{
    IssueCodebaseSelection, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
    LogicalCodebaseStore, LogicalRepositoryId, RepositoryCheckoutId,
};
use tempfile::tempdir;
use uuid::Uuid;

const T11_PROJECT_ID: &str = "project_0001";
const T11_ISSUE_ID: &str = "issue_0001";

fn t11_provider_snapshot() -> ProviderConfigSnapshot {
    ProviderConfigSnapshot {
        author: ProviderName::Fake,
        reviewer: None,
        review_rounds: 0,
        permission_modes: Default::default(),
    }
}

fn t11_snapshot(logical_id: LogicalRepositoryId) -> AttemptTargetSnapshot {
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

fn t11_create_attempt(
    store: &CodingAttemptStore,
    work_item_id: &str,
    target_snapshot: Option<AttemptTargetSnapshot>,
) -> CodingExecutionAttempt {
    store
        .create_attempt(CreateCodingAttemptInput {
            project_id: T11_PROJECT_ID.to_string(),
            issue_id: T11_ISSUE_ID.to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: t11_provider_snapshot(),
            target_snapshot,
            max_auto_rework: 2,
        })
        .expect("create attempt")
}

fn t11_repo_worktree_input(
    repository_id: LogicalRepositoryId,
    worktree_path: PathBuf,
) -> UpsertRepoSharedWorktreeInput {
    UpsertRepoSharedWorktreeInput {
        project_id: T11_PROJECT_ID.to_string(),
        issue_id: T11_ISSUE_ID.to_string(),
        repository_id,
        branch_name: format!("worktree-{}", repository_id.0),
        worktree_path,
        base_branch: "main".to_string(),
    }
}

fn t11_logical_routing_fixture() -> (tempfile::TempDir, ProductAppPaths, LogicalRepositoryId) {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let logical_id = LogicalRepositoryId(Uuid::new_v4());
    LogicalCodebaseStore::new(paths.clone())
        .save_manifest(
            T11_PROJECT_ID,
            &LogicalCodebaseManifest::new(
                T11_PROJECT_ID,
                root.path().join("aggregate-root"),
                vec![logical_id],
            ),
        )
        .expect("save manifest");
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::all_members(
            T11_PROJECT_ID,
            T11_ISSUE_ID,
            None,
        ))
        .expect("save selection");
    (root, paths, logical_id)
}

/// 多仓 attempt 删除：按 snapshot.logical_repository_id 删仓维 worktree 文件。
#[test]
fn finalize_coding_attempt_deletion_removes_repo_worktree_for_snapshot_attempt() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store = CodingAttemptStore::new(paths.clone());
    let lifecycle = LifecycleStore::new(paths.clone());
    let logical_id = LogicalRepositoryId(Uuid::new_v4());
    let attempt = t11_create_attempt(
        &coding_store,
        "work_item_0001",
        Some(t11_snapshot(logical_id)),
    );
    lifecycle
        .upsert_repo_shared_worktree(t11_repo_worktree_input(
            logical_id,
            PathBuf::from("/tmp/worktree-a"),
        ))
        .expect("seed repo worktree");

    finalize_coding_attempt_deletion(&coding_store, &paths, &attempt)
        .expect("finalize deletion");

    assert!(
        lifecycle
            .get_repo_shared_worktree(&attempt.project_id, &attempt.issue_id, logical_id)
            .expect("read repo worktree")
            .is_none(),
        "多仓 attempt 删除必须按 snapshot 清仓维 worktree 文件"
    );
}

/// 同 logical repo 仍有其他活动 attempt → 保留仓维 worktree（同仓其他 item 复用）。
#[test]
fn finalize_coding_attempt_deletion_keeps_repo_worktree_for_active_same_repo_attempt() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store = CodingAttemptStore::new(paths.clone());
    let lifecycle = LifecycleStore::new(paths.clone());
    let logical_id = LogicalRepositoryId(Uuid::new_v4());
    let first = t11_create_attempt(
        &coding_store,
        "work_item_0001",
        Some(t11_snapshot(logical_id)),
    );
    let _second = t11_create_attempt(
        &coding_store,
        "work_item_0002",
        Some(t11_snapshot(logical_id)),
    );
    lifecycle
        .upsert_repo_shared_worktree(t11_repo_worktree_input(
            logical_id,
            PathBuf::from("/tmp/worktree-a"),
        ))
        .expect("seed repo worktree");

    finalize_coding_attempt_deletion(&coding_store, &paths, &first)
        .expect("finalize deletion");

    assert!(
        lifecycle
            .get_repo_shared_worktree(&first.project_id, &first.issue_id, logical_id)
            .expect("read repo worktree")
            .is_some(),
        "同 logical repo 仍有活动 attempt 时必须保留仓维 worktree"
    );
}

/// 单仓 attempt 删除走老逻辑：无其他 attempt 时清 issue-shared-worktree.json。
#[test]
fn finalize_coding_attempt_deletion_keeps_legacy_cleanup_for_none_attempt() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store = CodingAttemptStore::new(paths.clone());
    let lifecycle = LifecycleStore::new(paths.clone());
    let attempt = t11_create_attempt(&coding_store, "work_item_0001", None);
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: T11_PROJECT_ID.to_string(),
            issue_id: T11_ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "legacy-worktree".to_string(),
            worktree_path: PathBuf::from("/tmp/legacy-worktree"),
            base_branch: "main".to_string(),
        })
        .expect("seed legacy worktree");

    finalize_coding_attempt_deletion(&coding_store, &paths, &attempt)
        .expect("finalize deletion");

    assert!(
        lifecycle
            .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
            .expect("read legacy worktree")
            .is_none(),
        "单仓 attempt 删除必须继续走老 delete_issue_shared_worktree 逻辑"
    );
}

/// 多仓 issue/group plan 删除：枚举 shared-worktrees/ 全清。
#[test]
fn cleanup_shared_worktree_by_routing_enumerates_all_repo_worktrees_for_logical() {
    let (_root, paths, first_id) = t11_logical_routing_fixture();
    let second_id = LogicalRepositoryId(Uuid::new_v4());
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_repo_shared_worktree(t11_repo_worktree_input(
            first_id,
            PathBuf::from("/tmp/worktree-a"),
        ))
        .expect("seed first repo worktree");
    lifecycle
        .upsert_repo_shared_worktree(t11_repo_worktree_input(
            second_id,
            PathBuf::from("/tmp/worktree-b"),
        ))
        .expect("seed second repo worktree");

    cleanup_shared_worktree_by_routing(&paths, T11_PROJECT_ID, T11_ISSUE_ID)
        .expect("cleanup by routing");

    assert!(
        lifecycle
            .list_repo_shared_worktrees(T11_PROJECT_ID, T11_ISSUE_ID)
            .expect("list repo worktrees")
            .is_empty(),
        "多仓 issue 删除必须枚举 shared-worktrees/ 并全清"
    );
}

/// 单仓 issue/group plan 删除走老逻辑：清 issue-shared-worktree.json（不变）。
#[test]
fn cleanup_shared_worktree_by_routing_keeps_legacy_cleanup_for_legacy_routing() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: T11_PROJECT_ID.to_string(),
            issue_id: T11_ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "legacy-worktree".to_string(),
            worktree_path: PathBuf::from("/tmp/legacy-worktree"),
            base_branch: "main".to_string(),
        })
        .expect("seed legacy worktree");

    cleanup_shared_worktree_by_routing(&paths, T11_PROJECT_ID, T11_ISSUE_ID)
        .expect("cleanup by routing");

    assert!(
        lifecycle
            .get_issue_shared_worktree(T11_PROJECT_ID, T11_ISSUE_ID)
            .expect("read legacy worktree")
            .is_none(),
        "单仓 issue 删除必须继续走老 delete_issue_shared_worktree 逻辑"
    );
}

/// T11 fix round:生产路径级测试——coding 引擎 T6/T7 helper 把 `ProviderGatewayError`
/// 压成 `CodingWorkspaceEngineError::ProviderStream(e.to_string())`,e.to_string() 前缀
/// 即稳定码(`provider_gateway_*`)。转换点 `coding_workspace_api_error` 必须把它归一为
/// 稳定码(HTTP 409/403),而非 generic `coding_workspace_engine_failed`(500)。
#[test]
fn gateway_error_reaches_http_with_stable_code_via_provider_stream() {
    let cases = [
        (
            "provider_gateway_policy_missing: project_0001",
            "provider_gateway_policy_missing",
            StatusCode::CONFLICT,
        ),
        (
            "provider_gateway_capability: codex_danger_full_access_unsupported",
            "provider_gateway_capability",
            StatusCode::FORBIDDEN,
        ),
    ];

    for (details, expected_code, expected_status) in cases {
        let error = coding_workspace_api_error(CodingWorkspaceEngineError::ProviderStream(
            details.to_string(),
        ));
        assert_eq!(error.code, expected_code, "{details} code");
        assert_eq!(
            error.into_response().status(),
            expected_status,
            "{details} status mapping"
        );
    }
}

/// T11 fix round:coding 引擎 `start_streaming` 失败把 gateway 错误压成
/// `ProviderAdapterError{details=error.to_string()}`(经 `provider_adapter_error_from_gateway`),
/// 再经 `#[from]` 变成 `CodingWorkspaceEngineError::ProviderAdapter`。转换点必须同样
/// 从 details 前缀归一稳定码,而非 generic 500。
#[test]
fn gateway_error_reaches_http_with_stable_code_via_provider_adapter() {
    let error = coding_workspace_api_error(CodingWorkspaceEngineError::ProviderAdapter(
        crate::cross_cutting::provider_adapter::ProviderAdapterError::provider_unavailable(
            "provider_gateway_policy_drift: config_digest".to_string(),
        ),
    ));
    assert_eq!(error.code, "provider_gateway_policy_drift");
    assert_eq!(error.into_response().status(), StatusCode::CONFLICT);
}
