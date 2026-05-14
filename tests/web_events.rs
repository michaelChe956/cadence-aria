use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use futures_util::StreamExt;
use serde_json::json;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

#[test]
fn event_hub_records_events_with_incrementing_cursor() {
    let hub = EventHub::new();
    let first = hub.publish(
        "projection_updated",
        Some("task_0001"),
        json!({"version":1}),
    );
    let second = hub.publish(
        "paused_for_approval",
        Some("task_0001"),
        json!({"node_id":"N16"}),
    );

    assert_eq!(first.cursor, 1);
    assert_eq!(second.cursor, 2);
    let replay = hub.replay_after(0);
    assert_eq!(replay.len(), 2);
    assert_eq!(replay[1].event_type, "paused_for_approval");
}

#[tokio::test]
async fn events_route_replays_only_after_cursor() {
    let workspace = tempdir().expect("workspace");
    let hub = EventHub::new();
    let first = hub.publish(
        "provider.input_prepared",
        Some("task_0001"),
        json!({"input_ref":"run_n16_0001"}),
    );
    hub.publish(
        "provider.output_stream",
        Some("task_0001"),
        json!({"text":"chunk"}),
    );
    let state = WebAppState::with_events(
        workspace.path().to_path_buf(),
        WebRuntime::new_fake(workspace.path().to_path_buf()),
        hub,
    );
    let app = build_web_router(state);

    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/events?cursor={}", first.cursor))
        .body(Body::empty())
        .expect("request");
    let response = app.oneshot(request).await.expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let first_chunk = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("sse chunk")
        .expect("body item")
        .expect("body bytes");
    let text = std::str::from_utf8(&first_chunk).expect("sse utf8");

    assert!(text.contains("provider.output_stream"));
    assert!(!text.contains("provider.input_prepared"));
}
