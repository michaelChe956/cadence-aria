use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use std::fs;
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

    let task_id = started["task_id"].as_str().expect("task id");
    let artifact_dir = repo_b
        .path()
        .join(".aria/runtime/tasks")
        .join(task_id)
        .join("artifacts/spec");
    fs::create_dir_all(&artifact_dir).expect("artifact dir");
    fs::write(
        artifact_dir.join("story.json"),
        json!({
            "artifact_ref": "spec_story_0001",
            "artifact_kind": "spec",
            "producer_node": "N05",
            "markdown": "# Spec\n\nStory spec content"
        })
        .to_string(),
    )
    .expect("artifact");

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
    assert_eq!(issues[0]["artifacts"][0]["artifact_ref"], "spec_story_0001");
    assert_eq!(issues[0]["artifacts"][0]["stage"], "story_spec");
}

#[tokio::test]
async fn starting_product_issue_again_reuses_existing_execution_workspace() {
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

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Product Project","description":"Issue lifecycle"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Workspace A","path":repo_a.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Workspace B","path":repo_b.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"执行一次即可","description":"再次点击应该跳转"}),
    )
    .await;

    let (status, first) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/start",
        json!({"workspace_id":"repository_0001","provider_mode":"fake"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        first["workspace_id"],
        "product:project_0001:repository_0001"
    );
    assert_eq!(first["repository_id"], "repository_0001");

    let (status, second) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/start",
        json!({"workspace_id":"repository_0002","provider_mode":"fake"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["workspace_id"], first["workspace_id"]);
    assert_eq!(second["repository_id"], "repository_0001");
    assert_eq!(second["task_id"], first["task_id"]);
    assert_eq!(second["session_id"], first["session_id"]);

    let (status, issues) = request_json(
        app,
        Method::GET,
        "/api/projects/project_0001/issues",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issues["issues"][0]["workspace_id"], first["workspace_id"]);
    assert_eq!(issues["issues"][0]["task_id"], first["task_id"]);
    assert_eq!(issues["issues"][0]["session_id"], first["session_id"]);
}

#[tokio::test]
async fn deletes_workspace_project_repository_and_issue_records() {
    let root = tempdir().expect("root");
    let workspace_a = git_repo();
    let workspace_b = git_repo();
    let repo = git_repo();
    let events = EventHub::new();
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        events,
    );
    let app = build_web_router(state);

    request_json(
        app.clone(),
        Method::POST,
        "/api/workspaces",
        json!({"name":"Workspace A","path":workspace_a.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspaces",
        json!({"name":"Workspace B","path":workspace_b.path()}),
    )
    .await;
    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/workspaces/workspace_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, workspaces) =
        request_json(app.clone(), Method::GET, "/api/workspaces", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let workspaces = workspaces["workspaces"].as_array().expect("workspaces");
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0]["workspace_id"], "workspace_0002");

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Product","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Code Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"Issue to delete","description":null}),
    )
    .await;

    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/repositories/repository_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
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
        0
    );

    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, issues) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/issues",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issues["issues"].as_array().expect("issues").len(), 0);

    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, projects) = request_json(app, Method::GET, "/api/projects", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(projects["projects"].as_array().expect("projects").len(), 0);
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
