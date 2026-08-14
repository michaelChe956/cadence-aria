use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError;
use cadence_aria::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestData, ChoiceRequestSource, PermissionRequestData, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, ProviderSession, ProviderStatus, ProviderToolCall, ProviderToolResult,
    RiskLevel, StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{
    CodingAttemptStore, CreateCodingAttemptInput, CreateCodingExecutionUnitInput,
    CreateGroupCodingAttemptInput,
};
use cadence_aria::product::coding_models::{
    CodingAgentRole, CodingAttemptScope, CodingAttemptStatus, CodingChoiceGateStatus,
    CodingEntryType, CodingExecutionAttempt, CodingExecutionStage, CodingExecutionUnitStatus,
    CodingProviderPermissionMode, CodingProviderRole, CodingReworkInstruction,
    CodingRolePermissionModes, CodingRoleProviderConfigSnapshot,
    CodingRoleRunEventType, CodingRoleRunStatus, CodingRoleRunTrigger, CodingTimelineNode,
    CodingTimelineNodeStatus, FindingSeverity, PushStatus, RemoteKind, ReviewRequest,
    ReviewRequestKind, ReviewRequestOwnerKind, ReviewVerdict,
};
use cadence_aria::product::coding_workspace_engine::{
    CodingExecutionContext, CodingWorkspaceEngine,
};
use cadence_aria::product::git_workspace_service::GitWorkspaceService;
use cadence_aria::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateVerificationPlanInput, CreateWorkItemInput,
    CreateWorkspaceSessionInput, LifecycleStore, UpsertIssueSharedWorktreeInput,
};
use cadence_aria::product::models::{
    IssueSharedWorktreeStatus, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus,
    ProviderConversationRef, ProviderConversationRole, ProviderName, RepositoryProfileConfidence,
    VerificationCommand, VerificationCommandSafety, VerificationCommandSource,
    VerificationFallbackPolicy, VerificationScope, WorkItemStatus, WorkspaceType,
};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};
use cadence_aria::web::coding_ws_handler::CodingWsOutMessage;
use cadence_aria::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ProviderConfigSnapshot, WsExecutionEventKind,
    WsExecutionEventStatus,
};
use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn start_attempt_moves_to_worktree_prepare_and_creates_timeline_node() {
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::WorktreePrepare);
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].id, "coding_node_0001");
    assert_eq!(nodes[0].stage, CodingExecutionStage::WorktreePrepare);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Running);
    assert_eq!(nodes[0].agent_role, Some(CodingAgentRole::Git));

    assert_eq!(
        rx.recv().await.expect("stage event"),
        CodingWsOutMessage::CodingStageChange {
            stage: CodingExecutionStage::WorktreePrepare
        }
    );
    match rx.recv().await.expect("timeline event") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.id, "coding_node_0001");
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected timeline node event, got {other:?}"),
    }
}

#[test]
fn role_permission_modes_are_persisted_with_role_provider_config() {
    let root = tempfile::tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create attempt");

    let mut snapshot = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
        .expect("default role config");
    snapshot.set_permission_mode_for_role(
        &CodingProviderRole::CodeReviewer,
        CodingProviderPermissionMode::Auto,
    );
    store
        .update_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id, snapshot)
        .expect("save role config");

    let saved = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
        .expect("saved role config");
    assert_eq!(
        saved.permission_mode_for_role(&CodingProviderRole::CodeReviewer),
        CodingProviderPermissionMode::Auto
    );
}

#[tokio::test]
async fn execute_worktree_prepare_creates_git_worktree_and_completes_timeline_node() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    init_repo(&repo);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let started = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");
    let _stage = rx.recv().await.expect("stage event");
    let _node = rx.recv().await.expect("node event");

    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("prepare worktree");

    let expected_worktree = repo
        .join(".worktrees")
        .join("aria-work-items")
        .join("work_item_0001")
        .join("attempt-1");
    assert_eq!(
        prepared.worktree_path.as_deref(),
        Some(expected_worktree.as_path())
    );
    assert!(expected_worktree.join(".git").exists());
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[0].summary.as_deref(), Some("worktree 已准备"));

    match rx.recv().await.expect("timeline update") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Completed);
            assert_eq!(summary.as_deref(), Some("worktree 已准备"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected timeline update, got {other:?}"),
    }
}

#[tokio::test]
async fn worktree_prepare_uses_issue_shared_worktree_path_for_issue_branch() {
    let root = tempdir().expect("root");
    let repo = git_repo_in(root.path().join("repo").as_path());
    let (store, attempt) =
        coding_store_with_attempt(root.path(), "work_item_0001", "aria/issues/issue_0001");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .execute_worktree_prepare(&attempt, &repo)
        .await
        .expect("prepare shared worktree");

    assert_eq!(
        updated.worktree_path.as_deref(),
        Some(
            repo.join(".worktrees")
                .join("aria-issues")
                .join("issue_0001")
                .as_path()
        )
    );
}

#[tokio::test]
async fn final_confirm_releases_issue_shared_worktree_lock() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: root.path().join("repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "issue_worktree_lease_final_confirm",
        )
        .expect("lock");
    let (store, attempt) = final_confirm_attempt(paths.clone(), "work_item_0001");
    lifecycle
        .bind_issue_worktree_lock_to_attempt(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            &attempt.id,
        )
        .expect("bind lock to attempt");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .handle_final_confirm("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("final confirm");

    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("load shared")
        .expect("shared exists");
    assert_eq!(shared.current_active_work_item_id, None);
    assert_eq!(
        shared.last_completed_work_item_id.as_deref(),
        Some("work_item_0001")
    );
}

fn transferred_group_shared_worktree_lock(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> LifecycleStore {
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            branch_name: attempt.branch_name.clone(),
            worktree_path: attempt.worktree_path.clone().expect("group worktree"),
            base_branch: attempt.base_branch.clone(),
        })
        .expect("upsert shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0001",
            &attempt.id,
        )
        .expect("acquire first unit lock");
    lifecycle
        .transfer_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0001",
            "work_item_0002",
            &attempt.id,
        )
        .expect("transfer lock to the second unit");
    lifecycle
}

fn running_group_engine_with_two_units_for_terminal_lock_tests() -> (
    tempfile::TempDir,
    ProductAppPaths,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    seed_group_work_items_and_plan(&paths);
    let store = CodingAttemptStore::new(paths.clone());
    let worktree = root.path().join("group-worktree");
    init_group_worktree(&worktree);
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create group attempt");
    seed_authoritative_group_terminal_fixture(&store, &attempt);
    for (index, work_item_id) in ["work_item_0001", "work_item_0002"]
        .into_iter()
        .enumerate()
    {
        store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                plan_id: "work_item_plan_0001".to_string(),
                logical_work_item_id: work_item_id.to_string(),
                work_item_revision_id: format!("work_item_revision_{:04}", index + 1),
                dependency_logical_work_item_ids: if index == 1 {
                    vec!["work_item_0001".to_string()]
                } else {
                    Vec::new()
                },
                order_index: index as u32,
                status: if index == 0 {
                    CodingExecutionUnitStatus::Running
                } else {
                    CodingExecutionUnitStatus::Pending
                },
            })
            .expect("create group coding unit");
    }
    let attempt = crate::seed_coding_attempt_running(&store, &attempt.project_id, &attempt.issue_id, &attempt.id);
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    (root, paths, store, engine, attempt)
}

fn assert_group_shared_worktree_lock_released(
    lifecycle: &LifecycleStore,
    attempt: &CodingExecutionAttempt,
) {
    let shared = lifecycle
        .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        .expect("reload shared worktree")
        .expect("shared worktree exists");
    assert_eq!(shared.current_active_work_item_id, None);
    assert_eq!(shared.current_lock_owner_id, None);
    assert_eq!(shared.status, IssueSharedWorktreeStatus::Ready);
}

#[tokio::test]
async fn failed_group_attempt_releases_transferred_shared_worktree_lock_by_owner() {
    let (_root, paths, _store, engine, attempt) =
        running_group_engine_with_two_units_for_terminal_lock_tests();
    let lifecycle = transferred_group_shared_worktree_lock(&paths, &attempt);

    let failed = engine
        .handle_attempt_failed(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("group failure releases the transferred lock owned by the attempt");

    assert_eq!(failed.status, CodingAttemptStatus::Failed);
    assert_eq!(failed.current_work_item_id, None);
    assert_group_shared_worktree_lock_released(&lifecycle, &attempt);
}

#[tokio::test]
async fn aborted_group_attempt_releases_transferred_shared_worktree_lock_by_owner() {
    let (_root, paths, _store, engine, attempt) =
        running_group_engine_with_two_units_for_terminal_lock_tests();
    let lifecycle = transferred_group_shared_worktree_lock(&paths, &attempt);

    engine
        .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("group abort releases the transferred lock owned by the attempt");

    assert_group_shared_worktree_lock_released(&lifecycle, &attempt);
}

#[tokio::test]
async fn deleted_group_attempt_releases_transferred_shared_worktree_lock_by_owner() {
    let (_root, paths, _store, engine, attempt) =
        running_group_engine_with_two_units_for_terminal_lock_tests();
    let lifecycle = transferred_group_shared_worktree_lock(&paths, &attempt);

    engine
        .handle_delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("group delete releases the transferred lock owned by the attempt");

    assert_group_shared_worktree_lock_released(&lifecycle, &attempt);
}

#[tokio::test]
async fn dirty_group_delete_keeps_transferred_shared_worktree_lock() {
    let (_root, paths, store, engine, attempt) =
        running_group_engine_with_two_units_for_terminal_lock_tests();
    let lifecycle = transferred_group_shared_worktree_lock(&paths, &attempt);
    let worktree = attempt.worktree_path.as_deref().expect("group worktree");
    fs::write(worktree.join("dirty.txt"), "uncommitted\n").expect("dirty group worktree");

    let error = engine
        .handle_delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("dirty group delete keeps the transferred lock");

    assert!(error.to_string().contains("shared_worktree_dirty_manual_gate"));
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("reload attempt")
            .status,
        CodingAttemptStatus::Running
    );
    let shared = lifecycle
        .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        .expect("reload shared worktree")
        .expect("shared worktree exists");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some("work_item_0002")
    );
    assert_eq!(shared.current_lock_owner_id.as_deref(), Some(attempt.id.as_str()));
    assert_eq!(shared.status, IssueSharedWorktreeStatus::Running);
}

#[tokio::test]
async fn group_final_confirm_releases_transferred_shared_worktree_lock_by_owner() {
    let (_root, paths, _store, engine, attempt) = group_attempt_waiting_for_final_confirm();
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .release_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0002",
            &attempt.id,
        )
        .expect("reset fixture lock");
    let lifecycle = transferred_group_shared_worktree_lock(&paths, &attempt);

    engine
        .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("group final confirm releases the transferred lock owned by the attempt");

    assert_group_shared_worktree_lock_released(&lifecycle, &attempt);
}

fn single_work_item_engine_with_same_owner_lock_on_another_work_item() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
    LifecycleStore,
) {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let (store, attempt) =
        coding_store_with_attempt(root.path(), "work_item_0001", "aria/work-items/work_item_0001");
    let lifecycle = LifecycleStore::new(paths);
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            branch_name: attempt.branch_name.clone(),
            worktree_path: root.path().join("shared-worktree"),
            base_branch: attempt.base_branch.clone(),
        })
        .expect("upsert shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0002",
            &attempt.id,
        )
        .expect("acquire same-owner lock for another work item");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    (root, store, engine, attempt, lifecycle)
}

#[tokio::test]
async fn single_work_item_abort_rejects_same_owner_lock_for_another_work_item() {
    let (_root, store, engine, attempt, lifecycle) =
        single_work_item_engine_with_same_owner_lock_on_another_work_item();

    engine
        .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("single work-item abort keeps exact work-item validation");

    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("reload attempt")
            .status,
        CodingAttemptStatus::Created
    );
    let shared = lifecycle
        .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        .expect("reload shared worktree")
        .expect("shared worktree exists");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some("work_item_0002")
    );
}

#[tokio::test]
async fn single_work_item_failure_rejects_same_owner_lock_for_another_work_item() {
    let (_root, store, engine, attempt, lifecycle) =
        single_work_item_engine_with_same_owner_lock_on_another_work_item();
    let attempt = crate::seed_coding_attempt_running(&store, &attempt.project_id, &attempt.issue_id, &attempt.id);

    engine
        .handle_attempt_failed(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("single work-item failure keeps exact work-item validation");

    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("reload attempt")
            .status,
        CodingAttemptStatus::Running
    );
    let shared = lifecycle
        .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        .expect("reload shared worktree")
        .expect("shared worktree exists");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some("work_item_0002")
    );
}

#[tokio::test]
async fn single_work_item_delete_rejects_same_owner_lock_for_another_work_item() {
    let (_root, store, engine, attempt, lifecycle) =
        single_work_item_engine_with_same_owner_lock_on_another_work_item();

    engine
        .handle_delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("single work-item delete keeps exact work-item validation");

    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("reload attempt")
            .status,
        CodingAttemptStatus::Created
    );
    let shared = lifecycle
        .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        .expect("reload shared worktree")
        .expect("shared worktree exists");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some("work_item_0002")
    );
}

#[tokio::test]
async fn failed_attempt_releases_issue_shared_worktree_lock() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: root.path().join("repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "issue_worktree_lease_failed_attempt",
        )
        .expect("lock");
    let (store, attempt) = failed_attempt(paths.clone(), "work_item_0001");
    lifecycle
        .bind_issue_worktree_lock_to_attempt(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            &attempt.id,
        )
        .expect("bind lock to attempt");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .handle_attempt_failed("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("handle failed");

    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("load shared")
        .expect("shared exists");
    assert_eq!(shared.current_active_work_item_id, None);
}

#[tokio::test]
async fn dirty_shared_worktree_blocks_lock_release_and_next_work_item() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let shared_path = root.path().join("repo/.worktrees/aria-issues/issue_0001");
    git_repo_in(&shared_path);
    std::fs::write(shared_path.join("dirty.txt"), "uncommitted").expect("dirty file");
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: shared_path,
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            "issue_worktree_lease_dirty_attempt",
        )
        .expect("lock");
    let (store, attempt) = dirty_failed_attempt(paths.clone(), "work_item_0001");
    lifecycle
        .bind_issue_worktree_lock_to_attempt(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            &attempt.id,
        )
        .expect("bind lock to attempt");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let failed = engine
        .handle_attempt_failed("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("dirty worktree diagnostics must not replace terminal failure");

    assert_eq!(failed.status, CodingAttemptStatus::Failed);
    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("load shared")
        .expect("shared exists");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );
}

#[tokio::test]
async fn execute_coding_runs_provider_in_worktree_and_streams_timeline_events() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree.clone()),
            ..create_input()
        })
        .expect("create attempt");
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", &attempt.id);
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = FileWritingStreamingProvider;

    let updated = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(
        fs::read_to_string(worktree.join("generated.txt")).expect("generated file"),
        "generated by provider\n"
    );
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].stage, CodingExecutionStage::Coding);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[0].summary.as_deref(), Some("代码编写完成"));

    match rx.recv().await.expect("coding node created") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.id, "coding_node_0001");
            assert_eq!(node.stage, CodingExecutionStage::Coding);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected coding node created, got {other:?}"),
    }
    match rx.recv().await.expect("provider prompt event") {
        CodingWsOutMessage::CodingExecutionEvent { event } => {
            assert_eq!(event.event_id, "coding_node_0001_prompt");
            assert_eq!(event.node_id.as_deref(), Some("coding_node_0001"));
            assert_eq!(event.title, "Provider Prompt");
            assert!(
                event
                    .output
                    .expect("prompt output")
                    .contains("Coding Workspace")
            );
        }
        other => panic!("expected provider prompt event, got {other:?}"),
    }
    assert_eq!(
        rx.recv().await.expect("coding stream chunk"),
        CodingWsOutMessage::CodingStreamChunk {
            content: "created generated.txt".to_string(),
            node_id: Some("coding_node_0001".to_string()),
        }
    );
    assert_eq!(
        rx.recv().await.expect("coding message complete"),
        CodingWsOutMessage::CodingMessageComplete {
            node_id: Some("coding_node_0001".to_string()),
        }
    );
    match rx.recv().await.expect("coding node completed") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Completed);
            assert_eq!(summary.as_deref(), Some("代码编写完成"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected coding node completed, got {other:?}"),
    }
}

#[tokio::test]
async fn coding_coder_run_resumes_previous_coder_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            ..create_input()
        })
        .expect("create attempt");
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", &attempt.id);
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::default();

    let first = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("first coding run");
    let second = engine
        .execute_coding(&first, &provider, &CodingExecutionContext::default())
        .await
        .expect("second coding run");

    assert_eq!(second.stage, CodingExecutionStage::Coding);
    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 2);
    assert_eq!(
        inputs[0].permission_mode,
        ProviderPermissionMode::Auto
    );
    assert_eq!(
        inputs[1].permission_mode,
        ProviderPermissionMode::Auto
    );
    assert_eq!(inputs[0].timeout_secs, 10_800);
    assert_eq!(inputs[1].timeout_secs, 10_800);
    assert_eq!(inputs[0].resume_provider_session_id, None);
    assert_eq!(
        inputs[1].resume_provider_session_id.as_deref(),
        Some("coder-session-1")
    );
}

#[tokio::test]
async fn coding_coder_rework_with_resume_uses_delta_prompt() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            ..create_input()
        })
        .expect("create attempt");
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", &attempt.id);
    let context = CodingExecutionContext {
        work_item_markdown: Some(
            "# 爬楼梯问题 Work Item\n\n\
             ## 实现要求\n\
             这里是一段很长的已确认 Work Item，返修续接时不应重复发送。\n"
                .to_string(),
        ),
        verification_commands: vec!["uv run python -m unittest".to_string()],
    };
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        ["first coding done", "second coding done"],
        [
            Some("coder-session-1".to_string()),
            Some("coder-session-1".to_string()),
        ],
    );

    let first = engine
        .execute_coding(&attempt, &provider, &context)
        .await
        .expect("first coding run");
    store
        .save_rework_instruction(&attempt, &CodingReworkInstruction {
            id: "coding_rework_instruction_0001".to_string(),
            attempt_id: attempt.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round: 1,
            summary: "reviewer 要求补充边界测试".to_string(),
            fix_hints: vec!["补充 n=0 的输入处理".to_string()],
            questions: Vec::new(),
            created_at: "2026-06-07T00:00:00Z".to_string(),
            consumed_by_node_id: None,
            consumed_at: None,
        })
        .expect("save rework instruction");

    engine
        .execute_coding(&first, &provider, &context)
        .await
        .expect("second coding run");

    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 2);
    let second_input = &inputs[1];
    assert_eq!(
        second_input.resume_provider_session_id.as_deref(),
        Some("coder-session-1")
    );
    assert!(second_input.prompt.contains("增量代码编写指令"));
    assert!(second_input.prompt.contains("reviewer 要求补充边界测试"));
    assert!(second_input.prompt.contains("补充 n=0 的输入处理"));
    assert!(second_input.prompt.contains("uv run python -m unittest"));
    assert!(!second_input.prompt.contains("# 爬楼梯问题 Work Item"));
    assert!(
        !second_input
            .prompt
            .contains("这里是一段很长的已确认 Work Item，返修续接时不应重复发送。")
    );
    assert!(
        !second_input
            .prompt
            .contains("不要只输出计划或 Story/Design/Work Item 文档")
    );
}

#[tokio::test]
async fn group_next_work_item_coder_run_does_not_resume_previous_unit_session() {
    let (_root, _paths, store, _engine, attempt) = group_engine_with_last_running_unit();
    seed_authoritative_group_coder_fixture(&store, &attempt);
    let attempt = crate::seed_coding_attempt_running(&store, &attempt.project_id, &attempt.issue_id, &attempt.id);
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::Coder,
                provider: ProviderName::Fake,
                provider_session_id: "coder-session-from-unit-1".to_string(),
                updated_at: "2026-07-08T00:00:00Z".to_string(),
                last_node_id: Some("coding_node_0002".to_string()),
            }],
        )
        .expect("seed previous coder session");
    let context = CodingExecutionContext {
        work_item_markdown: Some(
            "# Work Item 002\n\n实现 ProviderDescriptor 元数据层与全局 ProviderStateStore。"
                .to_string(),
        ),
        verification_commands: vec!["cargo test --locked --lib provider_metadata".to_string()],
    };
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        ["unit 2 coding done"],
        [Some("coder-session-unit-2".to_string())],
    );

    engine
        .execute_coding(&attempt, &provider, &context)
        .await
        .expect("execute second unit coding");

    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    let input = &inputs[0];
    assert_eq!(input.resume_provider_session_id, None);
    assert!(input.prompt.contains("work_item_revision_0002"));
    assert!(input.prompt.contains("work_item_0002"));
    assert!(!input.prompt.contains("# Work Item 002"));
    assert!(!input.prompt.contains("ProviderDescriptor 元数据层"));
    assert!(!input.prompt.contains("增量代码编写指令"));
    assert!(!input.prompt.contains("本轮没有新增修复要求"));
}
