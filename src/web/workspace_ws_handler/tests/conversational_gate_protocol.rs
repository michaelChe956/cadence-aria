use super::*;

#[test]
fn conversational_gate_inbound_roundtrips_and_preserves_command_ids() {
    let feedback_json = serde_json::json!({
        "type": "human_gate_feedback",
        "command_id": "cmd-001",
        "feedback": "保留其余内容，只修正这个字段"
    });
    let feedback: WsInMessage = serde_json::from_value(feedback_json).unwrap();
    assert_eq!(
        feedback,
        WsInMessage::HumanGateFeedback {
            command_id: "cmd-001".to_string(),
            feedback: "保留其余内容，只修正这个字段".to_string(),
        }
    );
    let feedback_wire = serde_json::to_value(&feedback).unwrap();
    assert_eq!(feedback_wire["type"], "human_gate_feedback");
    assert_eq!(feedback_wire["command_id"], "cmd-001");
    assert_eq!(feedback_wire["feedback"], "保留其余内容，只修正这个字段");
    assert!(feedback_wire.get("HumanGateFeedback").is_none());

    let advance_json = serde_json::json!({
        "type": "advance",
        "command_id": "cmd-002"
    });
    let advance: WsInMessage = serde_json::from_value(advance_json).unwrap();
    assert_eq!(
        advance,
        WsInMessage::Advance {
            command_id: "cmd-002".to_string(),
        }
    );
    let advance_wire = serde_json::to_value(&advance).unwrap();
    assert_eq!(advance_wire["type"], "advance");
    assert_eq!(advance_wire["command_id"], "cmd-002");
    assert!(advance_wire.get("Advance").is_none());

    assert_eq!(message_type(&feedback), "human_gate_feedback");
    assert_eq!(message_type(&advance), "advance");
}

#[test]
fn conversational_gate_rejects_blank_command_id_at_handler_boundary() {
    for command_id in ["", "   ", "\t\n"] {
        let error = validate_command_id(command_id).expect_err("blank command ID must fail");
        let WsOutMessage::ProtocolError {
            code,
            message,
            context,
        } = error
        else {
            panic!("blank command ID must produce protocol error");
        };
        assert_eq!(code, "INVALID_COMMAND_ID");
        assert!(!message.is_empty());
        assert_eq!(context, None);
    }

    assert!(validate_command_id("cmd-001").is_ok());
}

#[tokio::test]
async fn conversational_gate_blank_command_id_is_rejected_through_dispatch() {
    let (context, _engine, mut outbound_rx, _events) =
        super::single_candidate_scope_rejection::scope_test_context(
            crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
        );

    handle_workspace_inbound_message(
        context,
        WsInMessage::HumanGateFeedback {
            command_id: "   ".to_string(),
            feedback: "should not be dispatched".to_string(),
        },
    )
    .await;

    let outbound = outbound_rx.recv().await.expect("protocol error outbound");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected text protocol error");
    };
    let error: WsOutMessage = serde_json::from_str(&json).expect("protocol error json");
    assert!(matches!(
        error,
        WsOutMessage::ProtocolError { code, .. } if code == "INVALID_COMMAND_ID"
    ));
}
