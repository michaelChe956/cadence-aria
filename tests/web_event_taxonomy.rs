use cadence_aria::web::events::{EventHub, WebEventType};
use serde_json::json;

#[test]
fn event_taxonomy_contains_every_design_event_type() {
    let expected = vec![
        "projection_updated",
        "node_started",
        "node_completed",
        "node_failed",
        "paused_for_approval",
        "provider.input_prepared",
        "provider_output",
        "artifact_written",
        "gate_blocked",
        "checkpoint_created",
        "rollback_previewed",
        "rollback_completed",
        "error",
    ];
    let actual = WebEventType::all()
        .into_iter()
        .map(|event_type| event_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn event_hub_can_publish_all_design_event_types() {
    let hub = EventHub::new();
    for event_type in WebEventType::all() {
        hub.publish(event_type.as_str(), Some("task_0001"), json!({"ok": true}));
    }
    let replay = hub.replay_after(0);
    assert_eq!(replay.len(), 13);
    assert_eq!(replay[0].event_type, "projection_updated");
    assert_eq!(replay[12].event_type, "error");
}
