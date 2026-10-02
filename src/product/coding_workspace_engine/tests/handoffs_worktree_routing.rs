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
use crate::product::lifecycle_store::{UpsertIssueSharedWorktreeInput, UpsertRepoSharedWorktreeInput};
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
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
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

/// 播种自持（owner == 本 attempt）的 issue 维 legacy 布局——缺陷 #13 层2
/// 现场：Logical 路由 group create（分流修复前）误写的形态。
fn seed_self_owned_legacy_layout(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    owner_attempt_id: &str,
) {
    crate::product::lifecycle_store::LifecycleStore::new(paths.clone())
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: attempt.target_snapshot.as_ref()
                .map(|snapshot| snapshot.physical_repository_id.clone())
                .unwrap_or_else(|| "repository_0001".to_string()),
            branch_name: attempt.branch_name.clone(),
            worktree_path: paths
                .issue_root(&attempt.project_id, &attempt.issue_id)
                .join("legacy-wt"),
            base_branch: attempt.base_branch.clone(),
        })
        .expect("seed legacy layout");
    crate::product::lifecycle_store::LifecycleStore::new(paths.clone())
        .try_acquire_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0001",
            "issue_worktree_lease_legacy_seed",
        )
        .expect("acquire legacy lease");
    crate::product::lifecycle_store::LifecycleStore::new(paths.clone())
        .bind_issue_worktree_lock_to_attempt(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0001",
            owner_attempt_id,
        )
        .expect("bind legacy lock owner");
}

/// 补齐 group attempt 的 plan binding + unit（终态转换
/// `update_group_terminal_status_locked` 的结构校验依赖）。
fn seed_group_plan_structure(
    paths: &ProductAppPaths,
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    let lineage = crate::product::models::WorkItemPlanLineage {
        id: "plan_0001".to_string(),
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        story_spec_refs: Vec::new(),
        design_spec_refs: Vec::new(),
        active_revision_id: Some("plan_revision_0001".to_string()),
        active_amendment_id: None,
        created_at: "2026-10-02T00:00:00Z".to_string(),
        updated_at: "2026-10-02T00:00:00Z".to_string(),
    };
    let revision_store =
        crate::product::work_item_revision_store::WorkItemRevisionStore::new(paths.clone());
    revision_store
        .put_plan_lineage(&lineage)
        .expect("seed plan lineage");
    revision_store
        .put_plan_revision(
            &lineage,
            &crate::product::models::WorkItemPlanRevision {
                id: "plan_revision_0001".to_string(),
                plan_id: "plan_0001".to_string(),
                revision_no: 1,
                supersedes: None,
                reason: crate::product::models::PlanRevisionReason::InitialCompile,
                work_item_bindings: std::collections::BTreeMap::from([(
                    "work_item_0001".to_string(),
                    "work_item_revision_0001".to_string(),
                )]),
                dependency_graph_revision_id: "dependency_graph_0001".to_string(),
                validation_report_ref: String::new(),
                plan_projection_bundle_id: "plan_projection_0001".to_string(),
                publication_provenance_ref: None,
                created_at: "2026-10-02T00:00:00Z".to_string(),
            },
        )
        .expect("seed plan revision");
    store
        .save_plan_binding(
            attempt,
            &crate::product::coding_models::CodingAttemptPlanBinding {
                attempt_id: attempt.id.clone(),
                plan_id: "plan_0001".to_string(),
                bound_plan_revision_id: "plan_revision_0001".to_string(),
                applied_amendment_ids: Vec::new(),
                updated_at: "2026-10-02T00:00:00Z".to_string(),
            },
        )
        .expect("seed plan binding");
    store
        .create_coding_unit(
            crate::product::coding_attempt_store::CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                plan_id: "plan_0001".to_string(),
                logical_work_item_id: "work_item_0001".to_string(),
                work_item_revision_id: "work_item_revision_0001".to_string(),
                dependency_logical_work_item_ids: Vec::new(),
                order_index: 0,
                status: crate::product::coding_models::CodingExecutionUnitStatus::Running,
            },
        )
        .expect("seed coding unit");
}

#[tokio::test]
async fn handle_delete_attempt_tolerates_self_owned_legacy_layout() {
    // 缺陷 #13 层2 出口：删除链遇 legacy_shared_worktree_present 且 record
    // 自持（owner == 本 attempt）时必须容忍——删除是错误布局的清理出口，
    // 不得被迁移断言拦死（恢复/删除双死锁）。断言面：不再死于
    // legacy_shared_worktree_present（推进到 group 终态结构校验——本
    // fixture 未铺 plan projection bundle 全链，结构校验失败属 fixture
    // 深度限制，非本修复语义）。
    let (_root, paths, logical_id) = logical_routing_fixture();
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = group_attempt(&store, logical_id);
    seed_group_plan_structure(&paths, &store, &attempt);
    seed_self_owned_legacy_layout(&paths, &attempt, &attempt.id);

    let result = engine(&store)
        .handle_delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await;

    match result {
        Ok(()) => {} // 全链通过（fixture 补齐后）——容忍语义成立。
        Err(error) => {
            let message = error.to_string();
            assert!(
                !message.contains("legacy_shared_worktree_present"),
                "self-owned legacy layout must be tolerated on delete, got: {message}"
            );
            assert!(
                message.contains("coding_group_attempt_incomplete"),
                "post-tolerance failure must come from group terminal structure checks, got: {message}"
            );
        }
    }
}

#[tokio::test]
async fn handle_delete_attempt_fails_closed_on_foreign_legacy_layout() {
    // 他人（owner != 本 attempt）的 legacy 残留仍 fail-closed：迁移契约
    // §4.2.6 语义不变，容忍仅限自持布局（无需 group plan 结构——owner
    // 校验在终态结构校验之前先行拒绝）。
    let (_root, paths, logical_id) = logical_routing_fixture();
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = group_attempt(&store, logical_id);
    seed_self_owned_legacy_layout(&paths, &attempt, "coding_attempt_someone_else");

    let error = engine(&store)
        .handle_delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("foreign legacy layout must stay fail-closed");

    assert!(
        error.to_string().contains("legacy_shared_worktree_present"),
        "foreign legacy layout must keep the migration contract, got: {error}"
    );
}
