use cadence_aria::protocol::repl_wire::{
    event_registry, validate_event_payload, Command, EventEnvelope, HelloRequest, MessageType,
    RequestEnvelope, ResponseEnvelope, WireError, PROTOCOL_VERSION,
};
use serde_json::json;

#[test]
fn request_envelope_serializes_with_snake_case_fields() {
    let envelope = RequestEnvelope::new(
        "req_001",
        Command::Hello,
        HelloRequest {
            last_seen_event_id: Some(41),
        },
    )
    .expect("hello request should serialize");

    let value = serde_json::to_value(envelope).expect("request envelope to json");

    assert_eq!(value["protocol_version"], PROTOCOL_VERSION);
    assert_eq!(value["message_type"], "request");
    assert_eq!(value["request_id"], "req_001");
    assert_eq!(value["command"], "hello");
    assert_eq!(value["payload"]["last_seen_event_id"], 41);
    assert!(value.get("lastSeenEventId").is_none());
}

#[test]
fn response_envelope_requires_error_when_not_ok() {
    let response = ResponseEnvelope::failure(
        "req_001",
        Command::Hello,
        WireError {
            code: "replay_window_lost".to_string(),
            message: "event replay window is no longer available".to_string(),
            details: None,
        },
    );

    let value = serde_json::to_value(response).expect("response envelope to json");

    assert_eq!(value["message_type"], "response");
    assert_eq!(value["ok"], false);
    assert_eq!(value["command"], "hello");
    assert_eq!(value["error"]["code"], "replay_window_lost");
}

#[test]
fn event_registry_contains_p1_reserved_event_types() {
    let event_types: Vec<_> = event_registry()
        .iter()
        .map(|schema| schema.event_type)
        .collect();

    for expected in [
        "task.created",
        "task.phase_changed",
        "artifact.materialized",
        "projection.compiled",
        "constraint_bundle.compiled",
        "traceability.updated",
        "gate.opened",
        "gate.resolved",
        "provider_run.started",
        "provider_run.completed",
        "provider_run.failed",
        "policy_mode.degraded",
        "worktree.lease_acquired",
        "openspec.rollback",
    ] {
        assert!(
            event_types.contains(&expected),
            "missing event type {expected}"
        );
    }
}

#[test]
fn event_payload_validation_requires_minimum_fields() {
    validate_event_payload(
        "task.created",
        &json!({
            "task_id": "task_001",
            "phase": "intake"
        }),
    )
    .expect("task.created minimum payload should be valid");

    let error = validate_event_payload("task.created", &json!({ "phase": "intake" }))
        .expect_err("missing task_id should fail");
    assert_eq!(error.code, "invalid_event_payload");
}

#[test]
fn event_envelope_uses_occurred_at_not_created_at() {
    let event = EventEnvelope::new(
        7,
        "task.created",
        "2026-04-26T00:00:00Z",
        json!({
            "task_id": "task_001",
            "phase": "intake"
        }),
    )
    .expect("event envelope");

    let value = serde_json::to_value(event).expect("event to json");

    assert_eq!(value["message_type"], "event");
    assert_eq!(value["event_id"], 7);
    assert_eq!(value["occurred_at"], "2026-04-26T00:00:00Z");
    assert!(value.get("created_at").is_none());
}

#[test]
fn message_type_rejects_unknown_values() {
    let error = serde_json::from_value::<MessageType>(json!("unknown"))
        .expect_err("unknown message type should fail");
    assert!(error.to_string().contains("unknown variant"));
}
