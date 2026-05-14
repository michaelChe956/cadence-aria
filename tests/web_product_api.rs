use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
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
    let value = serde_json::from_slice(&bytes).expect("json");
    (status, value)
}
