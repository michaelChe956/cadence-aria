use cadence_aria::daemon::state_machine::DaemonState;
use cadence_aria::protocol::repl_wire::{
    ApproveGateRequest, Command, RejectGateRequest, ReplyGateRequest, RequestEnvelope,
};
use tempfile::tempdir;

#[test]
fn approval_gate_commands_emit_gate_resolved_events() {
    let workspace = tempdir().expect("temp workspace");
    let mut state = DaemonState::bootstrap(workspace.path()).expect("bootstrap daemon state");

    for (request_id, command, payload) in [
        (
            "req_approve",
            Command::ApproveGate,
            serde_json::to_value(ApproveGateRequest {
                gate_id: "gate_001".to_string(),
            })
            .expect("approve payload"),
        ),
        (
            "req_reject",
            Command::RejectGate,
            serde_json::to_value(RejectGateRequest {
                gate_id: "gate_002".to_string(),
                reason: Some("scope rejected".to_string()),
            })
            .expect("reject payload"),
        ),
        (
            "req_reply",
            Command::ReplyGate,
            serde_json::to_value(ReplyGateRequest {
                gate_id: "gate_003".to_string(),
                reply_text: "补充信息".to_string(),
            })
            .expect("reply payload"),
        ),
    ] {
        let response = state
            .handle_request(RequestEnvelope {
                protocol_version: cadence_aria::protocol::repl_wire::PROTOCOL_VERSION.to_string(),
                message_type: cadence_aria::protocol::repl_wire::MessageType::Request,
                request_id: request_id.to_string(),
                command,
                sent_at: "2026-04-26T00:00:00Z".to_string(),
                payload,
            })
            .expect("gate response");
        assert!(response.ok);
    }

    let resolved_events: Vec<_> = state
        .events()
        .iter()
        .filter(|event| event.event_type == "gate.resolved")
        .collect();
    assert_eq!(resolved_events.len(), 3);
    assert_eq!(resolved_events[0].payload["gate_id"], "gate_001");
    assert_eq!(resolved_events[0].payload["resolution"], "approved");
    assert_eq!(resolved_events[1].payload["resolution"], "rejected");
    assert_eq!(resolved_events[2].payload["resolution"], "reply_recorded");
}
