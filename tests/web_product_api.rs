use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use std::process::Command;
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn creates_project_repository_and_issue_via_product_api() {
    let root = tempdir().expect("root");
    let events = EventHub::new();
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        events,
    );
    let app = build_web_router(state);

    let (status, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Aria","description":"Workbench"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["project_id"], "project_0001");
    assert_eq!(created["name"], "Aria");

    let (status, project) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(project["project_id"], "project_0001");
    assert_eq!(project["name"], "Aria");

    let (status, opened) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/open",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(opened["project_id"], "project_0001");
    assert!(opened["last_opened_at"].is_string());

    let (status, missing) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_missing",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(missing["code"], "project_not_found");
    assert_eq!(missing["message"], "project not found");
    let missing_text = missing.to_string();
    assert!(!missing_text.contains(".aria"));
    assert!(!missing_text.contains("project.json"));
    assert!(!missing_text.contains(&root.path().display().to_string()));

    let projects = app
        .oneshot(
            Request::builder()
                .uri("/api/projects")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list");
    assert_eq!(projects.status(), StatusCode::OK);
}

#[tokio::test]
async fn manages_workspace_repositories_and_runs_issue_in_selected_repository() {
    let root = tempdir().expect("root");
    let repo_a = git_repo();
    let repo_b = git_repo();
    let events = EventHub::new();
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        events,
    );
    let app = build_web_router(state);

    let (status, workspace) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Product Workspace","description":"Issue lifecycle"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(workspace["project_id"], "project_0001");

    let (status, repository_a) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo A","path":repo_a.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(repository_a["repository_id"], "repository_0001");
    assert_eq!(repository_a["project_id"], "project_0001");

    let (status, repository_b) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo B","path":repo_b.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(repository_b["repository_id"], "repository_0002");

    let (status, repositories) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/repositories",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        repositories["repositories"]
            .as_array()
            .expect("repositories")
            .len(),
        2
    );

    let (status, issue) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title":"优化任务管理页面",
            "description":"展示 story spec、design spec、work item 和完成状态"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issue["issue_id"], "issue_0001");
    assert_eq!(issue["project_id"], "project_0001");
    assert!(issue["repo_id"].is_null());
    assert_eq!(issue["phase"], "clarification");
    assert_eq!(issue["status"], "draft");

    let (status, started) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/start",
        json!({"repository_id":"repository_0002","provider_mode":"fake"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(started["issue_id"], "issue_0001");
    assert_eq!(started["repository_id"], "repository_0002");
    assert_eq!(
        started["workspace_id"],
        "product:project_0001:repository_0002"
    );
    assert!(
        started["task_id"]
            .as_str()
            .unwrap_or_default()
            .starts_with("task_")
    );

    let projection_uri = format!(
        "/api/projection?workspace_id={}&task_id={}",
        started["workspace_id"].as_str().expect("workspace id"),
        started["task_id"].as_str().expect("task id")
    );
    let projection = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(projection_uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("projection");
    assert_eq!(projection.status(), StatusCode::OK);

    let (status, issues) = request_json(
        app,
        Method::GET,
        "/api/projects/project_0001/issues",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let issues = issues["issues"].as_array().expect("issues");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0]["repo_id"], "repository_0002");
    assert_eq!(issues[0]["phase"], "development");
    assert_eq!(issues[0]["status"], "in_progress");
    assert_eq!(issues[0]["active_binding_id"], "binding_0001");
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
    let status = Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(status.success());
    dir
}
