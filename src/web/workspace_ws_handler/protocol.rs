use super::*;

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
    if matches!(msg, WsInMessage::Hello { .. } | WsInMessage::Ping) {
        return true;
    }

    match stage {
        WorkspaceStage::PrepareContext => matches!(
            msg,
            WsInMessage::ContextNote { .. }
                | WsInMessage::StartGeneration { .. }
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
            | WsInMessage::Ping
    )
}

pub(crate) fn message_type(msg: &WsInMessage) -> &'static str {
    match msg {
        WsInMessage::UserMessage { .. } => "user_message",
        WsInMessage::ContextNote { .. } => "context_note",
        WsInMessage::StartGeneration { .. } => "start_generation",
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
        WsInMessage::HumanConfirm { .. } => "human_confirm",
        WsInMessage::RevertWorkItem { .. } => "revert_work_item",
        WsInMessage::Abort => "abort",
        WsInMessage::Ping => "ping",
    }
}
