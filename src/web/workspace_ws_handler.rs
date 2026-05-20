use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderExecutionEvent, ProviderExecutionEventKind,
    ProviderExecutionEventStatus, ProviderStatus, RiskLevel,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::ProviderName;
use crate::product::workspace_engine::{
    EngineEvent, ReviewDecisionOutcome, WorkspaceEngine, WorkspaceSession, WorkspaceStage,
};
use crate::product::workspace_repository::workspace_repository_for_session;
use crate::web::state::WebAppState;
use crate::web::workspace_context::ensure_workspace_context_message;
use crate::web::workspace_ws_types::{
    HumanConfirmDecision, RevisionPath, WsExecutionEvent, WsExecutionEventKind,
    WsExecutionEventStatus, WsInMessage, WsOutMessage, WsPermissionRiskLevel, WsProviderStatus,
};

pub async fn workspace_ws(
    ws: WebSocketUpgrade,
    Path(session_id): Path<String>,
    State(state): State<WebAppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_workspace_socket(socket, session_id, state))
}

#[derive(Debug)]
enum OutboundControl {
    Text(String),
    CloseDueToIdleTimeout,
}

async fn send_json_outbound<T: serde::Serialize>(
    outbound_tx: &mpsc::Sender<OutboundControl>,
    message: &T,
) -> bool {
    match serde_json::to_string(message) {
        Ok(json) => outbound_tx.send(OutboundControl::Text(json)).await.is_ok(),
        Err(_) => false,
    }
}

fn spawn_idle_timeout_task(
    last_client_message_at: Arc<Mutex<tokio::time::Instant>>,
    outbound_tx: mpsc::Sender<OutboundControl>,
    timeout_after: std::time::Duration,
    tick_every: std::time::Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick_every);
        loop {
            interval.tick().await;
            let last_seen = *last_client_message_at.lock().await;
            if last_seen.elapsed() > timeout_after {
                let _ = outbound_tx
                    .send(OutboundControl::CloseDueToIdleTimeout)
                    .await;
                break;
            }
        }
    })
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
    let session_record =
        match ensure_workspace_context_message(&app_paths, &lifecycle, session_record) {
            Ok(session) => session,
            Err(error) => {
                let err = WsOutMessage::Error {
                    message: format!("workspace context unavailable: {error}"),
                };
                if let Ok(json) = serde_json::to_string(&err) {
                    let _ = ws_sender.send(Message::Text(json.into())).await;
                }
                return;
            }
        };

    let repository = match workspace_repository_for_session(&app_paths, &lifecycle, &session_record)
    {
        Ok(repository) => repository,
        Err(error) => {
            let err = WsOutMessage::Error {
                message: format!("workspace repository unavailable: {error}"),
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
    session.repository_path = Some(repository.path);
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

    let (outbound_tx, mut outbound_rx) = mpsc::channel::<OutboundControl>(64);

    let send_task = tokio::spawn(async move {
        while let Some(control) = outbound_rx.recv().await {
            match control {
                OutboundControl::Text(msg) => {
                    if ws_sender.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                OutboundControl::CloseDueToIdleTimeout => {
                    let _ = ws_sender.close().await;
                    break;
                }
            }
        }
    });

    let outbound_for_events = outbound_tx.clone();
    let event_forward_task = tokio::spawn(async move {
        while let Some(event) = engine_rx.recv().await {
            let ws_msg = match event {
                EngineEvent::StreamChunk {
                    role,
                    content,
                    node_id,
                } => WsOutMessage::StreamChunk {
                    role,
                    content,
                    node_id,
                },
                EngineEvent::MessageComplete {
                    message_id,
                    checkpoint_id,
                    node_id,
                } => WsOutMessage::MessageComplete {
                    message_id,
                    checkpoint_id,
                    node_id,
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
                EngineEvent::ExecutionEvent {
                    event,
                    node_id,
                    agent,
                } => WsOutMessage::ExecutionEvent {
                    event: ws_execution_event(event, node_id, agent),
                },
                EngineEvent::TimelineNodeCreated { node } => {
                    WsOutMessage::TimelineNodeCreated { node }
                }
                EngineEvent::TimelineNodeUpdated {
                    node_id,
                    status,
                    summary,
                    completed_at,
                } => WsOutMessage::TimelineNodeUpdated {
                    node_id,
                    status,
                    summary,
                    completed_at,
                },
                EngineEvent::ReviewComplete {
                    node_id,
                    round,
                    verdict,
                    comments,
                    summary,
                } => WsOutMessage::ReviewComplete {
                    node_id,
                    round,
                    verdict,
                    comments,
                    summary,
                },
                EngineEvent::ReviewDecisionRequired {
                    node_id,
                    round,
                    options,
                } => WsOutMessage::ReviewDecisionRequired {
                    node_id,
                    round,
                    options,
                },
                EngineEvent::Error { message } => WsOutMessage::Error { message },
                EngineEvent::ProtocolError {
                    code,
                    message,
                    context,
                } => WsOutMessage::ProtocolError {
                    code,
                    message,
                    context,
                },
                EngineEvent::PermissionTimeout {
                    permission_id,
                    node_id,
                } => WsOutMessage::ProtocolError {
                    code: "PERMISSION_TIMEOUT".to_string(),
                    message: format!("Permission request {permission_id} timed out"),
                    context: Some(serde_json::json!({
                        "permission_id": permission_id,
                        "node_id": node_id,
                    })),
                },
            };
            if !send_json_outbound(&outbound_for_events, &ws_msg).await {
                break;
            }
        }
    });

    let current_run: Arc<Mutex<Option<ActiveRun>>> = Arc::new(Mutex::new(None));
    let next_run_id: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
    let last_client_message_at = Arc::new(Mutex::new(tokio::time::Instant::now()));
    let idle_timeout_task = spawn_idle_timeout_task(
        last_client_message_at.clone(),
        outbound_tx.clone(),
        std::time::Duration::from_secs(90),
        std::time::Duration::from_secs(5),
    );

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
                let _ = send_json_outbound(&outbound_tx, &err).await;
                continue;
            }
        };
        *last_client_message_at.lock().await = tokio::time::Instant::now();

        let stage = if requires_stage_validation(&in_msg) {
            Some({
                let engine = engine.lock().await;
                engine.current_stage()
            })
        } else {
            None
        };
        if let Some(stage) = stage.as_ref()
            && !is_message_valid_for_stage(&in_msg, stage)
        {
            let err = WsOutMessage::ProtocolError {
                code: "INVALID_MESSAGE_FOR_STAGE".to_string(),
                message: format!(
                    "message {} not allowed in stage {}",
                    message_type(&in_msg),
                    stage.as_str()
                ),
                context: Some(serde_json::json!({
                    "stage": stage.as_str(),
                    "received": message_type(&in_msg),
                })),
            };
            let _ = send_json_outbound(&outbound_tx, &err).await;
            continue;
        }

        match in_msg {
            WsInMessage::UserMessage { content } => {
                if let Err(message) = spawn_provider_run_from_handler(
                    state.provider_registry.clone(),
                    engine.clone(),
                    current_run.clone(),
                    next_run_id.clone(),
                    ProviderRunKind::Author { content },
                )
                .await
                {
                    let err = WsOutMessage::Error { message };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
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
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                } else {
                    let state_msg = engine.build_session_state();
                    let _ = send_json_outbound(&outbound_tx, &state_msg).await;
                }
            }
            WsInMessage::Confirm => {
                handle_human_confirm_from_handler(
                    state.provider_registry.clone(),
                    engine.clone(),
                    current_run.clone(),
                    next_run_id.clone(),
                    outbound_tx.clone(),
                    HumanConfirmDecision::Confirm,
                    None,
                )
                .await;
            }
            WsInMessage::ProviderSelect { role, provider } => {
                let mut engine = engine.lock().await;
                if let Err(e) = engine.set_provider(&role, provider) {
                    let err = WsOutMessage::Error { message: e };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                } else {
                    let state_msg = engine.build_session_state();
                    let _ = send_json_outbound(&outbound_tx, &state_msg).await;
                }
            }
            WsInMessage::PermissionResponse {
                id,
                approved,
                reason,
            } => {
                tracing::info!(permission_id = %id, approved, "ws inbound permission response");
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
            WsInMessage::ReviewDecisionResponse {
                decision,
                extra_context,
            } => {
                handle_review_decision_from_handler(
                    state.provider_registry.clone(),
                    engine.clone(),
                    current_run.clone(),
                    next_run_id.clone(),
                    outbound_tx.clone(),
                    decision,
                    extra_context,
                )
                .await;
            }
            WsInMessage::Abort => {
                let active = { current_run.lock().await.take() };
                if let Some(run) = active {
                    let _ = run.command_tx.send(ProviderCommand::Abort).await;
                    run.cancel.cancel();
                }
            }
            WsInMessage::Ping => {
                let _ = send_json_outbound(&outbound_tx, &WsOutMessage::Pong).await;
            }
            WsInMessage::Hello { .. } => {
                let state_msg = engine.lock().await.build_session_state();
                let _ = send_json_outbound(&outbound_tx, &state_msg).await;
            }
            WsInMessage::ContextNote { content } => {
                let result = {
                    let mut engine = engine.lock().await;
                    engine.append_context_note(content).await
                };
                if let Err(message) = result {
                    let err = WsOutMessage::Error { message };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
            }
            WsInMessage::StartGeneration {
                provider_config,
                reviewer_enabled,
            } => {
                let result = {
                    let mut engine = engine.lock().await;
                    engine
                        .start_generation(provider_config, reviewer_enabled)
                        .await
                };
                match result {
                    Ok((_node, locked)) => {
                        let _ = send_json_outbound(&outbound_tx, &locked).await;
                        if let Err(message) = spawn_provider_run_from_handler(
                            state.provider_registry.clone(),
                            engine.clone(),
                            current_run.clone(),
                            next_run_id.clone(),
                            ProviderRunKind::Author {
                                content: String::new(),
                            },
                        )
                        .await
                        {
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx, &err).await;
                        }
                    }
                    Err(message) => {
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                }
            }
            WsInMessage::SelectRevisionPath {
                path,
                extra_context,
            } => {
                let (decision, extra_context) = map_revision_path(path, extra_context);
                handle_review_decision_from_handler(
                    state.provider_registry.clone(),
                    engine.clone(),
                    current_run.clone(),
                    next_run_id.clone(),
                    outbound_tx.clone(),
                    decision,
                    extra_context,
                )
                .await;
            }
            WsInMessage::RequestRevision { feedback } => {
                let payload = serde_json::to_value(feedback).ok();
                handle_human_confirm_from_handler(
                    state.provider_registry.clone(),
                    engine.clone(),
                    current_run.clone(),
                    next_run_id.clone(),
                    outbound_tx.clone(),
                    HumanConfirmDecision::RequestChange,
                    payload,
                )
                .await;
            }
            WsInMessage::HumanConfirm { decision, payload } => {
                handle_human_confirm_from_handler(
                    state.provider_registry.clone(),
                    engine.clone(),
                    current_run.clone(),
                    next_run_id.clone(),
                    outbound_tx.clone(),
                    decision,
                    payload,
                )
                .await;
            }
        }
    }

    let active = { current_run.lock().await.take() };
    if let Some(run) = active {
        let last_active_run_id = format!("run-{}", run.id);
        let _ = run.command_tx.send(ProviderCommand::Abort).await;
        run.cancel.cancel();
        let mut engine = engine.lock().await;
        let _ = engine
            .append_aborted_by_disconnect(last_active_run_id)
            .await;
        engine
            .transition_to_prepare_context_after_disconnect()
            .await;
        let state_msg = engine.build_session_state();
        let _ = send_json_outbound(&outbound_tx, &state_msg).await;
    }
    drop(outbound_tx);
    idle_timeout_task.abort();
    event_forward_task.abort();
    send_task.abort();
    let _ = event_forward_task.await;
    let _ = send_task.await;
}

async fn handle_review_decision_from_handler(
    provider_registry: Arc<ProviderRegistry>,
    engine: Arc<Mutex<WorkspaceEngine>>,
    current_run: Arc<Mutex<Option<ActiveRun>>>,
    next_run_id: Arc<Mutex<u64>>,
    outbound_tx: mpsc::Sender<OutboundControl>,
    decision: String,
    extra_context: Option<String>,
) {
    let outcome = {
        let mut engine = engine.lock().await;
        engine.handle_review_decision(decision, extra_context).await
    };

    match outcome {
        Ok(ReviewDecisionOutcome::HumanConfirm) => {}
        Ok(ReviewDecisionOutcome::StartRevision) => {
            if let Err(message) = spawn_provider_run_from_handler(
                provider_registry,
                engine,
                current_run,
                next_run_id,
                ProviderRunKind::Revision,
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Err(message) => {
            let err = WsOutMessage::Error { message };
            let _ = send_json_outbound(&outbound_tx, &err).await;
        }
    }
}

async fn handle_human_confirm_from_handler(
    provider_registry: Arc<ProviderRegistry>,
    engine: Arc<Mutex<WorkspaceEngine>>,
    current_run: Arc<Mutex<Option<ActiveRun>>>,
    next_run_id: Arc<Mutex<u64>>,
    outbound_tx: mpsc::Sender<OutboundControl>,
    decision: HumanConfirmDecision,
    payload: Option<serde_json::Value>,
) {
    let outcome = {
        let mut engine = engine.lock().await;
        engine.handle_human_confirm(decision, payload).await
    };

    match outcome {
        Ok(ReviewDecisionOutcome::HumanConfirm) => {}
        Ok(ReviewDecisionOutcome::StartRevision) => {
            if let Err(message) = spawn_provider_run_from_handler(
                provider_registry,
                engine,
                current_run,
                next_run_id,
                ProviderRunKind::Revision,
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Err(message) => {
            let err = WsOutMessage::ProtocolError {
                code: "INVALID_HUMAN_CONFIRM_ACTION".to_string(),
                message,
                context: None,
            };
            let _ = send_json_outbound(&outbound_tx, &err).await;
        }
    }
}

fn is_message_valid_for_stage(msg: &WsInMessage, stage: &WorkspaceStage) -> bool {
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
                WsInMessage::Abort | WsInMessage::PermissionResponse { .. }
            )
        }
        WorkspaceStage::CrossReview => matches!(msg, WsInMessage::Abort),
        WorkspaceStage::ReviewDecision => matches!(
            msg,
            WsInMessage::SelectRevisionPath { .. } | WsInMessage::ReviewDecisionResponse { .. }
        ),
        WorkspaceStage::Revision => matches!(msg, WsInMessage::Abort),
        WorkspaceStage::HumanConfirm => matches!(
            msg,
            WsInMessage::HumanConfirm { .. }
                | WsInMessage::RequestRevision { .. }
                | WsInMessage::Confirm
        ),
        WorkspaceStage::Completed => false,
    }
}

fn requires_stage_validation(msg: &WsInMessage) -> bool {
    !matches!(
        msg,
        WsInMessage::Abort
            | WsInMessage::PermissionResponse { .. }
            | WsInMessage::UserMessage { .. }
            | WsInMessage::Rollback { .. }
    )
}

fn message_type(msg: &WsInMessage) -> &'static str {
    match msg {
        WsInMessage::UserMessage { .. } => "user_message",
        WsInMessage::ContextNote { .. } => "context_note",
        WsInMessage::StartGeneration { .. } => "start_generation",
        WsInMessage::Hello { .. } => "hello",
        WsInMessage::Rollback { .. } => "rollback",
        WsInMessage::Confirm => "confirm",
        WsInMessage::ProviderSelect { .. } => "provider_select",
        WsInMessage::PermissionResponse { .. } => "permission_response",
        WsInMessage::ReviewDecisionResponse { .. } => "review_decision_response",
        WsInMessage::SelectRevisionPath { .. } => "select_revision_path",
        WsInMessage::RequestRevision { .. } => "request_revision",
        WsInMessage::HumanConfirm { .. } => "human_confirm",
        WsInMessage::Abort => "abort",
        WsInMessage::Ping => "ping",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::workspace_ws_types::{
        HumanConfirmDecision, ProviderConfigSnapshot, RevisionPath, StructuredFeedback,
    };

    fn provider_config() -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: None,
            review_rounds: 0,
        }
    }

    #[test]
    fn context_note_is_only_valid_in_prepare_context() {
        let msg = WsInMessage::ContextNote {
            content: "补充上下文".to_string(),
        };

        assert!(is_message_valid_for_stage(
            &msg,
            &WorkspaceStage::PrepareContext
        ));
        assert!(!is_message_valid_for_stage(&msg, &WorkspaceStage::Running));
    }

    #[test]
    fn start_generation_is_only_valid_in_prepare_context() {
        let msg = WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        };

        assert!(is_message_valid_for_stage(
            &msg,
            &WorkspaceStage::PrepareContext
        ));
        assert!(!is_message_valid_for_stage(&msg, &WorkspaceStage::Running));
    }

    #[test]
    fn hello_and_ping_are_valid_for_every_stage() {
        let hello = WsInMessage::Hello {
            session_id: "session-1".to_string(),
            last_seen_node_id: Some("node-1".to_string()),
        };
        let ping = WsInMessage::Ping;

        for stage in [
            WorkspaceStage::PrepareContext,
            WorkspaceStage::Running,
            WorkspaceStage::CrossReview,
            WorkspaceStage::ReviewDecision,
            WorkspaceStage::Revision,
            WorkspaceStage::HumanConfirm,
            WorkspaceStage::Completed,
        ] {
            assert!(is_message_valid_for_stage(&hello, &stage));
            assert!(is_message_valid_for_stage(&ping, &stage));
        }
    }

    #[tokio::test]
    async fn idle_timeout_sends_close_control_after_client_quiet() {
        let last_client_message_at = Arc::new(Mutex::new(tokio::time::Instant::now()));
        let (tx, mut rx) = mpsc::channel(1);

        let task = spawn_idle_timeout_task(
            last_client_message_at,
            tx,
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(1),
        );

        let control = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
            .await
            .expect("idle timeout control")
            .expect("close control");
        assert!(matches!(control, OutboundControl::CloseDueToIdleTimeout));

        task.abort();
    }

    #[test]
    fn revision_path_messages_are_only_valid_in_review_decision() {
        let select_path = WsInMessage::SelectRevisionPath {
            path: RevisionPath::ReviseWithContext,
            extra_context: Some("补充修改约束".to_string()),
        };
        let legacy_decision = WsInMessage::ReviewDecisionResponse {
            decision: "continue".to_string(),
            extra_context: None,
        };

        assert!(is_message_valid_for_stage(
            &select_path,
            &WorkspaceStage::ReviewDecision
        ));
        assert!(is_message_valid_for_stage(
            &legacy_decision,
            &WorkspaceStage::ReviewDecision
        ));
        assert!(!is_message_valid_for_stage(
            &select_path,
            &WorkspaceStage::HumanConfirm
        ));
        assert!(!is_message_valid_for_stage(
            &legacy_decision,
            &WorkspaceStage::HumanConfirm
        ));
    }

    #[test]
    fn human_confirm_messages_are_only_valid_in_human_confirm() {
        let human_confirm = WsInMessage::HumanConfirm {
            decision: HumanConfirmDecision::RequestChange,
            payload: Some(serde_json::json!({"description": "补充验收条件"})),
        };
        let legacy_request_revision = WsInMessage::RequestRevision {
            feedback: StructuredFeedback {
                feedback_types: vec!["clarity".to_string()],
                description: "补充验收条件".to_string(),
                target_artifact_version: Some(1),
            },
        };
        let legacy_confirm = WsInMessage::Confirm;

        assert!(is_message_valid_for_stage(
            &human_confirm,
            &WorkspaceStage::HumanConfirm
        ));
        assert!(is_message_valid_for_stage(
            &legacy_request_revision,
            &WorkspaceStage::HumanConfirm
        ));
        assert!(is_message_valid_for_stage(
            &legacy_confirm,
            &WorkspaceStage::HumanConfirm
        ));
        assert!(!is_message_valid_for_stage(
            &human_confirm,
            &WorkspaceStage::ReviewDecision
        ));
        assert!(!is_message_valid_for_stage(
            &legacy_request_revision,
            &WorkspaceStage::ReviewDecision
        ));
    }

    #[test]
    fn completed_stage_rejects_business_messages() {
        assert!(!is_message_valid_for_stage(
            &WsInMessage::Abort,
            &WorkspaceStage::Completed
        ));
        assert!(!is_message_valid_for_stage(
            &WsInMessage::ContextNote {
                content: "late note".to_string()
            },
            &WorkspaceStage::Completed
        ));
    }

    #[test]
    fn control_and_legacy_messages_do_not_require_stage_lock_validation() {
        assert!(!requires_stage_validation(&WsInMessage::Abort));
        assert!(!requires_stage_validation(
            &WsInMessage::PermissionResponse {
                id: "permission-1".to_string(),
                approved: true,
                reason: None,
            }
        ));
        assert!(!requires_stage_validation(&WsInMessage::UserMessage {
            content: "legacy generation request".to_string(),
        }));
        assert!(!requires_stage_validation(&WsInMessage::Rollback {
            checkpoint_id: "cp_001".to_string(),
        }));
        assert!(requires_stage_validation(&WsInMessage::ContextNote {
            content: "new protocol action".to_string(),
        }));
    }

    #[test]
    fn revision_path_maps_to_existing_review_decision_contract() {
        assert_eq!(
            map_revision_path(RevisionPath::Revise, Some("ignored".to_string())),
            ("continue".to_string(), None)
        );
        assert_eq!(
            map_revision_path(
                RevisionPath::ReviseWithContext,
                Some("补充约束".to_string())
            ),
            (
                "continue_with_context".to_string(),
                Some("补充约束".to_string())
            )
        );
        assert_eq!(
            map_revision_path(RevisionPath::SkipToHuman, Some("ignored".to_string())),
            ("human_intervene".to_string(), None)
        );
    }
}

#[derive(Debug)]
struct ActiveRun {
    id: u64,
    cancel: CancellationToken,
    command_tx: mpsc::Sender<ProviderCommand>,
}

enum ProviderRunKind {
    Author { content: String },
    Revision,
}

async fn abort_active_run(current_run: &Arc<Mutex<Option<ActiveRun>>>) {
    let active = { current_run.lock().await.take() };
    if let Some(run) = active {
        let _ = run.command_tx.send(ProviderCommand::Abort).await;
        run.cancel.cancel();
    }
}

async fn spawn_provider_run_from_handler(
    provider_registry: Arc<ProviderRegistry>,
    engine: Arc<Mutex<WorkspaceEngine>>,
    current_run: Arc<Mutex<Option<ActiveRun>>>,
    next_run_id: Arc<Mutex<u64>>,
    run_kind: ProviderRunKind,
) -> Result<(), String> {
    abort_active_run(&current_run).await;

    let provider_name = {
        let engine = engine.lock().await;
        engine.session().author_provider.clone()
    };
    let Some(provider_for_run) = provider_registry.get(&provider_name) else {
        return Err(format!("provider unavailable: {provider_name:?}"));
    };

    let run_id = {
        let mut next = next_run_id.lock().await;
        *next += 1;
        *next
    };
    let run_label = format!("run-{run_id}");
    let run_cancel = CancellationToken::new();
    let (command_tx, command_rx) = mpsc::channel(8);
    *current_run.lock().await = Some(ActiveRun {
        id: run_id,
        cancel: run_cancel.clone(),
        command_tx,
    });

    {
        let mut engine = engine.lock().await;
        engine.mark_active_run_started(run_label.clone());
    }

    let engine_for_run = engine.clone();
    let current_run_for_task = current_run.clone();
    let provider_registry_for_run = provider_registry.clone();
    tokio::spawn(async move {
        let mut engine = engine_for_run.lock().await;
        engine.use_run_token(run_cancel.clone());
        match run_kind {
            ProviderRunKind::Author { content } => {
                engine
                    .handle_user_message(content, provider_for_run, command_rx)
                    .await;
            }
            ProviderRunKind::Revision => {
                engine
                    .drive_revision_session(provider_for_run, command_rx)
                    .await;
            }
        }
        while engine.session().stage == WorkspaceStage::CrossReview {
            let reviewer_name = engine
                .session()
                .reviewer_provider
                .clone()
                .unwrap_or(ProviderName::Codex);
            let Some(provider_for_review) = provider_registry_for_run.get(&reviewer_name) else {
                break;
            };
            let (review_command_tx, review_command_rx) = mpsc::channel(8);
            {
                let mut current = current_run_for_task.lock().await;
                if let Some(active) = current.as_mut()
                    && active.id == run_id
                {
                    active.command_tx = review_command_tx;
                }
            }
            engine
                .drive_review_session(provider_for_review, review_command_rx)
                .await;
        }
        engine.mark_active_run_finished(&run_label);
        drop(engine);

        let mut current = current_run_for_task.lock().await;
        if current.as_ref().is_some_and(|active| active.id == run_id) {
            *current = None;
        }
    });

    Ok(())
}

fn map_revision_path(
    path: RevisionPath,
    extra_context: Option<String>,
) -> (String, Option<String>) {
    match path {
        RevisionPath::Revise => ("continue".to_string(), None),
        RevisionPath::ReviseWithContext => ("continue_with_context".to_string(), extra_context),
        RevisionPath::SkipToHuman => ("human_intervene".to_string(), None),
    }
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

fn ws_execution_event(
    event: ProviderExecutionEvent,
    node_id: Option<String>,
    agent: Option<crate::product::models::ProviderName>,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: event.event_id,
        node_id,
        agent,
        kind: ws_execution_event_kind(event.kind),
        status: ws_execution_event_status(event.status),
        title: event.title,
        detail: event.detail,
        command: event.command,
        cwd: event.cwd,
        output: event.output,
        exit_code: event.exit_code,
    }
}

fn ws_execution_event_kind(kind: ProviderExecutionEventKind) -> WsExecutionEventKind {
    match kind {
        ProviderExecutionEventKind::Provider => WsExecutionEventKind::Provider,
        ProviderExecutionEventKind::Turn => WsExecutionEventKind::Turn,
        ProviderExecutionEventKind::Command => WsExecutionEventKind::Command,
        ProviderExecutionEventKind::Output => WsExecutionEventKind::Output,
        ProviderExecutionEventKind::Artifact => WsExecutionEventKind::Artifact,
    }
}

fn ws_execution_event_status(status: ProviderExecutionEventStatus) -> WsExecutionEventStatus {
    match status {
        ProviderExecutionEventStatus::Started => WsExecutionEventStatus::Started,
        ProviderExecutionEventStatus::Running => WsExecutionEventStatus::Running,
        ProviderExecutionEventStatus::WaitingApproval => WsExecutionEventStatus::WaitingApproval,
        ProviderExecutionEventStatus::Completed => WsExecutionEventStatus::Completed,
        ProviderExecutionEventStatus::Failed => WsExecutionEventStatus::Failed,
        ProviderExecutionEventStatus::Aborted => WsExecutionEventStatus::Aborted,
    }
}
