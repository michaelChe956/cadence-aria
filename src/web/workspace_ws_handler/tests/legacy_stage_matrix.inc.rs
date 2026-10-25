// 退役留档（T5/REQ-RET-02）：`revision_path_messages_are_only_valid_in_review_decision` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`author_decision_is_only_valid_in_author_confirm` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`human_confirm_messages_are_only_valid_in_human_confirm` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[test]
fn plan_repair_ws_commands_are_only_valid_in_human_confirm() {
    let confirm = WsInMessage::ConfirmPlanAmendment {
        amendment_id: "plan_amendment_0001".to_string(),
    };
    let cancel = WsInMessage::CancelPlanAmendment {
        amendment_id: "plan_amendment_0001".to_string(),
        reason: Some("用户取消".to_string()),
    };

    assert_eq!(message_type(&confirm), "confirm_plan_amendment");
    assert_eq!(message_type(&cancel), "cancel_plan_amendment");
    assert!(is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::Legacy,
        &confirm,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::Legacy,
        &cancel,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(!is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::Legacy,
        &confirm,
        &WorkspaceStage::Running
    ));
    assert!(!is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::Legacy,
        &cancel,
        &WorkspaceStage::Completed
    ));
    assert!(requires_stage_validation(&cancel));
    assert_eq!(
        serde_json::from_value::<WsInMessage>(serde_json::to_value(&confirm).unwrap()).unwrap(),
        confirm
    );
    assert_eq!(
        serde_json::from_value::<WsInMessage>(serde_json::to_value(&cancel).unwrap()).unwrap(),
        cancel
    );
}

#[test]
fn completed_stage_rejects_business_messages() {
    assert!(!is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::Legacy,
        &WsInMessage::Abort,
        &WorkspaceStage::Completed
    ));
    assert!(!is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::Legacy,
        &WsInMessage::ContextNote {
            content: "late note".to_string()
        },
        &WorkspaceStage::Completed
    ));
}

#[test]
fn control_and_legacy_messages_do_not_require_stage_lock_validation() {
    // C3/REQ-HTR-01：Abort 收编 stage 矩阵——不再豁免（门开态必须得到可见
    // 拒收回执而非静默假成功）；矩阵对 Abort 的非阻塞 stage 读见 socket 层。
    assert!(requires_stage_validation(&WsInMessage::Abort));
    assert!(!requires_stage_validation(
        &WsInMessage::PermissionResponse {
            id: "permission-1".to_string(),
            approved: true,
            reason: None,
        }
    ));
    assert!(!requires_stage_validation(&WsInMessage::ChoiceResponse {
        id: "choice-1".to_string(),
        selected_option_ids: vec!["continue".to_string()],
        free_text: None,
        answers: vec![],
        command_id: None,
        expected_run_id: None,
    }));
    assert!(!requires_stage_validation(&WsInMessage::UserMessage {
        content: "legacy generation request".to_string(),
    }));
    assert!(!requires_stage_validation(&WsInMessage::Rollback {
        checkpoint_id: "cp_001".to_string(),
    }));
    assert!(!requires_stage_validation(&WsInMessage::Hello {
        session_id: "session-1".to_string(),
        last_seen_node_id: None,
        role: None,
        after_event_seq: None,
    }));
    assert!(!requires_stage_validation(&WsInMessage::Ping));
    assert!(requires_stage_validation(&WsInMessage::ContextNote {
        content: "new protocol action".to_string(),
    }));
}

// C3/REQ-HTR-01：Abort 收编 stage 矩阵后的放行/拒收集——run 族阶段（含
// PrepareContext/AuthorConfirm 既有放行）受理；人工门开态（HumanConfirm）与
// 终态（Completed）拒绝，门开态点击中止得到可见错误而非静默假成功。
#[test]
fn abort_is_governed_by_stage_matrix_after_termination_reliability_change() {
    for stage in [
        WorkspaceStage::PrepareContext,
        WorkspaceStage::Running,
        WorkspaceStage::AuthorConfirm,
        WorkspaceStage::CrossReview,
        WorkspaceStage::Revision,
    ] {
        assert!(
            is_message_valid_for_stage_with_flow(
                WorkItemPlanFlowKind::SingleCandidate,
                &WsInMessage::Abort,
                &stage
            ),
            "run 族阶段必须受理 Abort: {stage:?}"
        );
    }
    for (flow_kind, stage) in [
        (
            WorkItemPlanFlowKind::SingleCandidate,
            WorkspaceStage::HumanConfirm,
        ),
        (WorkItemPlanFlowKind::Legacy, WorkspaceStage::HumanConfirm),
        (
            WorkItemPlanFlowKind::SingleCandidate,
            WorkspaceStage::Completed,
        ),
        (WorkItemPlanFlowKind::Legacy, WorkspaceStage::Completed),
        (
            WorkItemPlanFlowKind::SingleCandidate,
            WorkspaceStage::ReviewDecision,
        ),
    ] {
        assert!(
            !is_message_valid_for_stage_with_flow(flow_kind, &WsInMessage::Abort, &stage),
            "门开/终态阶段必须拒绝 Abort: {stage:?} ({flow_kind:?})"
        );
    }
}

#[test]
fn choice_response_message_type_is_reported_for_protocol_errors() {
    assert_eq!(
        message_type(&WsInMessage::ChoiceResponse {
            id: "choice-1".to_string(),
            selected_option_ids: vec!["continue".to_string()],
            free_text: None,
            answers: vec![],
            command_id: None,
            expected_run_id: None,
        }),
        "choice_response"
    );
}

#[test]
fn missing_active_run_error_uses_protocol_error() {
    let error = missing_active_run_error("choice_response", "choice-1");

    match error {
        WsOutMessage::ProtocolError {
            code,
            message,
            context,
        } => {
            assert_eq!(code, "ACTIVE_RUN_NOT_FOUND");
            assert!(message.contains("choice_response"));
            assert_eq!(
                context
                    .as_ref()
                    .and_then(|value| value.get("id"))
                    .and_then(|value| value.as_str()),
                Some("choice-1")
            );
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}
