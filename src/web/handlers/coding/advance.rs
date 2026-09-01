use crate::product::advance_store::AdvanceOutcome;
use crate::web::workspace_ws_types::WsOutMessage;

/// Map a durable advance outcome to its websocket completion/rejection event.
pub(crate) fn map_advance_outcome(command_id: String, outcome: AdvanceOutcome) -> WsOutMessage {
    match outcome {
        AdvanceOutcome::Rejected { code, reason, .. } => WsOutMessage::AdvanceRejected {
            command_id,
            code,
            reason,
        },
        AdvanceOutcome::Replayed { record } => {
            if record.status != crate::product::advance_store::AdvanceStatus::Ready {
                return WsOutMessage::AdvanceRejected {
                    command_id,
                    code: "ADVANCE_REPLAY_NOT_READY".to_string(),
                    reason: format!(
                        "advance record {} is in durable status {:?}",
                        record.id, record.status
                    ),
                };
            }
            match (record.attempt_id, record.workspace_entry) {
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
            }
        }
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
