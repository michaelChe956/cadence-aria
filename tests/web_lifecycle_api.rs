use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use std::{fs, process::Command};
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn issue_creation_requires_repository_and_lifecycle_lists_cards() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;

    let (status, missing_repo) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"Missing repo","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(missing_repo["code"], "repository_required");

    let (status, issue) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title":"登录会话过期",
            "description":"需要结合前端代码提示用户重新登录",
            "repository_id":"repository_0001"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issue["repo_id"], "repository_0001");

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lifecycle["issue"]["issue_id"], "issue_0001");
    assert_eq!(lifecycle["story_specs"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["design_specs"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn generate_endpoints_create_workspace_sessions_and_first_cards() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    let (status, story_response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        story_response["story_specs"][0]["story_spec_id"],
        "story_spec_0001"
    );
    assert_eq!(
        story_response["workspace_session"]["workspace_type"],
        "story"
    );

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lifecycle["story_specs"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn generating_story_specs_returns_404_when_bound_repository_was_deleted() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title":"登录会话过期",
            "description":"描述",
            "repository_id":"repository_0001"
        }),
    )
    .await;
    request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/repositories/repository_0001",
        json!({}),
    )
    .await;

    let (status, error) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], "repository_not_found");
}

#[tokio::test]
async fn workspace_session_message_run_and_confirm_update_session_state() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;

    let (status, message) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/message",
        json!({"role":"user","content":"请强调重新登录按钮"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(message["messages"][0]["content"], "请强调重新登录按钮");

    let (status, running) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/run-next",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(running["status"], "waiting_for_human");

    let (status, confirmed) = request_json(
        app,
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(confirmed["status"], "confirmed");
}

#[tokio::test]
async fn workspace_session_missing_message_and_run_next_return_not_found() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;

    let (status, message_error) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_missing/message",
        json!({"role":"user","content":"请强调重新登录按钮"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(message_error["code"], "workspace_session_not_found");

    let (status, run_next_error) = request_json(
        app,
        Method::POST,
        "/api/workspace-sessions/workspace_session_missing/run-next",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(run_next_error["code"], "workspace_session_not_found");
}

#[tokio::test]
async fn workspace_session_ambiguous_returns_conflict() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"重复会话","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    let first_session_path = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/workspace-sessions/workspace_session_0001.json");
    let duplicate_root = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0002/workspace-sessions");
    fs::create_dir_all(&duplicate_root).expect("duplicate workspace sessions root");
    fs::copy(
        first_session_path,
        duplicate_root.join("workspace_session_0001.json"),
    )
    .expect("duplicate workspace session");

    let (status, error) = request_json(
        app,
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/message",
        json!({"role":"user","content":"请强调重新登录按钮"}),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "workspace_session_ambiguous");
}

#[tokio::test]
async fn workspace_session_message_rejects_invalid_role_and_empty_content() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;

    for body in [
        json!({"role":"","content":"请强调重新登录按钮"}),
        json!({"role":"unknown","content":"请强调重新登录按钮"}),
        json!({"role":"user","content":"   "}),
    ] {
        let (status, error) = request_json(
            app.clone(),
            Method::POST,
            "/api/workspace-sessions/workspace_session_0001/message",
            body,
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["code"], "invalid_workspace_message");
    }
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
