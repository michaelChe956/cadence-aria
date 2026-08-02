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
use cadence_aria::product::work_item_projection::{
    PlanProjectionCompileInput, PlanProjectionCompiler,
};
use cadence_aria::product::work_item_revision_store::WorkItemRevisionStore;
use cadence_aria::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingAttemptStatus, CodingChoiceOption, CodingExecutionAttempt,
    CodingExecutionStage, CodingExecutionUnitStatus, CodingProviderRole, CodingTimelineNode,
    CodingTimelineNodeStatus, FindingSeverity, InternalPrReview, PushStatus, RemoteKind,
    ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict,
};
use cadence_aria::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateVerificationPlanInput, CreateWorkItemInput,
    CreateWorkspaceSessionInput, LifecycleStore,
};
use cadence_aria::product::models::{
    DependencyGraphRevision, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, PlanProjectionBundle,
    PlanRevisionReason, ProviderName, RepositoryProfileConfidence, VerificationCommand,
    VerificationCommandSafety, VerificationCommandSource, VerificationFallbackPolicy,
    HandoffRevision, VerificationScope, WorkItemKind, WorkItemPlanLineage, WorkItemPlanRevision,
    WorkItemPlanStatus, WorkItemStatus,
    WorkspaceSessionStatus, WorkspaceType,
};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::{CodingAttemptRunKey, WebAppState};
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
use cadence_aria::product::work_item_contract::DependencyContractGraph;
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

    assert_eq!(status, StatusCode::BAD_REQUEST, "response: {body}");
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
    fs::remove_dir_all(
        ProductAppPaths::new(root.path().join(".aria"))
            .issue_root("project_0001", "issue_0001")
            .join("work-items"),
    )
    .expect("remove legacy work items");

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
async fn creates_group_coding_attempt_from_schema_v2_revisions_without_legacy_work_items() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let issue_root = app_paths.issue_root("project_0001", "issue_0001");
    fs::remove_dir_all(issue_root.join("work-items")).expect("remove legacy work items");
    fs::remove_dir_all(issue_root.join("verification-plans"))
        .expect("remove legacy verification plans");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list legacy work items")
            .is_empty()
    );
    assert!(
        lifecycle
            .list_verification_plans("project_0001", "issue_0001")
            .expect("list legacy verification plans")
            .is_empty()
    );

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "response: {body}");
    let attempt_id = assert_global_attempt_id(&body);
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt = coding_store
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("persisted group attempt");
    assert_eq!(
        coding_store
            .get_plan_binding(&attempt)
            .expect("Revision-backed plan binding")
            .bound_plan_revision_id,
        "plan_revision_0001"
    );
    assert_eq!(
        coding_store
            .list_coding_units("project_0001", "issue_0001", &attempt_id)
            .expect("Revision-backed group units")
            .into_iter()
            .map(|unit| (unit.logical_work_item_id, unit.work_item_revision_id))
            .collect::<Vec<_>>(),
        vec![
            (
                "work_item_0001".to_string(),
                "work_item_revision_0001".to_string(),
            ),
            (
                "work_item_0002".to_string(),
                "work_item_revision_0002".to_string(),
            ),
        ]
    );
}

#[tokio::test]
async fn rejects_single_work_item_coding_for_a_schema_v2_group() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    fs::remove_dir_all(
        ProductAppPaths::new(root.path().join(".aria"))
            .issue_root("project_0001", "issue_0001")
            .join("work-items"),
    )
    .expect("remove legacy work items");

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "response: {body}");
    assert_eq!(body["code"], "schema_v2_group_coding_required");
}

#[tokio::test]
async fn rejects_group_coding_when_plan_projection_dependencies_do_not_match_the_graph() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let plan_projection_path = app_paths
        .issue_root("project_0001", "issue_0001")
        .join(
            "work-item-revisions/work_item_plan_0001/plan-projection-bundles/plan_projection_bundle_0001.json",
        );
    let mut plan_projection: Value = serde_json::from_slice(
        &fs::read(&plan_projection_path).expect("plan projection bundle"),
    )
    .expect("parse plan projection bundle");
    plan_projection["coder_group_context"]["dependency_edges"] = json!([]);
    fs::write(
        plan_projection_path,
        serde_json::to_vec_pretty(&plan_projection).expect("serialize plan projection bundle"),
    )
    .expect("write invalid plan projection bundle");

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "response: {body}");
    assert_eq!(body["code"], "coding_plan_revision_binding_missing");
    assert_group_attempt_creation_rolled_back(&app_paths);
}

#[tokio::test]
async fn rejects_group_coding_when_plan_projection_repeats_a_work_item_bundle_ref() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let plan_projection_path = app_paths
        .issue_root("project_0001", "issue_0001")
        .join(
            "work-item-revisions/work_item_plan_0001/plan-projection-bundles/plan_projection_bundle_0001.json",
        );
    let mut plan_projection: Value = serde_json::from_slice(
        &fs::read(&plan_projection_path).expect("plan projection bundle"),
    )
    .expect("parse plan projection bundle");
    let first_ref = plan_projection["work_item_projection_bundle_refs"][0].clone();
    plan_projection["work_item_projection_bundle_refs"]
        .as_array_mut()
        .expect("projection refs")
        .push(first_ref);
    fs::write(
        plan_projection_path,
        serde_json::to_vec_pretty(&plan_projection).expect("serialize plan projection bundle"),
    )
    .expect("write invalid plan projection bundle");

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "response: {body}");
    assert_eq!(body["code"], "coding_plan_revision_binding_missing");
    assert_group_attempt_creation_rolled_back(&app_paths);
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
async fn rejects_group_coding_attempt_when_a_legacy_single_item_attempt_is_active() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;

    let single = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "legacy_lock_holder".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .expect("active legacy single-item attempt");
    assert!(single.status.is_active());

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
async fn creates_coding_attempt_when_completed_dependency_has_no_handoff_summary() {
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

    assert_eq!(status, StatusCode::OK);
    assert_global_attempt_id(&body);
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

// ============================================================================
// Task 3 (TDD RED): HTTP delete_coding_attempt 必须清理已认领的 handoff lineage
//
// 这些测试覆盖 cleanup-attempt-handoff-revisions 变更的端到端契约：
//   - delete_coding_attempt 当前只做 worktree 清理与 delete_attempt，
//     不会删除 attempt 已认领的 handoff revision 文件（Task 4 才接入清理）。
//   - 因此本批测试处于 RED 形态：DELETE 后 handoff 文件仍在 lineage，断言 !exists 失败。
//
// 夹具构造方式（降级路径，brief 已授权）：
//   完整端到端推进 group attempt 到「unit 完成 + handoff 发布」需要 fake provider
//   驱动整条 coding→review→completion 链路，成本过高且偏离本任务目标（锁定删除清理）。
//   故降级为：用 HTTP 创建真实 schema_v2 group attempt，再用 store API 直接构造
//   「已发布 handoff」状态（写 handoff 文件 + 设 unit.latest_handoff_revision_id），
//   然后调 HTTP DELETE 断言 lineage 被清理。降级仍覆盖清理逻辑，仅不走完整 runner。
//
// 常量派生规则（与 group_initialization 一致）：
//   group attempt 的 unit id 按 order_index 派生为 coding_unit_0001 / coding_unit_0002；
//   本测试不改动 ID 派生规则，只是按既定规则定位已创建的 unit。
// ============================================================================

const DELETION_HANDOFF_PLAN_ID: &str = "work_item_plan_0001";

/// 已发布 handoff 的最小描述：逻辑 work item、对应 group unit、handoff revision id。
struct SeededHandoff {
    logical_work_item_id: &'static str,
    unit_id: &'static str,
    handoff_revision_id: &'static str,
}

/// 构造一个「已发布 handoff」的 schema_v2 group attempt 状态。
///
/// 步骤：HTTP 创建真实 group attempt → store 直构 handoff lineage（写文件 +
/// 设 unit.latest_handoff_revision_id）。返回 DELETE 后断言所需的全部句柄。
async fn seed_group_attempt_with_published_handoffs(
    app: axum::Router,
    app_paths: &ProductAppPaths,
    handoffs: &[SeededHandoff],
) -> (
    axum::Router,
    String,
    Vec<PathBuf>,
    WorkItemPlanLineage,
    WorkItemRevisionStore,
) {
    let issue_root = app_paths.issue_root("project_0001", "issue_0001");
    fs::remove_dir_all(issue_root.join("work-items")).expect("remove legacy work items");
    fs::remove_dir_all(issue_root.join("verification-plans"))
        .expect("remove legacy verification plans");

    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "group attempt creation: {body}");
    let attempt_id = assert_global_attempt_id(&body);

    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", DELETION_HANDOFF_PLAN_ID)
        .expect("seeded plan lineage");

    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let mut handoff_paths = Vec::new();
    for handoff in handoffs {
        let revision = HandoffRevision {
            id: handoff.handoff_revision_id.to_string(),
            logical_work_item_id: handoff.logical_work_item_id.to_string(),
            work_item_revision_id: format!("work_item_revision_{}", handoff.logical_work_item_id),
            coding_unit_run_id: format!("{}_run", handoff.unit_id),
            provided_contracts: Vec::new(),
            provided_capabilities: BTreeMap::new(),
            contract_hash: format!("hash_{}", handoff.handoff_revision_id),
            commit_sha: format!("commit_{}", handoff.handoff_revision_id),
            created_at: "2026-07-29T00:00:00Z".to_string(),
        };
        revision_store
            .put_handoff_revision(&lineage, &revision)
            .expect("seed handoff revision");
        let path = app_paths
            .issue_root("project_0001", "issue_0001")
            .join("work-item-revisions")
            .join(DELETION_HANDOFF_PLAN_ID)
            .join("logical-work-items")
            .join(handoff.logical_work_item_id)
            .join("handoff-revisions")
            .join(format!("{}.json", handoff.handoff_revision_id));
        assert!(path.exists(), "precondition: handoff published at {}", path.display());
        coding_store
            .update_coding_unit_latest_handoff_revision_id(
                "project_0001",
                "issue_0001",
                &attempt_id,
                handoff.unit_id,
                Some(handoff.handoff_revision_id.to_string()),
            )
            .expect("seed unit latest handoff revision id");
        handoff_paths.push(path);
    }

    (app, attempt_id, handoff_paths, lineage, revision_store)
}

#[tokio::test]
async fn delete_coding_attempt_removes_handoff_revision_for_completed_unit() {
    // 单 unit 已发布 handoff：DELETE attempt 后该 handoff 文件应从 lineage 删除。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));

    let (app, attempt_id, handoff_paths, _lineage, _store) =
        seed_group_attempt_with_published_handoffs(
            app,
            &app_paths,
            &[SeededHandoff {
                logical_work_item_id: "work_item_0001",
                unit_id: "coding_unit_0001",
                handoff_revision_id: "handoff_revision_coding_unit_run_0001",
            }],
        )
        .await;
    assert_eq!(handoff_paths.len(), 1);
    let handoff_path = &handoff_paths[0];

    let (status, _body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert!(
        !handoff_path.exists(),
        "handoff revision must be removed from lineage after DELETE: {}",
        handoff_path.display()
    );
}

#[tokio::test]
async fn delete_coding_attempt_removes_all_handoff_revisions_for_completed_units() {
    // 多 unit 均已发布 handoff：DELETE attempt 后所有 handoff 文件应从 lineage 删除。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));

    let (app, attempt_id, handoff_paths, _lineage, _store) =
        seed_group_attempt_with_published_handoffs(
            app,
            &app_paths,
            &[
                SeededHandoff {
                    logical_work_item_id: "work_item_0001",
                    unit_id: "coding_unit_0001",
                    handoff_revision_id: "handoff_revision_coding_unit_run_0001",
                },
                SeededHandoff {
                    logical_work_item_id: "work_item_0002",
                    unit_id: "coding_unit_0002",
                    handoff_revision_id: "handoff_revision_coding_unit_run_0002",
                },
            ],
        )
        .await;
    assert!(
        handoff_paths.iter().all(|p| p.exists()),
        "precondition: all handoffs published"
    );

    let (status, _body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    for path in &handoff_paths {
        assert!(
            !path.exists(),
            "handoff revision must be removed from lineage after DELETE: {}",
            path.display()
        );
    }
}

#[tokio::test]
async fn delete_coding_attempt_without_handoff_does_not_touch_lineage() {
    // 无 unit 认领 handoff：清理逻辑应为空操作——DELETE 正常返回 204，且不抛错、不误删。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));

    let (app, attempt_id, handoff_paths, lineage, _store) =
        seed_group_attempt_with_published_handoffs(app, &app_paths, &[]).await;
    assert!(handoff_paths.is_empty(), "no handoffs seeded");

    // lineage 下确无任何 handoff revision，作为「空操作」基线。
    assert!(
        WorkItemRevisionStore::new(app_paths.clone())
            .list_handoff_revisions(&lineage, "work_item_0001")
            .expect("list handoffs work_item_0001")
            .is_empty()
    );

    let (status, _body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 删除正常完成且 lineage 未受影响（仍为空，未抛错）。
    assert!(
        WorkItemRevisionStore::new(app_paths)
            .list_handoff_revisions(&lineage, "work_item_0001")
            .expect("list handoffs work_item_0001 after delete")
            .is_empty()
    );
}

#[tokio::test]
async fn delete_coding_attempt_then_rebuild_does_not_conflict_on_handoff() {
    // 删除后重建：第一个 attempt 的 handoff 被清理后，对同一 plan 重新创建 attempt
    // 不应报 group_completion_handoff_revision_conflict；重建成功。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));

    let (app, first_attempt_id, handoff_paths, _lineage, _store) =
        seed_group_attempt_with_published_handoffs(
            app,
            &app_paths,
            &[SeededHandoff {
                logical_work_item_id: "work_item_0001",
                unit_id: "coding_unit_0001",
                handoff_revision_id: "handoff_revision_coding_unit_run_0001",
            }],
        )
        .await;
    assert_eq!(handoff_paths.len(), 1);

    let (status, _body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&first_attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        !handoff_paths[0].exists(),
        "precondition for rebuild: handoff cleaned after DELETE"
    );

    // 重建：对同一 plan 创建新 group attempt，断言不返回冲突且成功。
    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rebuild response: {body}");
    assert_ne!(body["code"], "group_completion_handoff_revision_conflict");
    let second_attempt_id = assert_global_attempt_id(&body);
    assert_ne!(second_attempt_id, first_attempt_id);
}
