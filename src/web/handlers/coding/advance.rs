use crate::product::advance_store::AdvanceOutcome;
use crate::web::workspace_ws_types::WsOutMessage;

/// Task 5.1 exposes only the rejection/replay mapping. Completion is wired by
/// the later durable group-initialization tasks.
#[allow(dead_code)]
pub(crate) fn map_advance_outcome(command_id: String, outcome: AdvanceOutcome) -> WsOutMessage {
    match outcome {
        AdvanceOutcome::Rejected { code, reason, .. } => WsOutMessage::AdvanceRejected {
            command_id,
            code,
            reason,
        },
        AdvanceOutcome::Replayed { record } => match (record.attempt_id, record.workspace_entry) {
            (Some(attempt_id), Some(workspace_entry)) => WsOutMessage::AdvanceCompleted {
                command_id,
                attempt_id,
                workspace_entry,
            },
            _ => WsOutMessage::AdvanceRejected {
                command_id,
                code: "ADVANCE_REPLAY_INCOMPLETE".to_string(),
                reason: format!(
                    "advance record {} has no completed workspace entry",
                    record.id
                ),
            },
        },
        AdvanceOutcome::Completed {
            attempt_id,
            workspace_entry,
            ..
        } => WsOutMessage::AdvanceCompleted {
            command_id,
            attempt_id,
            workspace_entry,
        },
    }
}
