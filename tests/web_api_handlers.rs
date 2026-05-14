use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use std::process::Command;
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn api_create_advance_confirm_projection_contract() {
    let workspace = tempdir().expect("workspace");
    let state = WebAppState::new(
        workspace.path().to_path_buf(),
        WebRuntime::new_fake(workspace.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let create = request_json(
        app.clone(),
        Method::POST,
        "/api/tasks",
        json!({
            "request_text":"实现 Fibonacci square sum",
            "change_id":"aria-fibonacci-square",
            "policy_preset":"manual-write",
            "provider_mode":"fake",
            "timeout_secs":2400
        }),
    )
    .await;
    assert_eq!(create["task_id"], "task_0001");

    let advance = request_json(
        app.clone(),
        Method::POST,
        "/api/tasks/task_0001/advance",
        json!({}),
    )
    .await;
    assert_eq!(advance["status"], "paused_for_approval");
    assert_eq!(advance["pending_step"]["node_id"], "N16");

    let confirm = request_json(
        app.clone(),
        Method::POST,
        "/api/tasks/task_0001/confirm",
        json!({"checkpoint_id":"ckpt_0001","prompt":"确认执行","policy_override":null}),
    )
    .await;
    assert_eq!(confirm["node_id"], "N16");

    let projection = request_json(
        app,
        Method::GET,
        "/api/projection?task_id=task_0001",
        json!({}),
    )
    .await;
    assert_eq!(projection["active_task_id"], "task_0001");
}

#[tokio::test]
async fn api_workspace_issue_start_contract() {
    let app_root = tempdir().expect("app root");
    let workspace = git_repo();
    let state = WebAppState::new(
        app_root.path().to_path_buf(),
        WebRuntime::new_fake(app_root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let created_workspace = request_json(
        app.clone(),
        Method::POST,
        "/api/workspaces",
        json!({
            "name": "Repo",
            "path": workspace.path().display().to_string()
        }),
    )
    .await;
    assert_eq!(created_workspace["workspace_id"], "workspace_0001");
    assert_eq!(created_workspace["name"], "Repo");

    let created_issue = request_json(
        app.clone(),
        Method::POST,
        "/api/issues",
        json!({
            "title": "Implement picker",
            "description": "Start with workspace"
        }),
    )
    .await;
    assert_eq!(created_issue["issue_id"], "issue_0001");
    assert_eq!(created_issue["status"], "draft");

    let started = request_json(
        app,
        Method::POST,
        "/api/issues/issue_0001/start",
        json!({"workspace_id":"workspace_0001"}),
    )
    .await;
    assert_eq!(started["issue_id"], "issue_0001");
    assert_eq!(started["workspace_id"], "workspace_0001");
    assert_eq!(started["task_id"], "task_0001");
    assert_eq!(started["session_id"], "sess_task_0001");
}

#[tokio::test]
async fn api_workspace_aware_execution_contract() {
    let app_root = tempdir().expect("app root");
    let workspace = git_repo();
    let state = WebAppState::new(
        app_root.path().to_path_buf(),
        WebRuntime::new_fake(app_root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    request_json(
        app.clone(),
        Method::POST,
        "/api/workspaces",
        json!({
            "name": "Repo",
            "path": workspace.path().display().to_string()
        }),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/issues",
        json!({"title": "Execute in selected workspace"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/issues/issue_0001/start",
        json!({"workspace_id":"workspace_0001"}),
    )
    .await;

    let projection = request_json(
        app.clone(),
        Method::GET,
        "/api/projection?workspace_id=workspace_0001&task_id=task_0001",
        json!({}),
    )
    .await;
    assert_eq!(
        projection["workspace_root"],
        workspace
            .path()
            .canonicalize()
            .expect("canonical workspace")
            .display()
            .to_string()
    );
    assert_eq!(projection["active_task_id"], "task_0001");

    let advance = request_json(
        app,
        Method::POST,
        "/api/tasks/task_0001/advance?workspace_id=workspace_0001",
        json!({}),
    )
    .await;
    assert_eq!(advance["status"], "paused_for_approval");
    assert!(
        workspace
            .path()
            .join(".aria/runtime/tasks/task_0001/pending/provider-step.json")
            .exists()
    );
}

async fn request_json(app: axum::Router, method: Method, uri: &str, body: Value) -> Value {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

fn git_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("workspace");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(status.success());
    dir
}
