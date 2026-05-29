use std::fs;
use std::path::PathBuf;
use std::process::Command;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::CodingAttemptStore;
use cadence_aria::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingExecutionStage, CodingTimelineNode,
    CodingTimelineNodeStatus, FindingSeverity, InternalPrReview, PushStatus, RemoteKind,
    ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict, TestCommand, TestCommandStatus,
    TestingOverallStatus, TestingReport,
};
use cadence_aria::product::models::ProviderName;
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
    assert_eq!(
        attempt["branch_name"],
        "aria/work-items/work_item_0001/attempt-1"
    );

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
async fn reads_test_output_artifact_from_attempt_store() {
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
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let artifact_dir = store.attempt_test_output_root(
        "project_0001",
        "issue_0001",
        attempt["attempt_id"].as_str().expect("attempt id"),
    );
    fs::create_dir_all(&artifact_dir).expect("artifact dir");
    fs::write(artifact_dir.join("unit.stdout.log"), "unit stdout\n").expect("artifact");

    let (status, artifact) = request_json(
        app,
        Method::GET,
        "/api/coding-attempts/coding_attempt_0001/artifacts/unit.stdout.log",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(artifact["artifact_ref"], "unit.stdout.log");
    assert_eq!(artifact["artifact_kind"], "coding_attempt_artifact");
    assert_eq!(artifact["content_type"], "text/plain");
    assert_eq!(artifact["content"], "unit stdout\n");
}

async fn bootstrap_confirmed_work_item(app: axum::Router, repo_path: &std::path::Path) {
    bootstrap_confirmed_work_item_with_providers(app, repo_path, "fake", "fake").await;
}

async fn bootstrap_confirmed_work_item_with_providers(
    app: axum::Router,
    repo_path: &std::path::Path,
    work_item_author_provider: &str,
    work_item_reviewer_provider: &str,
) {
    bootstrap_story_and_design(app.clone(), repo_path).await;
    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title":"实现爬楼梯",
            "story_spec_ids":["story_spec_0001"],
            "design_spec_ids":["design_spec_0001"],
            "author_provider": work_item_author_provider,
            "reviewer_provider": work_item_reviewer_provider
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = request_json(
        app,
        Method::POST,
        "/api/workspace-sessions/workspace_session_0003/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

async fn bootstrap_unconfirmed_work_item(app: axum::Router, repo_path: &std::path::Path) {
    bootstrap_story_and_design(app.clone(), repo_path).await;
    let (status, _) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title":"实现爬楼梯",
            "story_spec_ids":["story_spec_0001"],
            "design_spec_ids":["design_spec_0001"],
            "author_provider":"fake",
            "reviewer_provider":"fake"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

async fn bootstrap_story_and_design(app: axum::Router, repo_path: &std::path::Path) {
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Coding","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo_path}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"爬楼梯","description":"实现 O(n) 算法","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"爬楼梯 Story","author_provider":"fake","reviewer_provider":"fake"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"爬楼梯 Design",
            "story_spec_ids":["story_spec_0001"],
            "design_kind":"backend",
            "author_provider":"fake",
            "reviewer_provider":"fake"
        }),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0002/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
}

async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

fn git_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("repo");
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "aria@example.com"]);
    run_git(dir.path(), &["config", "user.name", "Aria Test"]);
    fs::write(dir.path().join("README.md"), "# repo\n").expect("seed readme");
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "initial"]);
    dir
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("git");
    assert!(status.success());
}

fn sample_testing_report(attempt_id: &str) -> TestingReport {
    TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        commands: vec![TestCommand {
            command: vec!["cargo".to_string(), "test".to_string()],
            cwd: PathBuf::from("/tmp/worktree"),
            exit_code: Some(0),
            duration_ms: 100,
            stdout_ref: "artifacts/stdout.txt".to_string(),
            stderr_ref: "artifacts/stderr.txt".to_string(),
            status: TestCommandStatus::Passed,
        }],
        overall_status: TestingOverallStatus::Passed,
        provider_claim: None,
        backend_verified: true,
        started_at: "2026-05-23T00:00:00Z".to_string(),
        completed_at: Some("2026-05-23T00:01:00Z".to_string()),
    }
}

fn sample_code_review_report(attempt_id: &str) -> CodeReviewReport {
    CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        round: 1,
        verdict: ReviewVerdict::Approve,
        findings: vec![sample_finding()],
        tested_evidence_refs: vec!["testing_report_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "基础 code review 通过".to_string(),
        created_at: "2026-05-23T00:01:00Z".to_string(),
    }
}

fn sample_review_request(attempt_id: &str) -> ReviewRequest {
    ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        commit_sha: "abc123".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: vec!["打开远端分支发起审查".to_string()],
        created_at: "2026-05-23T00:02:00Z".to_string(),
        updated_at: "2026-05-23T00:02:00Z".to_string(),
    }
}

fn sample_internal_review(attempt_id: &str, review_request_id: &str) -> InternalPrReview {
    InternalPrReview {
        id: "internal_review_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        review_request_id: review_request_id.to_string(),
        verdict: ReviewVerdict::Approve,
        findings: vec![sample_finding()],
        impact_scope: vec!["src/lib.rs".to_string()],
        pr_description: "实现 work item".to_string(),
        commit_message_suggestion: "feat: implement work item".to_string(),
        tested_evidence_refs: vec!["testing_report_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "最终审查通过".to_string(),
        created_at: "2026-05-23T00:03:00Z".to_string(),
    }
}

fn sample_finding() -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Info,
        file_path: Some("src/lib.rs".to_string()),
        line: Some(1),
        message: "ok".to_string(),
        required_action: None,
        source_stage: CodingExecutionStage::CodeReview,
    }
}

fn sample_completed_node(attempt_id: &str) -> CodingTimelineNode {
    CodingTimelineNode {
        id: "coding_node_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        stage: CodingExecutionStage::Testing,
        title: "测试".to_string(),
        status: CodingTimelineNodeStatus::Completed,
        agent_role: Some(CodingAgentRole::Tester),
        summary: Some("测试通过".to_string()),
        started_at: "2026-05-23T00:01:00Z".to_string(),
        completed_at: Some("2026-05-23T00:02:00Z".to_string()),
        artifact_refs: vec!["testing_report_0001".to_string()],
    }
}

fn sample_running_node(attempt_id: &str) -> CodingTimelineNode {
    CodingTimelineNode {
        id: "coding_node_0002".to_string(),
        attempt_id: attempt_id.to_string(),
        stage: CodingExecutionStage::CodeReview,
        title: "Code Review".to_string(),
        status: CodingTimelineNodeStatus::Running,
        agent_role: Some(CodingAgentRole::Reviewer),
        summary: None,
        started_at: "2026-05-23T00:02:00Z".to_string(),
        completed_at: None,
        artifact_refs: vec![],
    }
}
