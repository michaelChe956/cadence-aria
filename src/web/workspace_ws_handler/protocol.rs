use std::collections::BTreeSet;

use super::*;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;

const CLIENT_SUBMITTED_SCOPE_FIELDS: [&str; 3] =
    ["scope", "review_invocation_scope", "review_scope"];

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

pub(crate) fn is_message_valid_for_stage(msg: &WsInMessage, stage: &WorkspaceStage) -> bool {
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
        WorkspaceStage::HumanConfirm => matches!(
            msg,
            WsInMessage::HumanConfirm { .. }
                | WsInMessage::ConfirmPlanAmendment { .. }
                | WsInMessage::CancelPlanAmendment { .. }
                | WsInMessage::StartLinkedWorkspaceAmendment { .. }
                | WsInMessage::WorkItemPlanCompileRecoveryAction { .. }
                | WsInMessage::RequestRevision { .. }
                | WsInMessage::Confirm
        ),
        WorkspaceStage::Completed => false,
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
        WsInMessage::Abort => "abort",
        WsInMessage::Ping => "ping",
    }
}
