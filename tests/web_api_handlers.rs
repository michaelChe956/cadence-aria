use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
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
