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
            // REQ-MTG-03（k3 F3 必改点二）：多 target record（target_attempts 非空）
            // 以「集合非空 + workspace_entry 在场」为完备判据——不再因单值
            // attempt_id 误落 ADVANCE_REPLAY_INCOMPLETE；单 target 保持
            // (attempt_id, workspace_entry) 双在场判据零变化。
            let complete = if record.target_attempts.is_empty() {
                record.attempt_id.is_some() && record.workspace_entry.is_some()
            } else {
                record.workspace_entry.is_some()
            };
            if complete {
                WsOutMessage::AdvanceCompleted {
                    command_id,
                    attempt_id: record.attempt_id.clone().unwrap_or_default(),
                    workspace_entry: record.workspace_entry.clone().unwrap_or_default(),
                    target_attempts: record.target_attempts.clone(),
                }
            } else {
                WsOutMessage::AdvanceRejected {
                    command_id,
                    code: "ADVANCE_REPLAY_INCOMPLETE".to_string(),
                    reason: format!(
                        "advance record {} has no completed workspace entry",
                        record.id
                    ),
                }
            }
        }
        AdvanceOutcome::Completed {
            attempt_id,
            workspace_entry,
            target_attempts,
            ..
        } => WsOutMessage::AdvanceCompleted {
            command_id,
            attempt_id,
            workspace_entry,
            target_attempts,
        },
}
}
