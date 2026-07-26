use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{
    CodingAttemptStore, CodingGitOperationKind, CodingGitOperationPhase, CreateChoiceGateInput,
    CreateCodingAttemptInput, CreateCodingExecutionUnitInput,
    CreateGroupCodingAttemptInput,
    PrepareCodingGitOperationInput,
};
use cadence_aria::product::git_workspace_service::GitWorkspaceService;
use cadence_aria::product::coding_workspace_runner::CodingRunnerCommand;
use cadence_aria::product::work_item_contract::DependencyContractEdge;
use cadence_aria::product::work_item_revision_store::WorkItemRevisionStore;
use cadence_aria::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingAttemptStatus, CodingChoiceOption, CodingExecutionAttempt,
    CodingExecutionStage, CodingExecutionUnitStatus, CodingProviderRole, CodingTimelineNode,
    CodingTimelineNodeStatus, FindingSeverity, InternalPrReview, PushStatus, RemoteKind,
    ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict, TestCommand,
    TestCommandStatus, TestingOverallStatus, TestingReport, WorkItemHandoff,
};
use cadence_aria::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateVerificationPlanInput, CreateWorkItemInput,
    CreateWorkspaceSessionInput, LifecycleStore,
};
use cadence_aria::product::models::{
    DependencyGraphRevision, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, PlanRevisionReason,
    ProviderName, RepositoryProfileConfidence, VerificationCommand, VerificationCommandSafety,
    VerificationCommandSource, VerificationFallbackPolicy, VerificationScope, WorkItemKind,
    WorkItemPlanLineage, WorkItemPlanRevision, WorkItemPlanStatus, WorkItemStatus,
    WorkspaceSessionStatus, WorkspaceType,
};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::{CodingAttemptRunKey, WebAppState};
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::sync::mpsc;
use tower::ServiceExt;
#[tokio::test]
async fn creates_coding_attempt_for_confirmed_work_item_and_surfaces_latest_attempt() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;

    let (status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "response: {attempt}");
    let attempt_id = assert_global_attempt_id(&attempt);
    assert_eq!(attempt["work_item_id"], "work_item_0001");
    assert_eq!(attempt["attempt_no"], 1);
    assert_eq!(attempt["status"], "created");
    assert_eq!(attempt["stage"], "prepare_context");
    assert_eq!(attempt["branch_name"], "aria/issues/issue_0001");

    let (status, duplicate) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(duplicate["code"], "coding_attempt_active");

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lifecycle["coding_attempts"].as_array().unwrap().len(), 1);
    assert_eq!(
        lifecycle["work_items"][0]["latest_attempt"]["attempt_id"],
        attempt_id
    );
}
#[tokio::test]
async fn creates_coding_attempt_with_confirmed_work_item_workspace_providers() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_with_providers(app.clone(), repo.path(), "codex", "claude_code")
        .await;

    let (status, attempt) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&attempt);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let persisted = store
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("persisted attempt");
    assert_eq!(
        persisted.provider_config_snapshot.author,
        ProviderName::Codex
    );
    assert_eq!(
        persisted.provider_config_snapshot.reviewer,
        Some(ProviderName::ClaudeCode)
    );
}
#[tokio::test]
async fn creates_coding_attempt_falls_back_from_unavailable_default_codex_to_claude_code() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::with_provider_availability(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        |provider| matches!(provider, ProviderName::ClaudeCode),
    ));
    bootstrap_confirmed_work_item_without_workspace_session(
        app.clone(),
        root.path(),
        repo.path(),
        "codex",
    )
    .await;

    let (status, attempt) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&attempt);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let persisted = store
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("persisted attempt");
    assert_eq!(
        persisted.provider_config_snapshot.author,
        ProviderName::ClaudeCode
    );
    assert_eq!(
        persisted.provider_config_snapshot.reviewer,
        Some(ProviderName::ClaudeCode)
    );
}
#[tokio::test]
async fn rejects_coding_attempt_when_work_item_plan_is_not_confirmed() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_unconfirmed_work_item(app.clone(), repo.path()).await;

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "work_item_plan_not_confirmed");
}
#[tokio::test]
async fn rejects_coding_attempt_when_dependency_work_item_is_not_completed() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_split_work_items(app.clone(), root.path(), repo.path()).await;

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "work_item_dependency_not_completed");
    assert_eq!(
        body["details"]["missing_dependencies"],
        json!(["work_item_0001"])
    );
}
#[tokio::test]
async fn rejects_second_active_work_item_on_same_issue_shared_worktree() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_two_ready_confirmed_work_items(app.clone(), root.path(), repo.path()).await;

    let (status, first) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_global_attempt_id(&first);
    assert_eq!(first["branch_name"], "aria/issues/issue_0001");

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "issue_worktree_active");
}
#[tokio::test]
async fn creates_group_coding_attempt_from_confirmed_work_item_plan() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_global_attempt_id(&body);
    assert_eq!(body["attempt_scope"], "work_item_group");
    assert_eq!(body["work_item_group_id"], "work_item_plan_0001");
    assert_eq!(body["current_work_item_id"], "work_item_0001");
    assert_eq!(body["active_unit_id"], "coding_unit_0001");
    assert_eq!(body["branch_name"], "aria/issues/issue_0001");
}
#[tokio::test]
async fn returns_group_coding_attempt_snapshot_with_units() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;

    let (status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&attempt);

    let (status, snapshot) = request_json(
        app,
        Method::GET,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(snapshot["attempt_scope"], "work_item_group");
    assert_eq!(snapshot["work_item_group_id"], "work_item_plan_0001");
    assert_eq!(snapshot["current_work_item_id"], "work_item_0001");
    assert_eq!(snapshot["active_unit_id"], "coding_unit_0001");
    assert_eq!(snapshot["units"].as_array().expect("units").len(), 2);
    assert_eq!(snapshot["units"][0]["status"], "running");
    assert_eq!(snapshot["units"][1]["status"], "pending");
}
#[tokio::test]
async fn group_coding_attempt_snapshot_uses_visible_handoff_instead_of_attempt_level_file() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create group attempt");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0001".to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("create unit1");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0002".to_string(),
            work_item_revision_id: "work_item_revision_0002".to_string(),
            dependency_logical_work_item_ids: vec!["work_item_0001".to_string()],
            order_index: 1,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("create unit2");
    store
        .save_work_item_handoff(&WorkItemHandoff {
            id: "work_item_handoff_stale".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_9999".to_string(),
            attempt_id: attempt.id.clone(),
            provider_run_ref: None,
            summary: "stale attempt-level handoff".to_string(),
            files_changed: Vec::new(),
            commit_sha: None,
            diff_summary: String::new(),
            tests_run: Vec::new(),
            test_result_summary: String::new(),
            review_summary: None,
            api_or_contract_changes: Vec::new(),
            open_risks: Vec::new(),
            next_work_item_notes: Vec::new(),
            created_at: "2026-06-27T00:00:00Z".to_string(),
        })
        .expect("save stale attempt handoff");
    store
        .save_coding_unit_handoff(
            "project_0001",
            "issue_0001",
            &attempt.id,
            "coding_unit_0001",
            &WorkItemHandoff {
                id: "work_item_handoff_0001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                attempt_id: attempt.id.clone(),
                provider_run_ref: None,
                summary: "unit1 handoff".to_string(),
                files_changed: Vec::new(),
                commit_sha: None,
                diff_summary: String::new(),
                tests_run: Vec::new(),
                test_result_summary: String::new(),
                review_summary: None,
                api_or_contract_changes: Vec::new(),
                open_risks: Vec::new(),
                next_work_item_notes: Vec::new(),
                created_at: "2026-06-27T00:00:00Z".to_string(),
            },
        )
        .expect("save unit1 handoff");
    store
        .update_coding_unit_latest_handoff_revision_id(
            "project_0001",
            "issue_0001",
            &attempt.id,
            "coding_unit_0001",
            Some("handoff_revision_0001".to_string()),
        )
        .expect("set handoff ref");

    let (status, snapshot) = request_json(
        app,
        Method::GET,
        &scoped_attempt_uri(&attempt.id, ""),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(snapshot["attempt_scope"], "work_item_group");
    assert_eq!(snapshot["work_item_handoff"]["work_item_id"], "work_item_0001");
    assert_eq!(snapshot["work_item_handoff"]["summary"], "unit1 handoff");
}

#[tokio::test]
async fn rejects_group_coding_attempt_for_unconfirmed_plan() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_draft_work_item_plan_group(app.clone(), repo.path()).await;

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "work_item_plan_not_confirmed");
}

#[tokio::test]
async fn rejects_group_coding_attempt_when_single_item_attempt_holds_issue_lock() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;

    let (single_status, single) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(single_status, StatusCode::OK);
    assert_global_attempt_id(&single);

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "issue_worktree_active");
}

#[tokio::test]
async fn group_coding_attempt_retry_is_not_blocked_after_unit_creation_failure() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let invalid_fixture = inject_invalid_group_second_work_item(&app_paths);

    let (first_status, first_body) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(first_status, StatusCode::BAD_REQUEST);
    assert_eq!(
        first_body["code"],
        "coding_plan_revision_binding_missing"
    );

    assert_group_attempt_creation_rolled_back(&app_paths);
    restore_group_second_work_item(invalid_fixture);

    let (retry_status, retry_body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(retry_status, StatusCode::OK);
    assert_global_attempt_id(&retry_body);
    assert_eq!(retry_body["active_unit_id"], "coding_unit_0001");
}

#[tokio::test]
async fn rejects_coding_attempt_when_required_dependency_handoff_is_missing() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_completed_dependency_without_handoff(app.clone(), root.path(), repo.path()).await;

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "work_item_handoff_missing");
    assert_eq!(
        body["details"]["missing_handoffs"],
        json!(["work_item_0001"])
    );
}

#[tokio::test]
async fn delete_coding_attempt_releases_active_lock_when_clean() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_two_ready_confirmed_work_items(app.clone(), root.path(), repo.path()).await;

    let (status, first) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&first);

    let (status, _body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_global_attempt_id(&second);
}

#[tokio::test]
async fn delete_coding_attempt_with_dirty_shared_worktree_still_removes_workspace() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_two_ready_confirmed_work_items(app.clone(), root.path(), repo.path()).await;

    let (status, first) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&first);
    let coding_store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = prepare_attempt_with_worktree(
        &coding_store,
        repo.path(),
        "project_0001",
        "issue_0001",
        &attempt_id,
    );
    let worktree_path = attempt.worktree_path.expect("attempt worktree path");
    fs::write(worktree_path.join("dirty.txt"), "dirty changes").expect("dirty file");

    let (status, _body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!worktree_path.exists());
    assert!(
        coding_store
            .get_attempt("project_0001", "issue_0001", &attempt_id)
            .is_err()
    );

    let (status, second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_global_attempt_id(&second);
}

#[tokio::test]
async fn delete_failed_coding_attempt_with_dirty_shared_worktree_still_removes_workspace() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    bootstrap_two_ready_confirmed_work_items(app.clone(), root.path(), repo.path()).await;

    let (status, first) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&first);
    let attempt_key = CodingAttemptRunKey::new(
        "project_0001",
        "issue_0001",
        attempt_id.clone(),
    );
    let coding_store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = prepare_attempt_with_worktree(
        &coding_store,
        repo.path(),
        "project_0001",
        "issue_0001",
        &attempt_id,
    );
    let worktree_path = attempt.worktree_path.expect("attempt worktree path");
    fs::write(worktree_path.join("dirty.txt"), "dirty changes").expect("dirty file");
    coding_store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt_id,
            CodingAttemptStatus::Running,
        )
        .expect("mark attempt running");
    coding_store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt_id,
            CodingAttemptStatus::Failed,
        )
        .expect("mark attempt failed");
    let (first_runner_tx, mut first_runner_rx) = mpsc::channel(1);
    let (second_runner_tx, mut second_runner_rx) = mpsc::channel(1);
    let first_run_id = state
        .coding_runs
        .insert_cancellable(&attempt_key, first_runner_tx)
        .expect("first runner")
        .run_id();
    let second_run_id = state
        .coding_runs
        .insert_cancellable(&attempt_key, second_runner_tx)
        .expect("second runner")
        .run_id();

    let delete_app = app.clone();
    let delete_uri = scoped_attempt_uri(&attempt_id, "");
    let delete_request = tokio::spawn(async move {
        request_json(delete_app, Method::DELETE, &delete_uri, json!({})).await
    });
    assert_eq!(
        first_runner_rx.recv().await.expect("first runner abort"),
        CodingRunnerCommand::AbortAttempt
    );
    assert_eq!(
        second_runner_rx.recv().await.expect("second runner abort"),
        CodingRunnerCommand::AbortAttempt
    );
    tokio::task::yield_now().await;
    assert!(
        !delete_request.is_finished(),
        "HTTP delete returned before runner removal"
    );
    state.coding_runs.remove(&attempt_key, first_run_id);
    assert!(
        !delete_request.is_finished(),
        "HTTP delete returned before every runner was removed"
    );
    state.coding_runs.remove(&attempt_key, second_run_id);
    let (status, _body) = delete_request.await.expect("delete request task");
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(state.coding_runs.runner_count(&attempt_key), 0);
    assert!(!worktree_path.exists());
    assert!(
        coding_store
            .get_attempt("project_0001", "issue_0001", &attempt_id)
            .is_err()
    );

    let (status, second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_global_attempt_id(&second);
}
