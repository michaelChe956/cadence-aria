use cadence_aria::web::events::EventHub;
use serde_json::json;

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
