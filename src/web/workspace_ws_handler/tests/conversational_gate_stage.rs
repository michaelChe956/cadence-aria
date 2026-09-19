use super::*;

#[test]
fn conversational_gate_stage_matrix_accepts_only_typed_gate_commands_and_advance() {
    let feedback = WsInMessage::HumanGateFeedback {
        command_id: "cmd-feedback".to_string(),
        feedback: "请修订候选".to_string(),
    };
    let confirm = WsInMessage::Confirm;
    let abandon = WsInMessage::AbandonHumanGate {
        command_id: "cmd-abandon".to_string(),
    };
    let advance = WsInMessage::Advance {
        command_id: "cmd-advance".to_string(),
    };

    // L2 重钉（T5/REQ-RET-02）：SC 门三命令面（HumanConfirm 双轨臂已删）。
    for message in [&feedback, &confirm, &abandon] {
        assert!(is_message_valid_for_stage_with_flow(
            WorkItemPlanFlowKind::SingleCandidate,
            message,
            &WorkspaceStage::HumanConfirm,
        ));
    }
    assert!(is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::SingleCandidate,
        &advance,
        &WorkspaceStage::Completed,
    ));
    // 非 SC 流的 Confirm 帧保留（approve 面）；typed abandon 对非 SC 流不放行。
    assert!(is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::Legacy,
        &confirm,
        &WorkspaceStage::HumanConfirm,
    ));
    assert!(!is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::Legacy,
        &abandon,
        &WorkspaceStage::HumanConfirm,
    ));

    for stage in [
        WorkspaceStage::Running,
        WorkspaceStage::CrossReview,
        WorkspaceStage::Revision,
        WorkspaceStage::Completed,
    ] {
        assert!(!is_message_valid_for_stage_with_flow(
            WorkItemPlanFlowKind::SingleCandidate,
            &feedback,
            &stage,
        ));
    }
    assert!(!is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::SingleCandidate,
        &advance,
        &WorkspaceStage::HumanConfirm,
    ));
    assert!(!is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::Legacy,
        &advance,
        &WorkspaceStage::Completed,
    ));
}

#[test]
fn conversational_gate_unknown_stage_message_is_zero_side_effect_protocol_error() {
    let feedback = WsInMessage::HumanGateFeedback {
        command_id: "cmd-feedback".to_string(),
        feedback: "请修订候选".to_string(),
    };
    let error = conversational_gate_stage_error(
        WorkItemPlanFlowKind::SingleCandidate,
        &WorkspaceStage::Running,
        &feedback,
    );
    let WsOutMessage::ProtocolError {
        code,
        message,
        context,
    } = error
    else {
        panic!("gate stage rejection must be a protocol error");
    };
    assert_eq!(code, "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID");
    assert!(message.contains("human_gate_feedback"));
    let context = context.expect("stage rejection context");
    assert_eq!(context["stage"], "running");
    assert_eq!(context["received"], "human_gate_feedback");
    assert_eq!(context["flow_kind"], "single_candidate");
}

#[test]
fn conversational_gate_socket_path_uses_flow_aware_admission() {
    let source = include_str!("../socket.rs");
    assert!(source.contains(
        "&& !is_message_valid_for_stage_with_flow(session_record.flow_kind, in_msg, stage)"
    ));
    assert!(!source.contains("if session_record.flow_kind == WorkItemPlanFlowKind::Legacy"));
}
