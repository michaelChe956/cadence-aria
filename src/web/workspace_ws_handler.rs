use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use axum::extract::ws::{CloseFrame, Message, WebSocket, close_code};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::parse_last_structured_output;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestSource, ProviderCommand, ProviderExecutionEvent,
    ProviderExecutionEventKind, ProviderExecutionEventStatus, ProviderStatus, RiskLevel,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::issue_store::IssueStore;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{ProviderName, WorkspaceSessionRecord, WorkspaceType};
use crate::product::work_item_split_engine::WorkItemSplitEngine;
use crate::product::workspace_engine::{
    AuthorDecisionOutcome, EngineEvent, PendingAuthorChoiceError, ReviewDecisionOutcome,
    WorkItemPlanAuthorOutcome, WorkspaceEngine, WorkspaceSession, WorkspaceStage,
    build_work_item_plan_revision_input,
};
use crate::product::workspace_repository::workspace_repository_for_session;
use crate::web::state::{WebAppState, WorkspaceActiveRun, WorkspaceRunRegistry};
use crate::web::test_controls::WorkspaceSocketControl;
use crate::web::types::GenerateWorkItemsRequest;
use crate::web::workspace_context::ensure_workspace_context_message;
use crate::web::workspace_ws_types::{
    ChoiceOption, HumanConfirmDecision, RevisionPath, WsExecutionEvent, WsExecutionEventKind,
    WsExecutionEventStatus, WsInMessage, WsOutMessage, WsPermissionRiskLevel, WsProviderStatus,
};

pub async fn workspace_ws(
    ws: WebSocketUpgrade,
    Path(session_id): Path<String>,
    State(state): State<WebAppState>,
) -> impl IntoResponse {
    if state
        .test_controls
        .consume_workspace_socket_reject(&session_id)
        .await
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    ws.on_upgrade(move |socket| handle_workspace_socket(socket, session_id, state))
        .into_response()
}

#[derive(Debug)]
enum OutboundControl {
    Text(String),
    CloseDueToIdleTimeout,
    CloseForTestDrop,
}

static NEXT_ACTIVE_RUN_TOKEN: AtomicU64 = AtomicU64::new(1);

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
    is_active_run: Arc<dyn Fn() -> bool + Send + Sync>,
    timeout_after: std::time::Duration,
    tick_every: std::time::Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick_every);
        loop {
            interval.tick().await;
            let last_seen = *last_client_message_at.lock().await;
            if last_seen.elapsed() > timeout_after && !is_active_run() {
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

    let mut session = WorkspaceSession::from_record(session_record.clone());
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

    let (session_state, restored_choice_request) = {
        let engine = engine.lock().await;
        (
            engine.build_session_state(),
            engine.pending_author_choice_request_message(),
        )
    };
    if let Ok(json) = serde_json::to_string(&session_state) {
        let _ = ws_sender.send(Message::Text(json.into())).await;
    }
    if let Some(choice_request) = restored_choice_request
        && let Ok(json) = serde_json::to_string(&choice_request)
    {
        let _ = ws_sender.send(Message::Text(json.into())).await;
    }

    let (outbound_tx, mut outbound_rx) = mpsc::channel::<OutboundControl>(64);
    let (socket_control_tx, mut socket_control_rx) = mpsc::channel::<WorkspaceSocketControl>(4);
    state
        .test_controls
        .register_workspace_socket(session_id.clone(), socket_control_tx)
        .await;

    let send_task = tokio::spawn(async move {
        while let Some(control) = outbound_rx.recv().await {
            match control {
                OutboundControl::Text(msg) => {
                    let diag_type = serde_json::from_str::<serde_json::Value>(&msg)
                        .ok()
                        .and_then(|value| {
                            let message_type = value.get("type")?.as_str()?.to_string();
                            let id = value
                                .get("id")
                                .and_then(serde_json::Value::as_str)
                                .map(ToString::to_string);
                            Some((message_type, id))
                        });
                    if let Some((message_type, id)) = diag_type.as_ref() {
                        eprintln!(
                            "[aria-choice-diag] ws send_task sending outbound type={} id={} bytes={}",
                            message_type,
                            id.as_deref().unwrap_or("<none>"),
                            msg.len()
                        );
                    }
                    if ws_sender.send(Message::Text(msg.into())).await.is_err() {
                        if let Some((message_type, id)) = diag_type.as_ref() {
                            eprintln!(
                                "[aria-choice-diag] ws send_task failed outbound type={} id={}",
                                message_type,
                                id.as_deref().unwrap_or("<none>")
                            );
                        }
                        break;
                    }
                    if let Some((message_type, id)) = diag_type.as_ref() {
                        eprintln!(
                            "[aria-choice-diag] ws send_task sent outbound type={} id={}",
                            message_type,
                            id.as_deref().unwrap_or("<none>")
                        );
                    }
                }
                OutboundControl::CloseDueToIdleTimeout => {
                    let _ = ws_sender.close().await;
                    break;
                }
                OutboundControl::CloseForTestDrop => {
                    let _ = ws_sender
                        .send(Message::Close(Some(CloseFrame {
                            code: close_code::AWAY,
                            reason: "test drop".into(),
                        })))
                        .await;
                    break;
                }
            }
        }
    });

    let outbound_for_socket_controls = outbound_tx.clone();
    let socket_control_task = tokio::spawn(async move {
        if let Some(WorkspaceSocketControl::CloseForTestDrop) = socket_control_rx.recv().await {
            let _ = outbound_for_socket_controls
                .send(OutboundControl::CloseForTestDrop)
                .await;
        }
    });

    let outbound_for_events = outbound_tx.clone();
    let session_id_for_events = session_id.clone();
    let workspace_runs_for_events = state.workspace_runs.clone();
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
                EngineEvent::ArtifactUpdate { version, payload } => {
                    WsOutMessage::ArtifactUpdate { version, payload }
                }
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
                EngineEvent::ChoiceRequest {
                    id,
                    prompt,
                    options,
                    allow_multiple,
                    allow_free_text,
                    source,
                } => {
                    eprintln!(
                        "[aria-choice-diag] ws outbound choice_request session={} id={} source={} options={} prompt_chars={}",
                        session_id_for_events,
                        id,
                        source.as_str(),
                        options.len(),
                        prompt.chars().count()
                    );
                    if source != ChoiceRequestSource::TextFallback {
                        let _ = workspace_runs_for_events
                            .register_choice(&session_id_for_events, id.clone())
                            .await;
                    }
                    WsOutMessage::ChoiceRequest {
                        id,
                        prompt,
                        options: options.into_iter().map(ws_choice_option).collect(),
                        allow_multiple,
                        allow_free_text,
                        source: source.as_str().to_string(),
                    }
                }
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
                    findings,
                    review_gate,
                } => WsOutMessage::ReviewComplete {
                    node_id,
                    round,
                    verdict,
                    comments,
                    summary,
                    findings,
                    review_gate,
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

    let current_run: Arc<Mutex<Option<WorkspaceActiveRun>>> = Arc::new(Mutex::new(None));
    let next_run_id: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
    let run_context = ProviderRunContext {
        provider_registry: state.provider_registry.clone(),
        engine: engine.clone(),
        current_run: current_run.clone(),
        workspace_runs: state.workspace_runs.clone(),
        session_id: session_id.clone(),
        next_run_id: next_run_id.clone(),
        app_paths: app_paths.clone(),
        session_record: session_record.clone(),
    };
    let last_client_message_at = Arc::new(Mutex::new(tokio::time::Instant::now()));
    let current_run_for_idle = current_run.clone();
    let idle_timeout_task = spawn_idle_timeout_task(
        last_client_message_at.clone(),
        outbound_tx.clone(),
        Arc::new(move || {
            current_run_for_idle
                .try_lock()
                .map(|run| run.is_some())
                .unwrap_or(true)
        }),
        state.test_controls.server_idle_timeout(),
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

        let stage_and_type = if requires_stage_validation(&in_msg) {
            Some({
                let engine = engine.lock().await;
                (
                    engine.current_stage(),
                    engine.session().workspace_type.clone(),
                )
            })
        } else {
            None
        };
        if let Some((stage, workspace_type)) = stage_and_type.as_ref()
            && !is_message_valid_for_stage(&in_msg, stage)
            && !(matches!(in_msg, WsInMessage::RequestRevision { .. })
                && *stage == WorkspaceStage::AuthorConfirm
                && *workspace_type == WorkspaceType::WorkItemPlan)
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
                    run_context.clone(),
                    ProviderRunKind::Author { content },
                    outbound_tx.clone(),
                )
                .await
                {
                    let err = WsOutMessage::Error { message };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
            }
            WsInMessage::Rollback { checkpoint_id } => {
                abort_active_run(&current_run, &state.workspace_runs, &session_id).await;
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
                    run_context.clone(),
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
                let command_tx =
                    active_run_command_tx(&current_run, &state.workspace_runs, &session_id).await;
                if let Some(command_tx) = command_tx {
                    let _ = command_tx
                        .send(ProviderCommand::PermissionResponse {
                            id,
                            approved,
                            reason,
                        })
                        .await;
                } else {
                    let _ = send_json_outbound(
                        &outbound_tx,
                        &missing_active_run_error("permission_response", &id),
                    )
                    .await;
                }
            }
            WsInMessage::ChoiceResponse {
                id,
                selected_option_ids,
                free_text,
            } => {
                tracing::info!(choice_id = %id, "ws inbound choice response");
                eprintln!(
                    "[aria-choice-diag] ws inbound choice_response session={} id={} selected={:?} free_text_present={}",
                    session_id,
                    id,
                    selected_option_ids,
                    free_text
                        .as_ref()
                        .is_some_and(|text| !text.trim().is_empty())
                );
                let active_run = active_run(&current_run, &state.workspace_runs, &session_id).await;
                if let Some(run) = active_run {
                    let mut pending_choice_ids = run.pending_choice_ids.lock().await;
                    if !pending_choice_ids.remove(&id) {
                        let _ =
                            send_json_outbound(&outbound_tx, &choice_id_unmatched_error(&id)).await;
                        continue;
                    }
                    drop(pending_choice_ids);

                    eprintln!(
                        "[aria-choice-diag] ws forwarding choice_response to active run session={} id={}",
                        session_id, id
                    );
                    if run
                        .command_tx
                        .send(ProviderCommand::ChoiceResponse {
                            id: id.clone(),
                            selected_option_ids: selected_option_ids.clone(),
                            free_text: free_text.clone(),
                        })
                        .await
                        .is_ok()
                    {
                        eprintln!(
                            "[aria-choice-diag] ws forwarded choice_response to active run session={} id={}",
                            session_id, id
                        );
                        continue;
                    }
                    eprintln!(
                        "[aria-choice-diag] ws failed to forward choice_response to active run session={} id={}; falling back",
                        session_id, id
                    );
                } else {
                    eprintln!(
                        "[aria-choice-diag] ws has no active run for choice_response session={} id={}; trying text fallback follow-up",
                        session_id, id
                    );
                }

                let prompt = {
                    let mut engine = engine.lock().await;
                    engine
                        .take_pending_author_choice_prompt(&id, selected_option_ids, free_text)
                        .await
                };
                match prompt {
                    Ok(content) => {
                        if let Err(message) = spawn_provider_run_from_handler(
                            run_context.clone(),
                            ProviderRunKind::AuthorChoiceFollowup { content },
                            outbound_tx.clone(),
                        )
                        .await
                        {
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx, &err).await;
                        }
                    }
                    Err(PendingAuthorChoiceError::NotFound { .. }) => {
                        let _ = send_json_outbound(
                            &outbound_tx,
                            &missing_active_run_error("choice_response", &id),
                        )
                        .await;
                    }
                    Err(error) => {
                        let err = WsOutMessage::ProtocolError {
                            code: error.code().to_string(),
                            message: error.message(),
                            context: Some(serde_json::json!({ "id": id })),
                        };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                }
            }
            WsInMessage::ReviewDecisionResponse {
                decision,
                extra_context,
            } => {
                handle_review_decision_from_handler(
                    run_context.clone(),
                    outbound_tx.clone(),
                    decision,
                    extra_context,
                )
                .await;
            }
            WsInMessage::AuthorDecision { decision } => {
                handle_author_decision_from_handler(
                    run_context.clone(),
                    outbound_tx.clone(),
                    decision,
                )
                .await;
            }
            WsInMessage::Abort => {
                if abort_active_run(&current_run, &state.workspace_runs, &session_id).await {
                    let _ = send_json_outbound(
                        &outbound_tx,
                        &WsOutMessage::ProviderStatus {
                            status: WsProviderStatus::Aborted,
                        },
                    )
                    .await;
                }
            }
            WsInMessage::Ping => {
                let _ = send_json_outbound(&outbound_tx, &WsOutMessage::Pong).await;
            }
            WsInMessage::Hello { .. } => {
                let engine_for_hello = engine.clone();
                let outbound_for_hello = outbound_tx.clone();
                tokio::spawn(async move {
                    let state_msg = {
                        let engine = engine_for_hello.lock().await;
                        engine.build_session_state()
                    };
                    let _ = send_json_outbound(&outbound_for_hello, &state_msg).await;
                });
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
                        let run_kind = {
                            let engine = engine.lock().await;
                            if engine.session().workspace_type == WorkspaceType::WorkItemPlan {
                                ProviderRunKind::WorkItemPlanAuthor
                            } else {
                                ProviderRunKind::Author {
                                    content: String::new(),
                                }
                            }
                        };
                        if let Err(message) = spawn_provider_run_from_handler(
                            run_context.clone(),
                            run_kind,
                            outbound_tx.clone(),
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
                    run_context.clone(),
                    outbound_tx.clone(),
                    decision,
                    extra_context,
                )
                .await;
            }
            WsInMessage::RequestRevision { feedback } => {
                let is_work_item_plan = {
                    let engine = engine.lock().await;
                    engine.session().workspace_type == WorkspaceType::WorkItemPlan
                };
                let feedback_text = {
                    let description = feedback.description.trim().to_string();
                    if description.is_empty() {
                        None
                    } else {
                        Some(description)
                    }
                };
                if is_work_item_plan {
                    let result = {
                        let mut engine = engine.lock().await;
                        engine
                            .request_work_item_plan_revision(feedback_text.clone())
                            .await
                    };
                    if let Err(message) = result {
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    } else if let Err(message) = spawn_provider_run_from_handler(
                        run_context.clone(),
                        ProviderRunKind::WorkItemPlanRevision {
                            feedback: feedback_text,
                        },
                        outbound_tx.clone(),
                    )
                    .await
                    {
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                } else {
                    let payload = serde_json::to_value(feedback).ok();
                    handle_human_confirm_from_handler(
                        run_context.clone(),
                        outbound_tx.clone(),
                        HumanConfirmDecision::RequestChange,
                        payload,
                    )
                    .await;
                }
            }
            WsInMessage::HumanConfirm { decision, payload } => {
                handle_human_confirm_from_handler(
                    run_context.clone(),
                    outbound_tx.clone(),
                    decision,
                    payload,
                )
                .await;
            }
            WsInMessage::RevertWorkItem {
                work_item_id,
                feedback,
                clear,
            } => {
                let result = {
                    let mut engine = engine.lock().await;
                    engine
                        .apply_revert_mark(&work_item_id, feedback, clear)
                        .await
                };
                if let Err(message) = result {
                    let err = WsOutMessage::Error { message };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
                // 成功时 apply_revert_mark 已发 EngineEvent::ArtifactUpdate，event forwarder 会推前端
            }
        }
    }

    let active = { current_run.lock().await.take() };
    if let Some(run) = active {
        let last_active_run_id = format!("run-{}", run.id);
        let owned_registry_run = state
            .workspace_runs
            .remove_if_token(&session_id, run.token)
            .await;
        abort_workspace_run(&run).await;
        if owned_registry_run {
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
    }
    drop(outbound_tx);
    idle_timeout_task.abort();
    socket_control_task.abort();
    event_forward_task.abort();
    send_task.abort();
    let _ = socket_control_task.await;
    let _ = event_forward_task.await;
    let _ = send_task.await;
}

#[derive(Clone)]
struct ProviderRunContext {
    provider_registry: Arc<ProviderRegistry>,
    engine: Arc<Mutex<WorkspaceEngine>>,
    current_run: Arc<Mutex<Option<WorkspaceActiveRun>>>,
    workspace_runs: WorkspaceRunRegistry,
    session_id: String,
    next_run_id: Arc<Mutex<u64>>,
    app_paths: ProductAppPaths,
    session_record: WorkspaceSessionRecord,
}

async fn handle_review_decision_from_handler(
    run_context: ProviderRunContext,
    outbound_tx: mpsc::Sender<OutboundControl>,
    decision: String,
    extra_context: Option<String>,
) {
    let outcome = {
        let mut engine = run_context.engine.lock().await;
        engine.handle_review_decision(decision, extra_context).await
    };

    match outcome {
        Ok(ReviewDecisionOutcome::HumanConfirm) => {}
        Ok(ReviewDecisionOutcome::ConfirmedWithChildSessions { .. }) => {
            // Review decision path never produces child sessions; defensive no-op.
        }
        Ok(ReviewDecisionOutcome::StartRevision) => {
            let run_kind = {
                let engine = run_context.engine.lock().await;
                if engine.session().workspace_type == WorkspaceType::WorkItemPlan {
                    ProviderRunKind::WorkItemPlanRevision {
                        feedback: engine.work_item_plan_revision_feedback(),
                    }
                } else {
                    ProviderRunKind::Revision
                }
            };
            if let Err(message) =
                spawn_provider_run_from_handler(run_context, run_kind, outbound_tx.clone()).await
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

async fn handle_author_decision_from_handler(
    run_context: ProviderRunContext,
    outbound_tx: mpsc::Sender<OutboundControl>,
    decision: crate::web::workspace_ws_types::AuthorDecision,
) {
    let outcome = {
        let mut engine = run_context.engine.lock().await;
        engine.handle_author_decision(decision).await
    };

    match outcome {
        Ok(AuthorDecisionOutcome::StartReview) => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::ReviewOnly,
                outbound_tx.clone(),
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(AuthorDecisionOutcome::HumanConfirm) => {}
        Ok(AuthorDecisionOutcome::PrepareContext) => {
            let state_msg = {
                let engine = run_context.engine.lock().await;
                engine.build_session_state()
            };
            let _ = send_json_outbound(&outbound_tx, &state_msg).await;
        }
        Err(message) => {
            let err = WsOutMessage::ProtocolError {
                code: "INVALID_AUTHOR_DECISION".to_string(),
                message,
                context: None,
            };
            let _ = send_json_outbound(&outbound_tx, &err).await;
        }
    }
}

async fn handle_human_confirm_from_handler(
    run_context: ProviderRunContext,
    outbound_tx: mpsc::Sender<OutboundControl>,
    decision: HumanConfirmDecision,
    payload: Option<serde_json::Value>,
) {
    let outcome = {
        let mut engine = run_context.engine.lock().await;
        engine.handle_human_confirm(decision, payload).await
    };

    match outcome {
        Ok(ReviewDecisionOutcome::HumanConfirm) => {}
        Ok(ReviewDecisionOutcome::ConfirmedWithChildSessions { child_sessions }) => {
            let lifecycle = LifecycleStore::new(run_context.app_paths.clone());
            for session in child_sessions {
                if let Err(error) =
                    ensure_workspace_context_message(&run_context.app_paths, &lifecycle, session)
                {
                    let err = WsOutMessage::Error {
                        message: format!("ensure child workspace context failed: {error}"),
                    };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                    return;
                }
            }
        }
        Ok(ReviewDecisionOutcome::StartRevision) => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::Revision,
                outbound_tx.clone(),
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

fn missing_active_run_error(message_type: &'static str, id: &str) -> WsOutMessage {
    WsOutMessage::ProtocolError {
        code: "ACTIVE_RUN_NOT_FOUND".to_string(),
        message: format!("{message_type} id={id} has no active provider run"),
        context: Some(serde_json::json!({
            "message_type": message_type,
            "id": id,
        })),
    }
}

fn choice_id_unmatched_error(id: &str) -> WsOutMessage {
    WsOutMessage::ProtocolError {
        code: "CHOICE_ID_UNMATCHED".to_string(),
        message: format!("ChoiceResponse id={id} not found in pending"),
        context: Some(serde_json::json!({ "choice_id": id })),
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
                WsInMessage::Abort
                    | WsInMessage::PermissionResponse { .. }
                    | WsInMessage::ChoiceResponse { .. }
            )
        }
        WorkspaceStage::AuthorConfirm => {
            matches!(
                msg,
                WsInMessage::AuthorDecision { .. }
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
            | WsInMessage::ChoiceResponse { .. }
            | WsInMessage::UserMessage { .. }
            | WsInMessage::Rollback { .. }
            | WsInMessage::Hello { .. }
            | WsInMessage::Ping
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
        WsInMessage::ChoiceResponse { .. } => "choice_response",
        WsInMessage::ReviewDecisionResponse { .. } => "review_decision_response",
        WsInMessage::AuthorDecision { .. } => "author_decision",
        WsInMessage::SelectRevisionPath { .. } => "select_revision_path",
        WsInMessage::RequestRevision { .. } => "request_revision",
        WsInMessage::HumanConfirm { .. } => "human_confirm",
        WsInMessage::RevertWorkItem { .. } => "revert_work_item",
        WsInMessage::Abort => "abort",
        WsInMessage::Ping => "ping",
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::web::workspace_ws_types::{
        AuthorDecision, HumanConfirmDecision, ProviderConfigSnapshot, RevisionPath,
        StructuredFeedback,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

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
            WorkspaceStage::AuthorConfirm,
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
            Arc::new(|| false),
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

    #[tokio::test]
    async fn idle_timeout_waits_while_provider_run_is_active() {
        let last_client_message_at = Arc::new(Mutex::new(tokio::time::Instant::now()));
        let (tx, mut rx) = mpsc::channel(1);
        let active = Arc::new(AtomicBool::new(true));
        let active_for_task = active.clone();

        let task = spawn_idle_timeout_task(
            last_client_message_at,
            tx,
            Arc::new(move || active_for_task.load(Ordering::SeqCst)),
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(1),
        );

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), rx.recv())
                .await
                .is_err()
        );

        active.store(false, Ordering::SeqCst);
        let control = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
            .await
            .expect("idle timeout after active run")
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
    fn author_decision_is_only_valid_in_author_confirm() {
        let msg = WsInMessage::AuthorDecision {
            decision: AuthorDecision::Accept,
        };

        assert!(is_message_valid_for_stage(
            &msg,
            &WorkspaceStage::AuthorConfirm
        ));
        assert!(!is_message_valid_for_stage(
            &msg,
            &WorkspaceStage::PrepareContext
        ));
        assert!(!is_message_valid_for_stage(
            &msg,
            &WorkspaceStage::HumanConfirm
        ));
        assert!(requires_stage_validation(&msg));
        assert_eq!(message_type(&msg), "author_decision");
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
        assert!(!requires_stage_validation(&WsInMessage::ChoiceResponse {
            id: "choice-1".to_string(),
            selected_option_ids: vec!["continue".to_string()],
            free_text: None,
        }));
        assert!(!requires_stage_validation(&WsInMessage::UserMessage {
            content: "legacy generation request".to_string(),
        }));
        assert!(!requires_stage_validation(&WsInMessage::Rollback {
            checkpoint_id: "cp_001".to_string(),
        }));
        assert!(!requires_stage_validation(&WsInMessage::Hello {
            session_id: "session-1".to_string(),
            last_seen_node_id: None,
        }));
        assert!(!requires_stage_validation(&WsInMessage::Ping));
        assert!(requires_stage_validation(&WsInMessage::ContextNote {
            content: "new protocol action".to_string(),
        }));
    }

    #[test]
    fn choice_response_message_type_is_reported_for_protocol_errors() {
        assert_eq!(
            message_type(&WsInMessage::ChoiceResponse {
                id: "choice-1".to_string(),
                selected_option_ids: vec!["continue".to_string()],
                free_text: None,
            }),
            "choice_response"
        );
    }

    #[test]
    fn missing_active_run_error_uses_protocol_error() {
        let error = missing_active_run_error("choice_response", "choice-1");

        match error {
            WsOutMessage::ProtocolError {
                code,
                message,
                context,
            } => {
                assert_eq!(code, "ACTIVE_RUN_NOT_FOUND");
                assert!(message.contains("choice_response"));
                assert_eq!(
                    context
                        .as_ref()
                        .and_then(|value| value.get("id"))
                        .and_then(|value| value.as_str()),
                    Some("choice-1")
                );
            }
            other => panic!("expected protocol error, got {other:?}"),
        }
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

    #[test]
    fn build_work_item_plan_generate_request_includes_validator_findings_as_revision_feedback() {
        use crate::product::app_paths::ProductAppPaths;
        use crate::product::checkpoint_store::CheckpointStore;
        use crate::product::lifecycle_store::{
            CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
            CreateWorkspaceSessionInput,
        };
        use crate::product::models::{
            IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, ProviderName, WorkItemSplitFinding,
            WorkItemSplitFindingSeverity, WorkspaceType,
        };
        use std::sync::Arc;

        let tmp = tempfile::tempdir().unwrap();
        let lifecycle = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));

        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repo_0001".to_string(),
                title: "Story".to_string(),
            })
            .unwrap();
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "Design".to_string(),
            })
            .unwrap();

        let finding = WorkItemSplitFinding {
            severity: WorkItemSplitFindingSeverity::Error,
            code: "write_scope_required".to_string(),
            message: "work item must have at least one exclusive_write_scope".to_string(),
            work_item_ids: vec!["wi_001".to_string()],
        };
        let plan = lifecycle
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: Some("issue_work_item_plan_0001".to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                source_story_spec_ids: vec![story.id],
                source_design_spec_ids: vec![design.id],
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: vec![],
                repository_profile_ref: None,
                verification_plan_ids: vec![],
                dependency_graph: vec![],
                created_from_provider_run: None,
                validator_findings: vec![finding],
            })
            .unwrap();

        let session_record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: plan.id,
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
            })
            .unwrap();

        let session = WorkspaceSession::from_record(session_record);
        let checkpoint_store = Arc::new(CheckpointStore::new(tmp.path().to_path_buf()));
        let (tx, _rx) = mpsc::channel(1);
        let engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle.clone(), tx, session);

        let request = build_work_item_plan_generate_request(&engine, &lifecycle).unwrap();

        let feedback = request
            .revision_feedback
            .expect("revision_feedback should be set when plan has findings");
        assert!(feedback.contains("write_scope_required"));
        assert!(feedback.contains("work item must have at least one exclusive_write_scope"));
    }
}

enum ProviderRunKind {
    Author { content: String },
    AuthorChoiceFollowup { content: String },
    Revision,
    ReviewOnly,
    WorkItemPlanAuthor,
    WorkItemPlanRevision { feedback: Option<String> },
}

fn parse_work_item_split_structured_output(full_output: &str) -> Result<serde_json::Value, String> {
    parse_last_structured_output(full_output)
        .map_err(|error| error.details)
        .and_then(|structured| {
            structured.ok_or_else(|| "missing structured output sentinel".to_string())
        })
}

async fn active_run_command_tx(
    current_run: &Arc<Mutex<Option<WorkspaceActiveRun>>>,
    workspace_runs: &WorkspaceRunRegistry,
    session_id: &str,
) -> Option<mpsc::Sender<ProviderCommand>> {
    active_run(current_run, workspace_runs, session_id)
        .await
        .map(|run| run.command_tx.clone())
}

async fn active_run(
    current_run: &Arc<Mutex<Option<WorkspaceActiveRun>>>,
    workspace_runs: &WorkspaceRunRegistry,
    session_id: &str,
) -> Option<WorkspaceActiveRun> {
    let local = { current_run.lock().await.clone() };
    if local.is_some() {
        return local;
    }
    workspace_runs.run(session_id).await
}

async fn abort_workspace_run(run: &WorkspaceActiveRun) {
    let _ = run.command_tx.send(ProviderCommand::Abort).await;
    run.cancel.cancel();
}

async fn abort_active_run(
    current_run: &Arc<Mutex<Option<WorkspaceActiveRun>>>,
    workspace_runs: &WorkspaceRunRegistry,
    session_id: &str,
) -> bool {
    let active = { current_run.lock().await.take() };
    if let Some(run) = active {
        let _ = workspace_runs.remove_if_token(session_id, run.token).await;
        abort_workspace_run(&run).await;
        return true;
    }

    if let Some(run) = workspace_runs.take(session_id).await {
        abort_workspace_run(&run).await;
        return true;
    }

    false
}

async fn spawn_provider_run_from_handler(
    run_context: ProviderRunContext,
    run_kind: ProviderRunKind,
    outbound_tx: mpsc::Sender<OutboundControl>,
) -> Result<(), String> {
    let run_context_clone = run_context.clone();
    let ProviderRunContext {
        provider_registry,
        engine,
        current_run,
        workspace_runs,
        session_id,
        next_run_id,
        app_paths: _,
        session_record: _,
    } = run_context;

    abort_active_run(&current_run, &workspace_runs, &session_id).await;

    let provider_name = {
        let engine = engine.lock().await;
        match &run_kind {
            ProviderRunKind::Author { .. }
            | ProviderRunKind::AuthorChoiceFollowup { .. }
            | ProviderRunKind::Revision => engine.session().author_provider.clone(),
            ProviderRunKind::ReviewOnly => engine
                .session()
                .reviewer_provider
                .clone()
                .unwrap_or(ProviderName::Codex),
            ProviderRunKind::WorkItemPlanAuthor | ProviderRunKind::WorkItemPlanRevision { .. } => {
                engine.session().author_provider.clone()
            }
        }
    };
    let provider_for_run = {
        let Some(provider) = provider_registry.get(&provider_name) else {
            return Err(format!("provider unavailable: {provider_name:?}"));
        };
        provider
    };

    let run_id = {
        let mut next = next_run_id.lock().await;
        *next += 1;
        *next
    };
    let run_label = format!("run-{run_id}");
    let run_token = NEXT_ACTIVE_RUN_TOKEN.fetch_add(1, Ordering::Relaxed);
    let run_cancel = CancellationToken::new();
    let (command_tx, command_rx) = mpsc::channel(8);
    let active_run = WorkspaceActiveRun {
        id: run_id,
        token: run_token,
        cancel: run_cancel.clone(),
        command_tx: command_tx.clone(),
        pending_choice_ids: Arc::new(Mutex::new(std::collections::HashSet::new())),
    };
    *current_run.lock().await = Some(active_run.clone());
    workspace_runs.insert(session_id.clone(), active_run).await;

    {
        let mut engine = engine.lock().await;
        engine.mark_active_run_started(run_label.clone());
    }

    let engine_for_run = engine.clone();
    let current_run_for_task = current_run.clone();
    let workspace_runs_for_task = workspace_runs.clone();
    let session_id_for_task = session_id.clone();
    let provider_registry_for_run = provider_registry.clone();
    let outbound_tx_for_task = outbound_tx.clone();
    tokio::spawn(async move {
        let mut engine = engine_for_run.lock().await;
        engine.use_run_token(run_cancel.clone());
        match run_kind {
            ProviderRunKind::Author { content } => {
                engine
                    .handle_user_message(content, provider_for_run.clone(), command_rx)
                    .await;
            }
            ProviderRunKind::AuthorChoiceFollowup { content } => {
                engine
                    .handle_author_choice_followup_message(
                        content,
                        provider_for_run.clone(),
                        command_rx,
                    )
                    .await;
            }
            ProviderRunKind::Revision => {
                engine
                    .drive_revision_session(provider_for_run.clone(), command_rx)
                    .await;
            }
            ProviderRunKind::ReviewOnly => {
                engine
                    .drive_review_session(provider_for_run.clone(), command_rx)
                    .await;
            }
            ProviderRunKind::WorkItemPlanAuthor => {
                let lifecycle_for_run = LifecycleStore::new(run_context_clone.app_paths.clone());
                let app_paths_for_run = run_context_clone.app_paths.clone();
                let session_record_for_run = run_context_clone.session_record.clone();
                let mut command_rx = command_rx;

                let mut request =
                    match build_work_item_plan_generate_request(&engine, &lifecycle_for_run)
                        .map_err(|e| format!("build request failed: {e}"))
                    {
                        Ok(r) => r,
                        Err(message) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    };

                let repository = match workspace_repository_for_session(
                    &app_paths_for_run,
                    &lifecycle_for_run,
                    &session_record_for_run,
                ) {
                    Ok(r) => r,
                    Err(error) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("load repository failed: {error}"),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                let issue = match IssueStore::new(app_paths_for_run.clone()).get(
                    &session_record_for_run.project_id,
                    &session_record_for_run.issue_id,
                ) {
                    Ok(i) => i,
                    Err(error) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("load issue failed: {error}"),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                let author_provider = engine.session().author_provider.clone();
                let invocation = match WorkItemSplitEngine::build_generate_invocation(
                    &request,
                    &lifecycle_for_run,
                    &issue,
                    &repository,
                    author_provider,
                ) {
                    Ok(invocation) => invocation,
                    Err(error) => {
                        engine.mark_active_run_finished(&run_label);
                        let err = WsOutMessage::Error {
                            message: format!("split generate failed: {}", error.message),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                let node_id = engine.begin_work_item_plan_author_run().await;
                let provider_input = engine.build_work_item_plan_streaming_input(
                    invocation.provider_type.clone(),
                    invocation.prompt.clone(),
                    invocation.worktree_path.clone(),
                );
                let provider_session = provider_for_run
                    .start(provider_input, run_cancel.clone())
                    .await;
                let full_output = match engine
                    .drive_work_item_plan_provider_session_to_output(
                        provider_session,
                        &mut command_rx,
                        node_id,
                        invocation.author_provider.clone(),
                    )
                    .await
                {
                    Ok(output) => output,
                    Err(_) => {
                        engine.mark_active_run_finished(&run_label);
                        return;
                    }
                };
                let structured_output = match parse_work_item_split_structured_output(&full_output)
                {
                    Ok(output) => output,
                    Err(message) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("split generate failed: {message}"),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                let output = match WorkItemSplitEngine::complete_generate_from_structured_output(
                    &request,
                    &lifecycle_for_run,
                    &issue,
                    &repository,
                    &invocation.author_provider,
                    &invocation.prompt,
                    structured_output,
                ) {
                    Ok(output) => output,
                    Err(error) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("split generate failed: {}", error.message),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                let mut outcome = match engine.complete_work_item_plan_author(output).await {
                    Ok(o) => o,
                    Err(message) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                // 使用本地循环处理 AutoRevision，直接复用 Task 1 的 `generate_revision`
                //（retained/redo 均为空，feedback 来自 validator findings），避免跨 spawn
                // 边界传递非 `Send` future 导致编译失败。
                // 循环上限由 `complete_work_item_plan_author` 内部的
                // `work_item_plan_author_retry_count` 控制（达到 3 次后返回 HumanConfirm）；
                // 下方的 `revision_iterations` 作为硬兜底。
                let mut revision_iterations = 0;
                loop {
                    match outcome {
                        WorkItemPlanAuthorOutcome::AuthorConfirm => {
                            engine.mark_active_run_finished(&run_label);
                            return;
                        }
                        WorkItemPlanAuthorOutcome::HumanConfirm { reason: _ } => {
                            engine.mark_active_run_finished(&run_label);
                            return;
                        }
                        WorkItemPlanAuthorOutcome::AutoRevision { findings: _ } => {
                            revision_iterations += 1;
                            if revision_iterations > 5 {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error {
                                    message: "work item plan author revision exceeded hard limit"
                                        .to_string(),
                                };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }

                            // 每次重生前重新构建请求，以便把最新 persisted 的 validator_findings
                            // 作为 revision_feedback 注入 prompt。
                            request = match build_work_item_plan_generate_request(
                                &engine,
                                &lifecycle_for_run,
                            )
                            .map_err(|e| format!("build request failed: {e}"))
                            {
                                Ok(r) => r,
                                Err(message) => {
                                    engine.mark_active_run_finished(&run_label);
                                    drop(engine);
                                    let err = WsOutMessage::Error { message };
                                    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                    return;
                                }
                            };
                            let author_provider = { engine.session().author_provider.clone() };
                            let invocation = match WorkItemSplitEngine::build_revision_invocation(
                                &request,
                                &lifecycle_for_run,
                                &issue,
                                &repository,
                                author_provider,
                                &[],
                                &[],
                            ) {
                                Ok(invocation) => invocation,
                                Err(error) => {
                                    engine.mark_active_run_finished(&run_label);
                                    drop(engine);
                                    let err = WsOutMessage::Error {
                                        message: format!(
                                            "split generate_revision failed: {}",
                                            error.message
                                        ),
                                    };
                                    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                    return;
                                }
                            };
                            let node_id = engine
                                .begin_work_item_plan_auto_revision_run(revision_iterations)
                                .await;
                            let provider_input = engine.build_work_item_plan_streaming_input(
                                invocation.provider_type.clone(),
                                invocation.prompt.clone(),
                                invocation.worktree_path.clone(),
                            );
                            let provider_session = provider_for_run
                                .start(provider_input, run_cancel.clone())
                                .await;
                            let full_output = match engine
                                .drive_work_item_plan_provider_session_to_output(
                                    provider_session,
                                    &mut command_rx,
                                    node_id,
                                    invocation.author_provider.clone(),
                                )
                                .await
                            {
                                Ok(output) => output,
                                Err(_) => {
                                    engine.mark_active_run_finished(&run_label);
                                    return;
                                }
                            };
                            let structured_output =
                                match parse_work_item_split_structured_output(&full_output) {
                                    Ok(output) => output,
                                    Err(message) => {
                                        engine.mark_active_run_finished(&run_label);
                                        drop(engine);
                                        let err = WsOutMessage::Error {
                                            message: format!(
                                                "split generate_revision failed: {message}"
                                            ),
                                        };
                                        let _ =
                                            send_json_outbound(&outbound_tx_for_task, &err).await;
                                        return;
                                    }
                                };
                            let output =
                                match WorkItemSplitEngine::complete_revision_from_structured_output(
                                    &request,
                                    &lifecycle_for_run,
                                    &issue,
                                    &repository,
                                    &invocation.author_provider,
                                    &invocation.prompt,
                                    structured_output,
                                    &[],
                                    &[],
                                ) {
                                    Ok(o) => o,
                                    Err(error) => {
                                        engine.mark_active_run_finished(&run_label);
                                        drop(engine);
                                        let err = WsOutMessage::Error {
                                            message: format!(
                                                "split generate_revision failed: {}",
                                                error.message
                                            ),
                                        };
                                        let _ =
                                            send_json_outbound(&outbound_tx_for_task, &err).await;
                                        return;
                                    }
                                };
                            outcome = match engine.complete_work_item_plan_author(output).await {
                                Ok(o) => o,
                                Err(message) => {
                                    engine.mark_active_run_finished(&run_label);
                                    drop(engine);
                                    let err = WsOutMessage::Error { message };
                                    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                    return;
                                }
                            };
                        }
                    }
                }
            }
            ProviderRunKind::WorkItemPlanRevision { feedback } => {
                let lifecycle_for_run = LifecycleStore::new(run_context_clone.app_paths.clone());
                let app_paths_for_run = run_context_clone.app_paths.clone();
                let session_record_for_run = run_context_clone.session_record.clone();
                let mut command_rx = command_rx;

                let (retained, redo_specs, request) = match build_work_item_plan_revision_input(
                    &engine,
                    &lifecycle_for_run,
                    feedback.as_deref(),
                )
                .map_err(|e| format!("build revision input failed: {e}"))
                {
                    Ok(r) => r,
                    Err(message) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                let repository = match workspace_repository_for_session(
                    &app_paths_for_run,
                    &lifecycle_for_run,
                    &session_record_for_run,
                ) {
                    Ok(r) => r,
                    Err(error) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("load repository failed: {error}"),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                let issue = match IssueStore::new(app_paths_for_run.clone()).get(
                    &session_record_for_run.project_id,
                    &session_record_for_run.issue_id,
                ) {
                    Ok(i) => i,
                    Err(error) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("load issue failed: {error}"),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                let author_provider = engine.session().author_provider.clone();
                let invocation = match WorkItemSplitEngine::build_revision_invocation(
                    &request,
                    &lifecycle_for_run,
                    &issue,
                    &repository,
                    author_provider,
                    &retained,
                    &redo_specs,
                ) {
                    Ok(invocation) => invocation,
                    Err(error) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("split generate_revision failed: {}", error.message),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                let Some(node_id) = engine.active_timeline_node_id() else {
                    engine.mark_active_run_finished(&run_label);
                    drop(engine);
                    let err = WsOutMessage::Error {
                        message: "work item plan revision node unavailable".to_string(),
                    };
                    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                    return;
                };
                let provider_input = engine.build_work_item_plan_streaming_input(
                    invocation.provider_type.clone(),
                    invocation.prompt.clone(),
                    invocation.worktree_path.clone(),
                );
                let provider_session = provider_for_run
                    .start(provider_input, run_cancel.clone())
                    .await;
                let full_output = match engine
                    .drive_work_item_plan_provider_session_to_output(
                        provider_session,
                        &mut command_rx,
                        node_id,
                        invocation.author_provider.clone(),
                    )
                    .await
                {
                    Ok(output) => output,
                    Err(_) => {
                        engine.mark_active_run_finished(&run_label);
                        return;
                    }
                };
                let structured_output = match parse_work_item_split_structured_output(&full_output)
                {
                    Ok(output) => output,
                    Err(message) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("split generate_revision failed: {message}"),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                let output = match WorkItemSplitEngine::complete_revision_from_structured_output(
                    &request,
                    &lifecycle_for_run,
                    &issue,
                    &repository,
                    &invocation.author_provider,
                    &invocation.prompt,
                    structured_output,
                    &retained,
                    &redo_specs,
                ) {
                    Ok(o) => o,
                    Err(error) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("split generate_revision failed: {}", error.message),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                let mut outcome = match engine.complete_work_item_plan_revision(output).await {
                    Ok(o) => o,
                    Err(message) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                // revision 也可能 validate 失败，使用与 author 相同的本地 AutoRevision 循环。
                // 由于 revision 的 retained/redo 是面向用户反馈的局部集合，AutoRevision 时
                // 退化为整组生成（retained/redo 均为空），让模型基于 validator findings 全局调整。
                let mut revision_iterations = 0;
                loop {
                    match outcome {
                        WorkItemPlanAuthorOutcome::AuthorConfirm => {
                            engine.mark_active_run_finished(&run_label);
                            return;
                        }
                        WorkItemPlanAuthorOutcome::HumanConfirm { reason: _ } => {
                            engine.mark_active_run_finished(&run_label);
                            return;
                        }
                        WorkItemPlanAuthorOutcome::AutoRevision { findings: _ } => {
                            revision_iterations += 1;
                            if revision_iterations > 5 {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error {
                                    message: "work item plan revision exceeded hard limit"
                                        .to_string(),
                                };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }

                            // 每次重生前重新构建请求，把最新 persisted 的 validator_findings
                            // 作为 revision_feedback 注入 prompt。
                            let request = match build_work_item_plan_generate_request(
                                &engine,
                                &lifecycle_for_run,
                            )
                            .map_err(|e| format!("build request failed: {e}"))
                            {
                                Ok(r) => r,
                                Err(message) => {
                                    engine.mark_active_run_finished(&run_label);
                                    drop(engine);
                                    let err = WsOutMessage::Error { message };
                                    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                    return;
                                }
                            };

                            // 整组 AutoRevision 时丢弃局部 retained/redo，使用完整 generate_revision。
                            let author_provider = engine.session().author_provider.clone();
                            let invocation = match WorkItemSplitEngine::build_revision_invocation(
                                &request,
                                &lifecycle_for_run,
                                &issue,
                                &repository,
                                author_provider,
                                &[],
                                &[],
                            ) {
                                Ok(invocation) => invocation,
                                Err(error) => {
                                    engine.mark_active_run_finished(&run_label);
                                    drop(engine);
                                    let err = WsOutMessage::Error {
                                        message: format!(
                                            "split generate_revision failed: {}",
                                            error.message
                                        ),
                                    };
                                    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                    return;
                                }
                            };
                            let node_id = engine
                                .begin_work_item_plan_auto_revision_run(revision_iterations)
                                .await;
                            let provider_input = engine.build_work_item_plan_streaming_input(
                                invocation.provider_type.clone(),
                                invocation.prompt.clone(),
                                invocation.worktree_path.clone(),
                            );
                            let provider_session = provider_for_run
                                .start(provider_input, run_cancel.clone())
                                .await;
                            let full_output = match engine
                                .drive_work_item_plan_provider_session_to_output(
                                    provider_session,
                                    &mut command_rx,
                                    node_id,
                                    invocation.author_provider.clone(),
                                )
                                .await
                            {
                                Ok(output) => output,
                                Err(_) => {
                                    engine.mark_active_run_finished(&run_label);
                                    return;
                                }
                            };
                            let structured_output =
                                match parse_work_item_split_structured_output(&full_output) {
                                    Ok(output) => output,
                                    Err(message) => {
                                        engine.mark_active_run_finished(&run_label);
                                        drop(engine);
                                        let err = WsOutMessage::Error {
                                            message: format!(
                                                "split generate_revision failed: {message}"
                                            ),
                                        };
                                        let _ =
                                            send_json_outbound(&outbound_tx_for_task, &err).await;
                                        return;
                                    }
                                };
                            let output =
                                match WorkItemSplitEngine::complete_revision_from_structured_output(
                                    &request,
                                    &lifecycle_for_run,
                                    &issue,
                                    &repository,
                                    &invocation.author_provider,
                                    &invocation.prompt,
                                    structured_output,
                                    &[],
                                    &[],
                                ) {
                                    Ok(o) => o,
                                    Err(error) => {
                                        engine.mark_active_run_finished(&run_label);
                                        drop(engine);
                                        let err = WsOutMessage::Error {
                                            message: format!(
                                                "split generate_revision failed: {}",
                                                error.message
                                            ),
                                        };
                                        let _ =
                                            send_json_outbound(&outbound_tx_for_task, &err).await;
                                        return;
                                    }
                                };

                            outcome = match engine.complete_work_item_plan_revision(output).await {
                                Ok(o) => o,
                                Err(message) => {
                                    engine.mark_active_run_finished(&run_label);
                                    drop(engine);
                                    let err = WsOutMessage::Error { message };
                                    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                    return;
                                }
                            };
                        }
                    }
                }
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
                    && active.token == run_token
                {
                    active.command_tx = review_command_tx.clone();
                }
            }
            workspace_runs_for_task
                .replace_command_tx_if_token(&session_id_for_task, run_token, review_command_tx)
                .await;
            engine
                .drive_review_session(provider_for_review, review_command_rx)
                .await;
        }
        engine.mark_active_run_finished(&run_label);
        drop(engine);

        let _ = workspace_runs_for_task
            .remove_if_token(&session_id_for_task, run_token)
            .await;
        let mut current = current_run_for_task.lock().await;
        if current
            .as_ref()
            .is_some_and(|active| active.token == run_token)
        {
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

fn ws_choice_option(option: ChoiceOptionData) -> ChoiceOption {
    ChoiceOption {
        id: option.id,
        label: option.label,
        description: option.description,
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

fn build_work_item_plan_generate_request(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
) -> Result<GenerateWorkItemsRequest, String> {
    let session = engine.session();
    let plan = lifecycle
        .get_issue_work_item_plan(&session.project_id, &session.issue_id, &session.entity_id)
        .map_err(|e| format!("load plan failed: {e}"))?;
    let provider_name_string = |name: &ProviderName| -> Result<String, String> {
        serde_json::to_value(name)
            .map_err(|e| format!("serialize provider name failed: {e}"))
            .and_then(|v| {
                v.as_str()
                    .map(ToString::to_string)
                    .ok_or_else(|| format!("provider name is not a string: {v}"))
            })
    };
    let revision_feedback = if plan.validator_findings.is_empty() {
        None
    } else {
        let feedback = plan
            .validator_findings
            .iter()
            .map(|finding| {
                format!(
                    "- [{}][{}] {} (work items: {})",
                    finding.severity.as_str(),
                    finding.code,
                    finding.message,
                    finding.work_item_ids.join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        Some(feedback)
    };
    Ok(GenerateWorkItemsRequest {
        title: plan.id.clone(),
        story_spec_ids: plan.source_story_spec_ids.clone(),
        design_spec_ids: plan.source_design_spec_ids.clone(),
        include_integration_tests: Some(plan.options.include_integration_tests),
        include_e2e_tests: Some(plan.options.include_e2e_tests),
        force_frontend_backend_split: Some(plan.options.force_frontend_backend_split),
        require_execution_plan_confirm: Some(plan.options.require_execution_plan_confirm),
        author_provider: Some(provider_name_string(&session.author_provider)?),
        reviewer_provider: session
            .reviewer_provider
            .as_ref()
            .map(provider_name_string)
            .transpose()?,
        review_rounds: Some(session.review_rounds),
        superpowers_enabled: Some(session.superpowers_enabled),
        openspec_enabled: Some(session.openspec_enabled),
        revision_feedback,
    })
}
