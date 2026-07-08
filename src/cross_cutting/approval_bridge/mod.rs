use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ChoiceAnswerData, ChoiceRequestData, PermissionRequestData, ProviderCommand, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, RiskLevel,
};

mod cleanup;
mod commands;
mod guards;
#[cfg(test)]
mod tests;

use cleanup::cleanup_pending_permissions;
use commands::listen_for_permission_commands;
use guards::{PendingChoiceGuard, PendingPermissionGuard};

static NEXT_PERMISSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDecision {
    pub approved: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceDecision {
    pub selected_option_ids: Vec<String>,
    pub free_text: Option<String>,
    pub answers: Vec<ChoiceAnswerData>,
}

type PendingPermissions =
    Arc<Mutex<HashMap<String, (oneshot::Sender<PermissionDecision>, Instant)>>>;
type PendingChoices = Arc<Mutex<HashMap<String, oneshot::Sender<ChoiceDecision>>>>;

pub struct ApprovalBridge {
    mode: ProviderPermissionMode,
    event_tx: mpsc::Sender<ProviderEvent>,
    command_tx: mpsc::Sender<ProviderCommand>,
    pending: PendingPermissions,
    pending_choices: PendingChoices,
    cleanup_cancel: CancellationToken,
}

impl ApprovalBridge {
    pub fn new(mode: ProviderPermissionMode, event_tx: mpsc::Sender<ProviderEvent>) -> Self {
        let (command_tx, command_rx) = mpsc::channel(8);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let pending_choices: PendingChoices = Arc::new(Mutex::new(HashMap::new()));
        let cleanup_cancel = CancellationToken::new();

        tokio::spawn(listen_for_permission_commands(
            command_rx,
            Arc::clone(&pending),
            Arc::clone(&pending_choices),
            event_tx.clone(),
        ));
        tokio::spawn(cleanup_pending_permissions(
            Arc::clone(&pending),
            event_tx.clone(),
            cleanup_cancel.clone(),
        ));

        Self {
            mode,
            event_tx,
            command_tx,
            pending,
            pending_choices,
            cleanup_cancel,
        }
    }

    pub fn command_sender(&self) -> mpsc::Sender<ProviderCommand> {
        self.command_tx.clone()
    }

    pub async fn request_tool(
        &self,
        tool_name: &str,
        description: &str,
        risk_level: RiskLevel,
        cancel: CancellationToken,
    ) -> Result<PermissionDecision, ProviderAdapterError> {
        if self.mode == ProviderPermissionMode::Auto {
            let event_id = next_permission_id();
            let _ = self
                .event_tx
                .send(ProviderEvent::Execution(ProviderExecutionEvent {
                    event_id,
                    kind: ProviderExecutionEventKind::Provider,
                    status: ProviderExecutionEventStatus::Completed,
                    title: "Auto approval".to_string(),
                    detail: Some(format!("{tool_name}: {description}")),
                    command: None,
                    cwd: None,
                    output: Some(
                        serde_json::json!({
                            "auto_approved": true,
                            "tool_name": tool_name,
                            "description": description,
                            "risk_level": format!("{risk_level:?}"),
                        })
                        .to_string(),
                    ),
                    exit_code: None,
                }))
                .await;
            return Ok(PermissionDecision {
                approved: true,
                reason: Some("auto_approved".to_string()),
            });
        }

        let id = next_permission_id();
        let (decision_tx, decision_rx) = oneshot::channel();
        self.pending
            .lock()
            .await
            .insert(id.clone(), (decision_tx, Instant::now()));
        let mut pending_guard = PendingPermissionGuard::new(id.clone(), Arc::clone(&self.pending));

        let request = ProviderEvent::PermissionRequest(PermissionRequestData {
            id: id.clone(),
            tool_name: tool_name.to_string(),
            description: description.to_string(),
            risk_level,
        });

        let send_result = tokio::select! {
            _ = cancel.cancelled() => {
                pending_guard.remove_now().await;
                return Err(permission_bridge_error("permission request cancelled"));
            }
            result = self.event_tx.send(request) => result,
        };

        if send_result.is_err() {
            pending_guard.remove_now().await;
            return Err(permission_bridge_error(
                "permission request event receiver closed",
            ));
        }

        tokio::select! {
            _ = cancel.cancelled() => {
                pending_guard.remove_now().await;
                Err(permission_bridge_error("permission request cancelled"))
            }
            _ = self.event_tx.closed() => {
                pending_guard.remove_now().await;
                Err(permission_bridge_error("permission request event receiver closed"))
            }
            decision = decision_rx => {
                pending_guard.remove_now().await;
                decision.map_err(|_| permission_bridge_error("permission response channel closed"))
            }
        }
    }

    pub async fn request_choice(
        &self,
        request: ChoiceRequestData,
        cancel: CancellationToken,
    ) -> Result<ChoiceDecision, ProviderAdapterError> {
        let id = request.id.clone();
        eprintln!(
            "[aria-choice-diag] bridge emitting choice_request id={} source={} options={}",
            id,
            request.source.as_str(),
            request.options.len()
        );
        let (decision_tx, decision_rx) = oneshot::channel();
        self.pending_choices
            .lock()
            .await
            .insert(id.clone(), decision_tx);
        let request_id = id.clone();
        let mut pending_guard = PendingChoiceGuard::new(id, Arc::clone(&self.pending_choices));

        let send_result = tokio::select! {
            _ = cancel.cancelled() => {
                pending_guard.remove_now().await;
                return Err(permission_bridge_error("choice request cancelled"));
            }
            result = self.event_tx.send(ProviderEvent::ChoiceRequest(request)) => result,
        };

        if send_result.is_err() {
            pending_guard.remove_now().await;
            return Err(permission_bridge_error(
                "choice request event receiver closed",
            ));
        }

        tokio::select! {
            _ = cancel.cancelled() => {
                pending_guard.remove_now().await;
                Err(permission_bridge_error("choice request cancelled"))
            }
            _ = self.event_tx.closed() => {
                pending_guard.remove_now().await;
                Err(permission_bridge_error("choice request event receiver closed"))
            }
            decision = decision_rx => {
                pending_guard.remove_now().await;
                let decision = decision.map_err(|_| permission_bridge_error("choice response channel closed"))?;
                eprintln!(
                    "[aria-choice-diag] bridge resolved choice_request id={} selected={:?} free_text_present={}",
                    request_id,
                    decision.selected_option_ids,
                    decision.free_text.as_ref().is_some_and(|text| !text.trim().is_empty())
                );
                Ok(decision)
            }
        }
    }
}

impl Drop for ApprovalBridge {
    fn drop(&mut self) {
        self.cleanup_cancel.cancel();
    }
}

fn next_permission_id() -> String {
    let id = NEXT_PERMISSION_ID.fetch_add(1, Ordering::Relaxed);
    format!("permission_{id}")
}

fn permission_bridge_error(message: &'static str) -> ProviderAdapterError {
    ProviderAdapterError::permission_denied(message, String::new(), String::new())
}
