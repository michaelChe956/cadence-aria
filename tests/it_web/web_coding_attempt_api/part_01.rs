use std::fs;
use std::path::PathBuf;
use std::process::Command;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{CodingAttemptStore, CreateChoiceGateInput};
use cadence_aria::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingChoiceOption, CodingExecutionAttempt,
    CodingExecutionStage, CodingProviderRole, CodingTimelineNode, CodingTimelineNodeStatus,
    FindingSeverity, InternalPrReview, PushStatus, RemoteKind, ReviewFinding, ReviewRequest,
    ReviewRequestKind, ReviewVerdict, TestCommand, TestCommandStatus, TestingOverallStatus,
    TestingReport,
};
use cadence_aria::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateVerificationPlanInput, CreateWorkItemInput,
    CreateWorkspaceSessionInput, LifecycleStore,
};
use cadence_aria::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, ProviderName, RepositoryProfileConfidence,
    VerificationCommand, VerificationCommandSafety, VerificationCommandSource,
    VerificationFallbackPolicy, VerificationScope, WorkItemKind, WorkItemPlanStatus,
    WorkItemStatus, WorkspaceSessionStatus, WorkspaceType,
};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use tempfile::tempdir;
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

    assert_eq!(status, StatusCode::OK);
    assert_eq!(attempt["attempt_id"], "coding_attempt_0001");
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
        "coding_attempt_0001"
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
    assert_eq!(attempt["attempt_id"], "coding_attempt_0001");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let persisted = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
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
    assert_eq!(attempt["attempt_id"], "coding_attempt_0001");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let persisted = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
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
    assert_eq!(body["attempt_scope"], "work_item_group");
    assert_eq!(body["work_item_group_id"], "work_item_plan_0001");
    assert_eq!(body["current_work_item_id"], "work_item_0001");
    assert_eq!(body["active_unit_id"], "coding_unit_0001");
    assert_eq!(body["branch_name"], "aria/issues/issue_0001");
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

    let (single_status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(single_status, StatusCode::OK);

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
    let units_blocker = app_paths
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("coding-attempts")
        .join("coding_attempt_0001")
        .join("units");
    fs::create_dir_all(units_blocker.parent().expect("attempt dir")).expect("attempt dir");
    fs::write(&units_blocker, "block unit directory creation").expect("units blocker");

    let (first_status, _first_body) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(first_status, StatusCode::INTERNAL_SERVER_ERROR);

    let coding_store = CodingAttemptStore::new(app_paths.clone());
    assert!(
        coding_store
            .list_attempts_for_work_item("project_0001", "issue_0001", "work_item_0001")
            .expect("list attempts after failed create")
            .is_empty()
    );
    assert!(!units_blocker.exists());

    let (retry_status, retry_body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(retry_status, StatusCode::OK);
    assert_eq!(retry_body["attempt_id"], "coding_attempt_0001");
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
async fn abort_coding_attempt_releases_issue_shared_worktree_lock() {
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

    let (status, _body) = request_json(
        app.clone(),
        Method::POST,
        &format!(
            "/api/coding-attempts/{}/abort",
            first["attempt_id"].as_str().unwrap()
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["work_item_id"], "work_item_0002");
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
    let attempt_id = first["attempt_id"].as_str().unwrap();

    let (status, _body) = request_json(
        app.clone(),
        Method::DELETE,
        &format!("/api/coding-attempts/{}", attempt_id),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn returns_coding_attempt_snapshot_with_persisted_execution_state() {
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
    assert_eq!(status, StatusCode::OK);
    let attempt_id = attempt["attempt_id"].as_str().expect("attempt id");

    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let testing_report = sample_testing_report(attempt_id);
    let code_review = sample_code_review_report(attempt_id);
    let review_request = sample_review_request(attempt_id);
    let internal_review = sample_internal_review(attempt_id, &review_request.id);
    store
        .save_testing_report(&testing_report)
        .expect("save testing report");
    store
        .save_code_review_report(&code_review)
        .expect("save code review");
    store
        .save_review_request(&review_request)
        .expect("save review request");
    store
        .save_internal_pr_review(&internal_review)
        .expect("save internal review");
    store
        .save_timeline_node(sample_completed_node(attempt_id))
        .expect("save completed node");
    store
        .save_timeline_node(sample_running_node(attempt_id))
        .expect("save running node");
    store
        .create_choice_gate(CreateChoiceGateInput {
            attempt_id: attempt_id.to_string(),
            choice_id: "choice_0001".to_string(),
            stage: CodingExecutionStage::Coding,
            node_id: Some("coding_node_0002".to_string()),
            role: CodingProviderRole::Coder,
            provider: ProviderName::Codex,
            source: "request_user_input".to_string(),
            prompt: "请选择实现范围".to_string(),
            options: vec![CodingChoiceOption {
                id: "backend_first".to_string(),
                label: "先做后端".to_string(),
                description: None,
            }],
            allow_multiple: false,
            allow_free_text: true,
        })
        .expect("create choice gate");

    let (status, snapshot) = request_json(
        app,
        Method::GET,
        "/api/coding-attempts/coding_attempt_0001",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(snapshot["attempt"]["attempt_id"], "coding_attempt_0001");
    assert_eq!(snapshot["attempt"]["stage"], "prepare_context");
    assert_eq!(snapshot["active_node_id"], "coding_node_0002");
    assert_eq!(snapshot["timeline_nodes"].as_array().unwrap().len(), 2);
    assert_eq!(snapshot["testing_report"]["id"], testing_report.id.as_str());
    assert_eq!(
        snapshot["code_review_reports"][0]["summary"],
        code_review.summary.as_str()
    );
    assert_eq!(
        snapshot["review_request"]["commit_sha"],
        review_request.commit_sha.as_str()
    );
    assert_eq!(
        snapshot["internal_pr_review"]["summary"],
        internal_review.summary.as_str()
    );
    assert_eq!(snapshot["pending_choices"][0]["choice_id"], "choice_0001");
    assert_eq!(
        snapshot["pending_choices"][0]["source"],
        "request_user_input"
    );
}

#[tokio::test]
async fn aborts_coding_attempt_and_allows_next_attempt_for_same_work_item() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;

    let (status, first) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["attempt_id"], "coding_attempt_0001");

    let (status, aborted) = request_json(
        app.clone(),
        Method::POST,
        "/api/coding-attempts/coding_attempt_0001/abort",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(aborted["attempt_id"], "coding_attempt_0001");
    assert_eq!(aborted["status"], "aborted");

    let (status, second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["attempt_id"], "coding_attempt_0002");
    assert_eq!(second["attempt_no"], 2);
}

#[tokio::test]
async fn deletes_coding_attempt_and_preserves_work_item() {
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
    assert_eq!(status, StatusCode::OK);
    assert_eq!(attempt["attempt_id"], "coding_attempt_0001");

    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = prepare_attempt_with_worktree(
        &store,
        repo.path(),
        "project_0001",
        "issue_0001",
        "coding_attempt_0001",
    );
    let artifact_dir =
        store.attempt_test_output_root("project_0001", "issue_0001", "coding_attempt_0001");
    fs::create_dir_all(&artifact_dir).expect("artifact dir");
    fs::write(artifact_dir.join("unit.stdout.log"), "unit stdout\n").expect("artifact");
    store
        .save_testing_report(&sample_testing_report("coding_attempt_0001"))
        .expect("save testing report");
    store
        .save_timeline_node(sample_running_node("coding_attempt_0001"))
        .expect("save timeline node");
    let attempt_dir = artifact_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("attempt dir")
        .to_path_buf();
    let worktree_path = attempt.worktree_path.clone().expect("worktree path");
    assert!(attempt_dir.exists());
    assert!(worktree_path.exists());
    assert!(branch_exists(repo.path(), &attempt.branch_name));

    let (status, _body) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/coding-attempts/coding_attempt_0001",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!attempt_dir.exists());
    assert!(!worktree_path.exists());
    assert!(!branch_exists(repo.path(), &attempt.branch_name));

    let (status, _) = request_json(
        app.clone(),
        Method::GET,
        "/api/coding-attempts/coding_attempt_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, lifecycle) = request_json(
        app.clone(),
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 1);
    assert!(lifecycle["work_items"][0]["latest_attempt"].is_null());
    assert!(lifecycle["coding_attempts"].as_array().unwrap().is_empty());

    let (status, second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["attempt_id"], "coding_attempt_0001");
    assert_eq!(second["attempt_no"], 1);
}

#[tokio::test]
async fn delete_work_item_cascades_coding_attempts_worktrees_and_branches() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first = prepare_attempt_with_worktree(
        &store,
        repo.path(),
        "project_0001",
        "issue_0001",
        "coding_attempt_0001",
    );
    let first_artifact_dir =
        store.attempt_test_output_root("project_0001", "issue_0001", "coding_attempt_0001");
    fs::create_dir_all(&first_artifact_dir).expect("first artifact dir");
    fs::write(first_artifact_dir.join("unit.stdout.log"), "first\n").expect("first artifact");
    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/coding-attempts/coding_attempt_0001/abort",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let second = prepare_attempt_with_worktree(
        &store,
        repo.path(),
        "project_0001",
        "issue_0001",
        "coding_attempt_0002",
    );
    let second_artifact_dir =
        store.attempt_test_output_root("project_0001", "issue_0001", "coding_attempt_0002");
    fs::create_dir_all(&second_artifact_dir).expect("second artifact dir");
    fs::write(second_artifact_dir.join("unit.stdout.log"), "second\n").expect("second artifact");
    let first_attempt_dir = first_artifact_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("first attempt dir")
        .to_path_buf();
    let second_attempt_dir = second_artifact_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("second attempt dir")
        .to_path_buf();

    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "deleted");
    assert!(!first_attempt_dir.exists());
    assert!(!second_attempt_dir.exists());
    assert!(
        !first
            .worktree_path
            .as_ref()
            .expect("first worktree")
            .exists()
    );
    assert!(
        !second
            .worktree_path
            .as_ref()
            .expect("second worktree")
            .exists()
    );
    assert!(!branch_exists(repo.path(), &first.branch_name));
    assert!(!branch_exists(repo.path(), &second.branch_name));

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(lifecycle["work_items"].as_array().unwrap().is_empty());
    assert!(lifecycle["coding_attempts"].as_array().unwrap().is_empty());
}
