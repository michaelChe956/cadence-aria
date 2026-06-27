use tokio::sync::mpsc;

use crate::cross_cutting::streaming_provider::{ProviderCommand, ProviderEvent};

use super::{ChoiceDecision, PendingChoices, PendingPermissions, PermissionDecision};

pub(super) async fn listen_for_permission_commands(
    mut command_rx: mpsc::Receiver<ProviderCommand>,
    pending: PendingPermissions,
    pending_choices: PendingChoices,
    event_tx: mpsc::Sender<ProviderEvent>,
) {
    while let Some(command) = command_rx.recv().await {
        match command {
            ProviderCommand::PermissionResponse {
                id,
                approved,
                reason,
            } => {
                tracing::info!(permission_id = %id, approved, "bridge received permission response");
                if let Some((decision_tx, _created_at)) = pending.lock().await.remove(&id) {
                    tracing::info!(permission_id = %id, "bridge dispatched decision to pending");
                    let _ = decision_tx.send(PermissionDecision { approved, reason });
                } else {
                    tracing::warn!(permission_id = %id, "bridge: no pending entry for id");
                    let _ = event_tx
                        .send(ProviderEvent::ProtocolError {
                            code: "PERMISSION_ID_UNMATCHED".to_string(),
                            message: format!("PermissionResponse id={id} not found in pending"),
                            context: Some(serde_json::json!({ "permission_id": id })),
                        })
                        .await;
                }
            }
            ProviderCommand::ChoiceResponse {
                id,
                selected_option_ids,
                free_text,
            } => {
                tracing::info!(choice_id = %id, "bridge received choice response");
                eprintln!(
                    "[aria-choice-diag] bridge received choice_response id={} selected={:?} free_text_present={}",
                    id,
                    selected_option_ids,
                    free_text
                        .as_ref()
                        .is_some_and(|text| !text.trim().is_empty())
                );
                if let Some(decision_tx) = pending_choices.lock().await.remove(&id) {
                    eprintln!(
                        "[aria-choice-diag] bridge matched pending choice_response id={}",
                        id
                    );
                    let _ = decision_tx.send(ChoiceDecision {
                        selected_option_ids,
                        free_text,
                    });
                } else {
                    tracing::warn!(choice_id = %id, "bridge: no pending choice entry for id");
                    eprintln!(
                        "[aria-choice-diag] bridge missing pending choice_response id={}",
                        id
                    );
                    let _ = event_tx
                        .send(ProviderEvent::ProtocolError {
                            code: "CHOICE_ID_UNMATCHED".to_string(),
                            message: format!("ChoiceResponse id={id} not found in pending"),
                            context: Some(serde_json::json!({ "choice_id": id })),
                        })
                        .await;
                }
            }
            ProviderCommand::Abort => {
                let mut pending_permissions = pending.lock().await;
                for (_, (decision_tx, _created_at)) in pending_permissions.drain() {
                    let _ = decision_tx.send(PermissionDecision {
                        approved: false,
                        reason: Some("aborted".to_string()),
                    });
                }
                let mut pending_choices = pending_choices.lock().await;
                for (_, decision_tx) in pending_choices.drain() {
                    let _ = decision_tx.send(ChoiceDecision {
                        selected_option_ids: Vec::new(),
                        free_text: Some("aborted".to_string()),
                    });
                }
            }
            ProviderCommand::ToolResult(_) => {}
        }
    }
}
