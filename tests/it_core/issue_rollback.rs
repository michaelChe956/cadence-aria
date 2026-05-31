use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::interactive::checkpoint::CheckpointService;
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

#[test]
fn rollback_rejects_repo_root_as_development_worktree() {
    let repo = tempdir().expect("repo");
    let service = CheckpointService::new(repo.path(), "task_0001");

    let result = service.ensure_reset_target_is_not_repo_root(repo.path());

    assert!(result.is_err());
}

#[tokio::test]
async fn issue_rollback_api_rejects_missing_worktree_without_reset() {
    let root = tempdir().expect("root");
    let events = EventHub::new();
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        events,
    );
    let app = build_web_router(state);

    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/issues/issue_0001/rollback/preview",
        json!({"execution_record_id":"execution_0001"}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(response["code"], "issue_rollback_missing_worktree");
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
