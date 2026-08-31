use std::collections::BTreeSet;

use super::*;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;

const CLIENT_SUBMITTED_SCOPE_FIELDS: [&str; 3] =
    ["scope", "review_invocation_scope", "review_scope"];

pub(crate) fn single_candidate_generation_decision_error(
    flow_kind: WorkItemPlanFlowKind,
    message: &WsInMessage,
) -> Option<WsOutMessage> {
    if flow_kind != WorkItemPlanFlowKind::SingleCandidate
        || !matches!(
            message,
            WsInMessage::SelectWorkItemGenerationMode { .. }
                | WsInMessage::WorkItemDraftDecision { .. }
                | WsInMessage::WorkItemBatchDecision { .. }
        )
    {
        return None;
    }

    Some(WsOutMessage::ProtocolError {
        code: "SINGLE_CANDIDATE_GENERATION_DECISION_FORBIDDEN".to_string(),
        message: "single-candidate generation mode is selected internally".to_string(),
        context: Some(serde_json::json!({ "message_type": message_type(message) })),
    })
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

#[allow(dead_code)]
pub(crate) fn is_message_valid_for_stage(msg: &WsInMessage, stage: &WorkspaceStage) -> bool {
    is_message_valid_for_stage_with_flow(WorkItemPlanFlowKind::Legacy, msg, stage)
}

pub(crate) fn is_message_valid_for_stage_with_flow(
    flow_kind: WorkItemPlanFlowKind,
    msg: &WsInMessage,
    stage: &WorkspaceStage,
) -> bool {
    if matches!(
        msg,
        WsInMessage::Hello { .. }
            | WsInMessage::Ping
            | WsInMessage::SaveHumanPresentationRevision { .. }
    ) {
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
        WorkspaceStage::AuthorConfirm => {
            matches!(
                msg,
                WsInMessage::AuthorDecision { .. }
                    | WsInMessage::SelectWorkItemGenerationMode { .. }
                    | WsInMessage::RequestOutlineRevision { .. }
                    | WsInMessage::WorkItemDraftDecision { .. }
                    | WsInMessage::WorkItemBatchDecision { .. }
                    | WsInMessage::RevertWorkItem { .. }
                    | WsInMessage::Abort
            )
        }
        WorkspaceStage::CrossReview => {
            matches!(msg, WsInMessage::Abort | WsInMessage::ChoiceResponse { .. })
        }
        WorkspaceStage::ReviewDecision => matches!(
            msg,
            WsInMessage::SelectRevisionPath { .. } | WsInMessage::ReviewDecisionResponse { .. }
        ),
        WorkspaceStage::Revision => {
            matches!(msg, WsInMessage::Abort | WsInMessage::ChoiceResponse { .. })
        }
        WorkspaceStage::HumanConfirm => {
            if flow_kind == WorkItemPlanFlowKind::SingleCandidate {
                matches!(
                    msg,
                    WsInMessage::HumanGateFeedback { .. }
                        | WsInMessage::Confirm
                        | WsInMessage::HumanConfirm {
                            decision:
                                crate::web::workspace_ws_types::HumanConfirmDecision::Terminate,
                            ..
                        }
                )
            } else {
                matches!(
                    msg,
                    WsInMessage::HumanConfirm { .. }
                        | WsInMessage::ConfirmPlanAmendment { .. }
                        | WsInMessage::CancelPlanAmendment { .. }
                        | WsInMessage::StartLinkedWorkspaceAmendment { .. }
                        | WsInMessage::WorkItemPlanCompileRecoveryAction { .. }
                        | WsInMessage::RequestRevision { .. }
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
            | WsInMessage::SaveHumanPresentationRevision { .. }
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
        WsInMessage::ReviewDecisionResponse { .. } => "review_decision_response",
        WsInMessage::AuthorDecision { .. } => "author_decision",
        WsInMessage::SelectWorkItemGenerationMode { .. } => "select_work_item_generation_mode",
        WsInMessage::SelectRevisionPath { .. } => "select_revision_path",
        WsInMessage::RequestRevision { .. } => "request_revision",
        WsInMessage::RequestOutlineRevision { .. } => "request_outline_revision",
        WsInMessage::WorkItemDraftDecision { .. } => "work_item_draft_decision",
        WsInMessage::WorkItemBatchDecision { .. } => "work_item_batch_decision",
        WsInMessage::WorkItemPlanCompileRecoveryAction { .. } => {
            "work_item_plan_compile_recovery_action"
        }
        WsInMessage::SaveHumanPresentationRevision { .. } => "save_human_presentation_revision",
        WsInMessage::HumanConfirm { .. } => "human_confirm",
        WsInMessage::ConfirmPlanAmendment { .. } => "confirm_plan_amendment",
        WsInMessage::CancelPlanAmendment { .. } => "cancel_plan_amendment",
        WsInMessage::StartLinkedWorkspaceAmendment { .. } => "start_linked_workspace_amendment",
        WsInMessage::RevertWorkItem { .. } => "revert_work_item",
        WsInMessage::HumanGateFeedback { .. } => "human_gate_feedback",
        WsInMessage::Advance { .. } => "advance",
        WsInMessage::Abort => "abort",
        WsInMessage::Ping => "ping",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
    use crate::web::workspace_ws_types::{
        WorkItemBatchDecisionDto, WorkItemDraftDecisionDto, WorkItemGenerationModeDto,
    };

    #[test]
    fn single_candidate_generation_decision_messages_are_forbidden_but_legacy_remains_compatible() {
        let messages = [
            WsInMessage::SelectWorkItemGenerationMode {
                mode: WorkItemGenerationModeDto::Serial,
            },
            WsInMessage::WorkItemDraftDecision {
                outline_id: "outline_client_supplied".to_string(),
                decision: WorkItemDraftDecisionDto::Accept,
                feedback: None,
            },
            WsInMessage::WorkItemBatchDecision {
                decision: WorkItemBatchDecisionDto::AcceptAll,
                feedback: None,
                first_affected_outline_id: None,
            },
        ];

        for message in messages {
            let error = single_candidate_generation_decision_error(
                WorkItemPlanFlowKind::SingleCandidate,
                &message,
            )
            .expect("single-candidate must reject client generation decisions");
            let WsOutMessage::ProtocolError { code, .. } = error else {
                panic!("expected protocol error");
            };
            assert_eq!(code, "SINGLE_CANDIDATE_GENERATION_DECISION_FORBIDDEN");
            assert!(
                single_candidate_generation_decision_error(WorkItemPlanFlowKind::Legacy, &message)
                    .is_none(),
                "legacy generation decision protocol must remain compatible"
            );
        }
    }
}
