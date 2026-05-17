use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{ProviderCommand, ProviderStatus, RiskLevel};
use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use crate::web::state::WebAppState;
use crate::web::workspace_ws_types::{
    WsInMessage, WsOutMessage, WsPermissionRiskLevel, WsProviderStatus,
};

pub async fn workspace_ws(
    ws: WebSocketUpgrade,
    Path(session_id): Path<String>,
    State(state): State<WebAppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_workspace_socket(socket, session_id, state))
}

async fn handle_workspace_socket(socket: WebSocket, session_id: String, state: WebAppState) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    let app_paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let session_record = match lifecycle.get_workspace_session(&session_id) {
        Ok(session) => session,
        Err(error) => {
            let err = WsOutMessage::Error {
                message: format!("workspace session not found: {error}"),
            };
            if let Ok(json) = serde_json::to_string(&err) {
                let _ = ws_sender.send(Message::Text(json.into())).await;
            }
            return;
        }
    };

    let checkpoint_store = Arc::new(CheckpointStore::new(
        app_paths.issue_lifecycle_root(&session_record.project_id, &session_record.issue_id),
    ));

    let (engine_tx, mut engine_rx) = mpsc::channel::<EngineEvent>(64);

    let mut session = WorkspaceSession::from_record(session_record);
    if let Ok(checkpoints) = checkpoint_store.list_checkpoints(&session.session_id) {
        session.restore_checkpoint_ids(&checkpoints);
    }
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle,
        engine_tx,
        session,
    )));

    let session_state = engine.lock().await.build_session_state();
    if let Ok(json) = serde_json::to_string(&session_state) {
        let _ = ws_sender.send(Message::Text(json.into())).await;
    }

    let (outbound_tx, mut outbound_rx) = mpsc::channel::<String>(64);

    let send_task = tokio::spawn(async move {
        while let Some(msg) = outbound_rx.recv().await {
            if ws_sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    let outbound_for_events = outbound_tx.clone();
    let event_forward_task = tokio::spawn(async move {
        while let Some(event) = engine_rx.recv().await {
            let ws_msg = match event {
                EngineEvent::StreamChunk { role, content } => {
                    WsOutMessage::StreamChunk { role, content }
                }
                EngineEvent::MessageComplete {
                    message_id,
                    checkpoint_id,
                } => WsOutMessage::MessageComplete {
                    message_id,
                    checkpoint_id,
                },
                EngineEvent::StageChange { stage } => WsOutMessage::StageChange { stage },
                EngineEvent::ArtifactUpdate { version, markdown } => WsOutMessage::ArtifactUpdate {
                    version,
                    markdown,
                    diff: None,
                },
                EngineEvent::PermissionRequest {
                    id,
                    tool_name,
                    description,
                    risk_level,
                } => WsOutMessage::PermissionRequest {
                    id,
                    tool_name,
                    description,
                    risk_level: ws_permission_risk_level(risk_level),
                },
                EngineEvent::ProviderStatus { status } => WsOutMessage::ProviderStatus {
                    status: ws_provider_status(status),
                },
                EngineEvent::Error { message } => WsOutMessage::Error { message },
            };
            if let Ok(json) = serde_json::to_string(&ws_msg)
                && outbound_for_events.send(json).await.is_err()
            {
                break;
            }
        }
    });

    let current_run: Arc<Mutex<Option<ActiveRun>>> = Arc::new(Mutex::new(None));
    let next_run_id: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));

    while let Some(Ok(msg)) = ws_receiver.next().await {
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };

        let in_msg: WsInMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                let err = WsOutMessage::Error {
                    message: format!("invalid message: {e}"),
                };
                if let Ok(json) = serde_json::to_string(&err) {
                    let _ = outbound_tx.send(json).await;
                }
                continue;
            }
        };

        match in_msg {
            WsInMessage::UserMessage { content } => {
                let active = { current_run.lock().await.take() };
                if let Some(run) = active {
                    let _ = run.command_tx.send(ProviderCommand::Abort).await;
                    run.cancel.cancel();
                }
                let provider_name = {
                    let engine = engine.lock().await;
                    engine.session().author_provider.clone()
                };
                let Some(provider_for_run) = state.provider_registry.get(&provider_name) else {
                    let err = WsOutMessage::Error {
                        message: format!("provider unavailable: {provider_name:?}"),
                    };
                    if let Ok(json) = serde_json::to_string(&err) {
                        let _ = outbound_tx.send(json).await;
                    }
                    continue;
                };
                let engine_for_run = engine.clone();
                let current_run_for_task = current_run.clone();
                let run_id = {
                    let mut next = next_run_id.lock().await;
                    *next += 1;
                    *next
                };
                let run_cancel = CancellationToken::new();
                let (command_tx, command_rx) = mpsc::channel(8);
                *current_run.lock().await = Some(ActiveRun {
                    id: run_id,
                    cancel: run_cancel.clone(),
                    command_tx,
                });
                tokio::spawn(async move {
                    let mut engine = engine_for_run.lock().await;
                    engine.use_run_token(run_cancel);
                    engine
                        .handle_user_message(content, provider_for_run, command_rx)
                        .await;
                    let mut current = current_run_for_task.lock().await;
                    if current.as_ref().is_some_and(|active| active.id == run_id) {
                        *current = None;
                    }
                });
            }
            WsInMessage::Rollback { checkpoint_id } => {
                let active = { current_run.lock().await.take() };
                if let Some(run) = active {
                    let _ = run.command_tx.send(ProviderCommand::Abort).await;
                    run.cancel.cancel();
                }
                let mut engine = engine.lock().await;
                if let Err(e) = engine.handle_rollback(&checkpoint_id).await {
                    let err = WsOutMessage::Error { message: e };
                    if let Ok(json) = serde_json::to_string(&err) {
                        let _ = outbound_tx.send(json).await;
                    }
                } else {
                    let state_msg = engine.build_session_state();
                    if let Ok(json) = serde_json::to_string(&state_msg) {
                        let _ = outbound_tx.send(json).await;
                    }
                }
            }
            WsInMessage::Confirm => {
                let mut engine = engine.lock().await;
                engine.handle_confirm().await;
            }
            WsInMessage::ProviderSelect { role, provider } => {
                let mut engine = engine.lock().await;
                if let Err(e) = engine.set_provider(&role, provider) {
                    let err = WsOutMessage::Error { message: e };
                    if let Ok(json) = serde_json::to_string(&err) {
                        let _ = outbound_tx.send(json).await;
                    }
                } else {
                    let state_msg = engine.build_session_state();
                    if let Ok(json) = serde_json::to_string(&state_msg) {
                        let _ = outbound_tx.send(json).await;
                    }
                }
            }
            WsInMessage::PermissionResponse {
                id,
                approved,
                reason,
            } => {
                let command_tx = {
                    current_run
                        .lock()
                        .await
                        .as_ref()
                        .map(|run| run.command_tx.clone())
                };
                if let Some(command_tx) = command_tx {
                    let _ = command_tx
                        .send(ProviderCommand::PermissionResponse {
                            id,
                            approved,
                            reason,
                        })
                        .await;
                }
            }
            WsInMessage::Abort => {
                let active = { current_run.lock().await.take() };
                if let Some(run) = active {
                    let _ = run.command_tx.send(ProviderCommand::Abort).await;
                    run.cancel.cancel();
                }
            }
        }
    }

    let active = { current_run.lock().await.take() };
    if let Some(run) = active {
        let _ = run.command_tx.send(ProviderCommand::Abort).await;
        run.cancel.cancel();
    }
    drop(outbound_tx);
    event_forward_task.abort();
    send_task.abort();
    let _ = event_forward_task.await;
    let _ = send_task.await;
}

#[derive(Debug)]
struct ActiveRun {
    id: u64,
    cancel: CancellationToken,
    command_tx: mpsc::Sender<ProviderCommand>,
}

fn ws_permission_risk_level(risk_level: RiskLevel) -> WsPermissionRiskLevel {
    match risk_level {
        RiskLevel::Low => WsPermissionRiskLevel::Low,
        RiskLevel::Medium => WsPermissionRiskLevel::Medium,
        RiskLevel::High => WsPermissionRiskLevel::High,
    }
}

fn ws_provider_status(status: ProviderStatus) -> WsProviderStatus {
    match status {
        ProviderStatus::Starting => WsProviderStatus::Starting,
        ProviderStatus::Running => WsProviderStatus::Running,
        ProviderStatus::WaitingApproval => WsProviderStatus::WaitingApproval,
        ProviderStatus::Completed => WsProviderStatus::Completed,
        ProviderStatus::Failed => WsProviderStatus::Failed,
        ProviderStatus::Aborted => WsProviderStatus::Aborted,
    }
}
