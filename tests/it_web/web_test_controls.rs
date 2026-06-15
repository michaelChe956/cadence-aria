use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::CodingAttemptStore;
use cadence_aria::product::coding_models::CodingProviderRole;
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::sync::Mutex;
use tower::ServiceExt;

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn test_control_routes_are_disabled_without_e2e_env() {
    let _guard = ENV_LOCK.lock().await;
    unsafe {
        std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
    }
    let root = tempdir().expect("root");
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let status = request_status(
        app,
        Method::POST,
        "/api/test/workspace-sessions/workspace_session_0001/ws/drop",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_control_routes_update_shared_state_when_enabled() {
    let _guard = ENV_LOCK.lock().await;
    unsafe {
        std::env::set_var("ARIA_E2E_TEST_CONTROLS", "1");
    }
    let root = tempdir().expect("root");
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let controls = state.test_controls.clone();
    let app = build_web_router(state);

    let fixture = request_json(
        app.clone(),
        Method::POST,
        "/api/test/workspace-sessions/workspace_session_0001/permission-fixture",
        json!({"mode": "single-request"}),
    )
    .await;
    assert_eq!(fixture["status"], "ok");
    assert!(
        controls
            .consume_permission_fixture("workspace_session_0001")
            .await
    );

    let timeout = request_json(
        app.clone(),
        Method::POST,
        "/api/test/permission-timeout",
        json!({"timeout_ms": 500}),
    )
    .await;
    assert_eq!(timeout["status"], "ok");
    assert_eq!(controls.permission_timeout(), Duration::from_millis(500));

    let ws_timeout = request_json(
        app,
        Method::POST,
        "/api/test/ws-timeout",
        json!({"server_idle_timeout_ms": 750}),
    )
    .await;
    assert_eq!(ws_timeout["status"], "ok");
    assert_eq!(controls.server_idle_timeout(), Duration::from_millis(750));

    unsafe {
        std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
    }
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

async fn request_status(app: axum::Router, method: Method, uri: &str, body: Value) -> StatusCode {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    app.oneshot(request).await.expect("response").status()
}

#[tokio::test]
async fn coding_role_run_fixture_seed_route_creates_attempt_with_runs() {
    let _guard = ENV_LOCK.lock().await;
    unsafe {
        std::env::set_var("ARIA_E2E_TEST_CONTROLS", "1");
    }
    let root = tempdir().expect("root");
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let body = request_json(
        app.clone(),
        Method::POST,
        "/api/test/coding-attempts/role-run-fixture",
        json!({"blocked_stage":"rework"}),
    )
    .await;

    assert_eq!(body["attempt_id"], "coding_attempt_0001");
    assert_eq!(body["project_id"], "project_0001");
    assert_eq!(body["issue_id"], "issue_0001");

    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    assert_eq!(runs.len(), 2);
    assert!(
        runs.iter()
            .any(|run| run.role == CodingProviderRole::Tester)
    );
    assert!(
        runs.iter()
            .any(|run| run.role == CodingProviderRole::Analyst)
    );

    unsafe {
        std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
    }
}
