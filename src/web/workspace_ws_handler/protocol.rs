use std::collections::BTreeSet;

use super::*;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;

const CLIENT_SUBMITTED_SCOPE_FIELDS: [&str; 3] =
    ["scope", "review_invocation_scope", "review_scope"];

pub(crate) const RETIRED_INBOUND_MESSAGE_TYPES: [&str; 10] = [
    "review_decision_response",
    "author_decision",
    "select_work_item_generation_mode",
    "select_revision_path",
    "request_outline_revision",
    "work_item_draft_decision",
    "work_item_batch_decision",
    "save_human_presentation_revision",
    "human_confirm",
    "revert_work_item",
];

/// 已删除的 legacy 决策消息在 parse 面即被拒收（serde 未知变体）；本函数把
/// 退役 wire 名从通用 parse 错误中识别出来，供 socket 层返回 stage-specific
/// protocol error（REQ-WSC-08「已删除消息协议错误拒绝」/REQ-RET-03「在途
/// legacy 会话决策拒绝」——零副作用：识别仅读原始文本，不触碰引擎状态）。
pub(crate) fn retired_inbound_message_type(text: &str) -> Option<&'static str> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let name = value.get("type")?.as_str()?;
    RETIRED_INBOUND_MESSAGE_TYPES
        .iter()
        .find(|retired| **retired == name)
        .copied()
}

pub(crate) fn retired_message_protocol_error(
    stage: &WorkspaceStage,
    message_type: &str,
) -> WsOutMessage {
    WsOutMessage::ProtocolError {
        code: "LEGACY_MESSAGE_RETIRED".to_string(),
        message: format!(
            "message type {message_type} was retired with the legacy workitem decision protocol"
        ),
        context: Some(serde_json::json!({
            "stage": stage.as_str(),
            "received": message_type,
        })),
    }
}

pub(crate) fn single_candidate_scope_submission_error(
    flow_kind: WorkItemPlanFlowKind,
    submitted_fields: &BTreeSet<String>,
) -> Option<WsOutMessage> {
    if flow_kind != WorkItemPlanFlowKind::SingleCandidate {
        return None;
    }

    let forbidden_fields = submitted_fields
        .iter()
        .filter(|field| CLIENT_SUBMITTED_SCOPE_FIELDS.contains(&field.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if forbidden_fields.is_empty() {
        return None;
    }

    Some(WsOutMessage::ProtocolError {
        code: "SINGLE_CANDIDATE_SCOPE_FORBIDDEN".to_string(),
        message: "single-candidate review scope is server-controlled".to_string(),
        context: Some(serde_json::json!({ "submitted_fields": forbidden_fields })),
    })
}

pub(crate) fn missing_active_run_error(message_type: &'static str, id: &str) -> WsOutMessage {
    WsOutMessage::ProtocolError {
        code: "ACTIVE_RUN_NOT_FOUND".to_string(),
        message: format!("{message_type} id={id} has no active provider run"),
        context: Some(serde_json::json!({
            "message_type": message_type,
            "id": id,
        })),
    }
}

pub(crate) fn choice_id_unmatched_error(id: &str) -> WsOutMessage {
    WsOutMessage::ProtocolError {
        code: "CHOICE_ID_UNMATCHED".to_string(),
        message: format!("ChoiceResponse id={id} not found in pending"),
        context: Some(serde_json::json!({ "choice_id": id })),
    }
}

#[allow(clippy::result_large_err)]
pub(crate) fn validate_command_id(command_id: &str) -> Result<(), WsOutMessage> {
    if command_id.trim().is_empty() {
        return Err(WsOutMessage::ProtocolError {
            code: "INVALID_COMMAND_ID".to_string(),
            message: "command_id must not be blank".to_string(),
            context: None,
        });
    }

    Ok(())
}

pub(crate) fn is_message_valid_for_stage_with_flow(
    flow_kind: WorkItemPlanFlowKind,
    msg: &WsInMessage,
    stage: &WorkspaceStage,
) -> bool {
    if matches!(msg, WsInMessage::Hello { .. } | WsInMessage::Ping) {
        return true;
    }

    match stage {
        WorkspaceStage::PrepareContext => matches!(
            msg,
            WsInMessage::ContextNote { .. }
                | WsInMessage::StartGeneration { .. }
                | WsInMessage::RetryInterruptedRun { .. }
                | WsInMessage::Abort
                | WsInMessage::UserMessage { .. }
                | WsInMessage::ProviderSelect { .. }
                | WsInMessage::Rollback { .. }
        ),
        WorkspaceStage::Running => {
            matches!(
                msg,
                WsInMessage::Abort
                    | WsInMessage::PermissionResponse { .. }
                    | WsInMessage::ChoiceResponse { .. }
                    | WsInMessage::StartLinkedWorkspaceAmendment { .. }
            )
        }
        // L2 退役后 author_confirm 的 legacy 逐段/author 决策消息已删除
        //（REQ-RET-02）；RequestRevision 的 WorkItemPlan 放行特例在 socket
        // 层（author_confirm + WorkItemPlan），此处不重复。
        WorkspaceStage::AuthorConfirm => matches!(msg, WsInMessage::Abort),
        WorkspaceStage::CrossReview => {
            matches!(msg, WsInMessage::Abort | WsInMessage::ChoiceResponse { .. })
        }
        // review_decision 阶段的唯一消息族（决策应答与修订路径选择两类
        // 入站）已随 L2 删除：任何消息在该阶段均不合法（SC 会话
        // 已由 review/routing 的 SC 守卫改道 human gate，不落入此阶段）。
        WorkspaceStage::ReviewDecision => false,
        WorkspaceStage::Revision => {
            matches!(msg, WsInMessage::Abort | WsInMessage::ChoiceResponse { .. })
        }
        WorkspaceStage::HumanConfirm => {
            if flow_kind == WorkItemPlanFlowKind::SingleCandidate {
                matches!(
                    msg,
                    WsInMessage::HumanGateFeedback { .. }
                        | WsInMessage::Confirm
                        // L0 typed 重承载（REQ-RET-02/REQ-CG-04）：typed abandon
                        // 门命令与 HumanGateFeedback/Confirm 同族放行。
                        | WsInMessage::AbandonHumanGate { .. }
                )
            } else {
                matches!(
                    msg,
                    WsInMessage::ConfirmPlanAmendment { .. }
                        | WsInMessage::CancelPlanAmendment { .. }
                        | WsInMessage::StartLinkedWorkspaceAmendment { .. }
                        | WsInMessage::WorkItemPlanCompileRecoveryAction { .. }
                        // approve（confirm 帧）legacy 分支保留：story/design 等
                        // 非 SC 流的确认帧面（T4 §4 登记限制的既有形态）。
                        | WsInMessage::Confirm
                )
            }
        }
        WorkspaceStage::Completed => {
            flow_kind == WorkItemPlanFlowKind::SingleCandidate
                && matches!(msg, WsInMessage::Advance { .. })
        }
    }
}

pub(crate) fn human_gate_message_boundary_error(
    flow_kind: WorkItemPlanFlowKind,
    stage: WorkspaceStage,
    message: &WsInMessage,
) -> Option<WsOutMessage> {
    if flow_kind != WorkItemPlanFlowKind::SingleCandidate
        || matches!(message, WsInMessage::HumanGateFeedback { .. })
    {
        return None;
    }
    if is_message_valid_for_stage_with_flow(flow_kind, message, &stage) {
        return None;
    }

    // Legacy/control traffic keeps its existing routing. New conversational-gate commands and
    // ordinary user messages fail closed before any provider/store operation.
    if stage == WorkspaceStage::HumanConfirm || matches!(message, WsInMessage::UserMessage { .. }) {
        return Some(conversational_gate_stage_error(flow_kind, &stage, message));
    }

    None
}

pub(crate) fn conversational_gate_stage_error(
    flow_kind: WorkItemPlanFlowKind,
    stage: &WorkspaceStage,
    msg: &WsInMessage,
) -> WsOutMessage {
    WsOutMessage::ProtocolError {
        code: "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID".to_string(),
        message: format!(
            "message {} not allowed in stage {}",
            message_type(msg),
            stage.as_str()
        ),
        context: Some(serde_json::json!({
            "stage": stage.as_str(),
            "received": message_type(msg),
            "flow_kind": flow_kind,
        })),
    }
}

pub(crate) fn advance_stage_error(
    command_id: String,
    stage: &WorkspaceStage,
    flow_kind: WorkItemPlanFlowKind,
) -> WsOutMessage {
    let flow_kind = match flow_kind {
        WorkItemPlanFlowKind::Legacy => "legacy",
        WorkItemPlanFlowKind::SingleCandidate => "single_candidate",
    };
    WsOutMessage::AdvanceRejected {
        command_id,
        code: "ADVANCE_STAGE_INVALID".to_string(),
        reason: format!(
            "advance is not allowed for {flow_kind} flow in stage {}",
            stage.as_str()
        ),
    }
}

pub(crate) fn requires_stage_validation(msg: &WsInMessage) -> bool {
    !matches!(
        msg,
        WsInMessage::Abort
            | WsInMessage::PermissionResponse { .. }
            | WsInMessage::ChoiceResponse { .. }
            | WsInMessage::UserMessage { .. }
            | WsInMessage::Rollback { .. }
            | WsInMessage::Hello { .. }
            | WsInMessage::Ping
    )
}

pub(crate) fn message_type(msg: &WsInMessage) -> &'static str {
    match msg {
        WsInMessage::UserMessage { .. } => "user_message",
        WsInMessage::ContextNote { .. } => "context_note",
        WsInMessage::StartGeneration { .. } => "start_generation",
        WsInMessage::RetryInterruptedRun { .. } => "retry_interrupted_run",
        WsInMessage::Hello { .. } => "hello",
        WsInMessage::Rollback { .. } => "rollback",
        WsInMessage::Confirm => "confirm",
        WsInMessage::ProviderSelect { .. } => "provider_select",
        WsInMessage::PermissionResponse { .. } => "permission_response",
        WsInMessage::ChoiceResponse { .. } => "choice_response",
        WsInMessage::RequestRevision { .. } => "request_revision",
        WsInMessage::WorkItemPlanCompileRecoveryAction { .. } => {
            "work_item_plan_compile_recovery_action"
        }
        WsInMessage::ConfirmPlanAmendment { .. } => "confirm_plan_amendment",
        WsInMessage::CancelPlanAmendment { .. } => "cancel_plan_amendment",
        WsInMessage::StartLinkedWorkspaceAmendment { .. } => "start_linked_workspace_amendment",
        WsInMessage::HumanGateFeedback { .. } => "human_gate_feedback",
        WsInMessage::Advance { .. } => "advance",
        WsInMessage::AbandonHumanGate { .. } => "abandon_human_gate",
        WsInMessage::Abort => "abort",
        WsInMessage::Ping => "ping",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;

    #[test]
    fn retired_legacy_decision_wire_names_are_recognized_for_stage_specific_rejection() {
        for wire_name in RETIRED_INBOUND_MESSAGE_TYPES {
            let raw = format!(r#"{{"type":"{wire_name}","decision":"confirm"}}"#);
            assert_eq!(retired_inbound_message_type(&raw), Some(wire_name));
        }
        // 现役消息名不得误判为退役（parse 成功路径）。
        for live in [
            "confirm",
            "advance",
            "human_gate_feedback",
            "abandon_human_gate",
        ] {
            let raw = format!(r#"{{"type":"{live}","command_id":"cmd"}}"#);
            assert_eq!(retired_inbound_message_type(&raw), None, "{live}");
        }
        assert_eq!(retired_inbound_message_type("not json"), None);

        let error = retired_message_protocol_error(&WorkspaceStage::HumanConfirm, "human_confirm");
        let WsOutMessage::ProtocolError { code, context, .. } = error else {
            panic!("expected protocol error");
        };
        let context = context.expect("retired error context");
        assert_eq!(code, "LEGACY_MESSAGE_RETIRED");
        assert_eq!(context["stage"], "human_confirm");
        assert_eq!(context["received"], "human_confirm");
    }

    #[test]
    fn review_decision_stage_accepts_nothing_after_legacy_retirement() {
        let confirm = WsInMessage::Confirm;
        assert!(!is_message_valid_for_stage_with_flow(
            WorkItemPlanFlowKind::Legacy,
            &confirm,
            &WorkspaceStage::ReviewDecision,
        ));
        assert!(!is_message_valid_for_stage_with_flow(
            WorkItemPlanFlowKind::SingleCandidate,
            &confirm,
            &WorkspaceStage::ReviewDecision,
        ));
    }
}
