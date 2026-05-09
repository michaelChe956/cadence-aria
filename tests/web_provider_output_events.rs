use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::{EventHub, WebEventType};
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::types::ProviderOutputChunk;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

#[test]
fn provider_output_event_carries_stdout_stderr_structured_output_gate_and_retry() {
    let hub = EventHub::new();
    let event = hub.publish_provider_output(
        Some("task_0001"),
        ProviderOutputChunk {
            node_id: "N16".to_string(),
            provider_run_id: "run_n16_0001".to_string(),
            stream: "stdout".to_string(),
            text: "running tests".to_string(),
            structured_output: Some(json!({"artifact_kind":"coding_report"})),
            manual_gate: Some("approval_required".to_string()),
            retry_attempt: Some(1),
        },
    );

    assert_eq!(event.event_type, WebEventType::ProviderOutput.as_str());
    assert_eq!(event.payload["stream"], "stdout");
    assert_eq!(
        event.payload["structured_output"]["artifact_kind"],
        "coding_report"
    );
    assert_eq!(event.payload["manual_gate"], "approval_required");
    assert_eq!(event.payload["retry_attempt"], 1);
}

#[test]
fn provider_auth_failure_is_classified_for_diagnostics_panel() {
    let workspace = tempdir().expect("workspace");
    let runtime = WebRuntime::new_fake(workspace.path().to_path_buf());
    let diagnostic =
        runtime.provider_command_diagnostic("codex", "command not found or not authenticated");

    assert_eq!(diagnostic["category"], "provider_error");
    assert_eq!(
        diagnostic["code"],
        "provider_authorization_or_command_unavailable"
    );
    assert!(
        diagnostic["message"]
            .as_str()
            .expect("message")
            .contains("codex")
    );
}

#[tokio::test]
async fn stop_task_handler_returns_stop_requested_and_publishes_projection_update() {
    let workspace = tempdir().expect("workspace");
    let state = WebAppState::new(
        workspace.path().to_path_buf(),
        WebRuntime::new_fake(workspace.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());

    let response = request_json(app, Method::POST, "/api/tasks/task_0001/stop", json!({})).await;

    assert_eq!(response["status"], "stop_requested");
    assert_eq!(response["task_id"], "task_0001");
    let events = state.events.replay_after(0);
    assert!(events.iter().any(|event| {
        event.event_type == WebEventType::ProjectionUpdated.as_str()
            && event.task_id.as_deref() == Some("task_0001")
            && event.payload["reason"] == "stop_requested"
    }));
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
