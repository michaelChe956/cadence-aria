//! 项目删除二次确认（2026-11-05 真机事故回归）：
//! `DELETE /api/projects/{project_id}` 未携带显式确认意图（`?confirmed=true`）
//! MUST 以稳定码 `deletion_confirmation_required`（409）拒绝且项目原样保留；
//! 携带确认意图才执行硬删。确认口径对齐级联删除 D2.3（不新造码），
//! 且由后端强制——不依赖前端自律。
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

fn app(root: &std::path::Path) -> axum::Router {
    let state = WebAppState::with_events(
        root.to_path_buf(),
        WebRuntime::new_fake(root.to_path_buf()),
        EventHub::new(),
    );
    build_web_router(state)
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

/// 无确认（缺 query 或显式 confirmed=false）⇒ 409 稳定码，项目仍在。
#[tokio::test]
async fn project_delete_without_confirmation_is_rejected_and_project_survives() {
    let root = tempdir().expect("root");
    let app = app(root.path());

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Aria","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 无 query：必须 409 + 稳定码，不得落删除。
    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "deletion_confirmation_required");

    // 显式 confirmed=false 同样拒绝：确认意图必须是显式 true。
    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001?confirmed=false",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "deletion_confirmation_required");

    // 两次拒绝后项目仍在（拒绝路径零副作用）。
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
}

/// 携带 confirmed=true 才真正硬删：200 回执 + 项目消失。
#[tokio::test]
async fn project_delete_with_confirmation_removes_project() {
    let root = tempdir().expect("root");
    let app = app(root.path());

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Aria","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001?confirmed=true",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["status"], "deleted");

    let (status, _) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, projects) = request_json(app, Method::GET, "/api/projects", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(projects["projects"].as_array().expect("projects").len(), 0);
}
