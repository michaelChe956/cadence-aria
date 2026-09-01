use super::*;

#[test]
fn conversational_gate_outbound_events_roundtrip_with_immutable_prefixes() {
    let events = [
        (
            WsOutMessage::HumanGateTurnOpen {
                turn_id: "turn-001".to_string(),
                command_id: "cmd-001".to_string(),
                remaining_budget: 2,
            },
            "human_gate_turn_open",
        ),
        (
            WsOutMessage::HumanGateTurnCompleted {
                turn_id: "turn-001".to_string(),
                artifact_ref: "artifact://candidate/001".to_string(),
            },
            "human_gate_turn_completed",
        ),
        (
            WsOutMessage::HumanGateTurnFailed {
                turn_id: "turn-001".to_string(),
                failure_class: "provider_failed".to_string(),
                message: "provider unavailable".to_string(),
            },
            "human_gate_turn_failed",
        ),
        (
            WsOutMessage::HumanGateBusy {
                turn_id: "turn-001".to_string(),
            },
            "human_gate_busy",
        ),
        (
            WsOutMessage::HumanGateClosed {
                decision: "confirm".to_string(),
                stage: "completed".to_string(),
            },
            "human_gate_closed",
        ),
        (
            WsOutMessage::AdvanceCompleted {
                command_id: "cmd-002".to_string(),
                attempt_id: "attempt-001".to_string(),
                workspace_entry: "workspace://work-item-group/001".to_string(),
            },
            "advance_completed",
        ),
        (
            WsOutMessage::AdvanceRejected {
                command_id: "cmd-003".to_string(),
                code: "PLAN_NOT_CONFIRMED".to_string(),
                reason: "plan must be confirmed before advance".to_string(),
            },
            "advance_rejected",
        ),
    ];

    for (event, expected_type) in events {
        let json = serde_json::to_value(&event).expect("serialize outbound event");
        assert_eq!(json["type"], expected_type);
        match &event {
            WsOutMessage::HumanGateTurnOpen {
                turn_id,
                command_id,
                remaining_budget,
            } => {
                assert_eq!(json["turn_id"], serde_json::json!(turn_id));
                assert_eq!(json["command_id"], serde_json::json!(command_id));
                assert_eq!(json["remaining_budget"], *remaining_budget);
            }
            WsOutMessage::HumanGateTurnCompleted {
                turn_id,
                artifact_ref,
            } => {
                assert_eq!(json["turn_id"], serde_json::json!(turn_id));
                assert_eq!(json["artifact_ref"], serde_json::json!(artifact_ref));
            }
            WsOutMessage::HumanGateTurnFailed {
                turn_id,
                failure_class,
                message,
            } => {
                assert_eq!(json["turn_id"], serde_json::json!(turn_id));
                assert_eq!(json["failure_class"], serde_json::json!(failure_class));
                assert_eq!(json["message"], serde_json::json!(message));
            }
            WsOutMessage::HumanGateBusy { turn_id } => {
                assert_eq!(json["turn_id"], serde_json::json!(turn_id));
            }
            WsOutMessage::HumanGateClosed { decision, stage } => {
                assert_eq!(json["decision"], serde_json::json!(decision));
                assert_eq!(json["stage"], serde_json::json!(stage));
            }
            WsOutMessage::AdvanceCompleted {
                command_id,
                attempt_id,
                workspace_entry,
            } => {
                assert_eq!(json["command_id"], serde_json::json!(command_id));
                assert_eq!(json["attempt_id"], serde_json::json!(attempt_id));
                assert_eq!(json["workspace_entry"], serde_json::json!(workspace_entry));
            }
            WsOutMessage::AdvanceRejected {
                command_id,
                code,
                reason,
            } => {
                assert_eq!(json["command_id"], serde_json::json!(command_id));
                assert_eq!(json["code"], serde_json::json!(code));
                assert_eq!(json["reason"], serde_json::json!(reason));
            }
            _ => unreachable!("event fixture only contains conversational gate events"),
        }
        assert!(!json.as_object().unwrap().contains_key("markdown"));
        let parsed = serde_json::from_value::<WsOutMessage>(json).expect("deserialize event");
        assert_eq!(parsed, event);
    }
}
