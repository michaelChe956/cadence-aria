use super::*;

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
pub(crate) enum OutboundControl {
    Text(String),
    CloseDueToIdleTimeout,
    CloseForTestDrop,
}

pub(crate) async fn send_json_outbound<T: serde::Serialize>(
    outbound_tx: &mpsc::Sender<OutboundControl>,
    message: &T,
) -> bool {
    match serde_json::to_string(message) {
        Ok(json) => outbound_tx.send(OutboundControl::Text(json)).await.is_ok(),
        Err(_) => false,
    }
}

pub(crate) fn spawn_idle_timeout_task(
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

pub(crate) async fn handle_workspace_socket(
    socket: WebSocket,
    session_id: String,
    state: WebAppState,
) {
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

    let (engine_tx, engine_rx) = mpsc::channel::<EngineEvent>(64);

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

    let event_forward_task = spawn_engine_event_forward_task(
        engine_rx,
        outbound_tx.clone(),
        session_id.clone(),
        state.workspace_runs.clone(),
    );

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
    let inbound_context = WorkspaceInboundContext {
        engine: engine.clone(),
        run_context: run_context.clone(),
        outbound_tx: outbound_tx.clone(),
        current_run: current_run.clone(),
        workspace_runs: state.workspace_runs.clone(),
        session_id: session_id.clone(),
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

    let should_resume_outline_run = {
        let engine = engine.lock().await;
        engine.session().workspace_type == WorkspaceType::WorkItemPlan
            && engine.session().stage == WorkspaceStage::Running
            && engine.active_node_type()
                == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemPlanOutlineRun)
            && engine.active_run_id().is_none()
    };
    if should_resume_outline_run
        && state.workspace_runs.run(&session_id).await.is_none()
        && let Err(message) = spawn_provider_run_from_handler(
            run_context.clone(),
            ProviderRunKind::WorkItemPlanAuthor,
            outbound_tx.clone(),
        )
        .await
    {
        let err = WsOutMessage::Error { message };
        let _ = send_json_outbound(&outbound_tx, &err).await;
    }

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

        handle_workspace_inbound_message(inbound_context.clone(), in_msg).await;
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
