use super::*;
use crate::product::logical_codebase::{
    PlanningContextResolver, RepositoryRouting, RepositoryRoutingErrorCode, ResumeDecision,
    resolve_issue_logical_codebase_id,
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

/// 逻辑代码库分支的规划会话 resume 校验入口（Task 11 / REQ-PLN-03）。
///
/// 在 provider 启动前校验 planning snapshot 指纹：
/// - 传统单仓路径（无 logical manifest 且 issue 无 selection）→ `Ok(None)`，不受影响。
/// - 逻辑代码库路径（manifest 与 selection 成对）→ 校验 planning snapshot 指纹。
/// - manifest 与 selection 不成对 → fail-closed，拒绝恢复。
/// - 指纹一致 → `SameContext`：沿用现有 session 审计与 prompt 上下文。
/// - 指纹漂移 → `StaleContext`：不得沿用可能过时/越权的 prompt/cwd/policy，调用方应
///   启动新会话并重建上下文（重新 build）。
pub(crate) async fn planning_resume_decision_with_fresh_index(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> Result<Option<ResumeDecision>, String> {
    let routing = RepositoryRouting::load_for_issue(app_paths, project_id, issue_id)
        .map_err(|error| format!("load repository routing failed: {error}"))?;
    match routing {
        RepositoryRouting::Legacy { .. } => Ok(None),
        RepositoryRouting::Logical { .. } => {
            #[cfg(test)]
            let resolver = PlanningContextResolver::new_without_freshness(app_paths.clone());
            #[cfg(not(test))]
            let resolver = PlanningContextResolver::new(app_paths.clone());
            let decision = resolver
                .resume_with_fresh_index(project_id, issue_id)
                .await
                .map_err(|error| format!("planning context resume failed: {error}"))?;
            Ok(Some(decision))
        }
        RepositoryRouting::FailClosed { code, reason } => {
            Err(planning_resume_routing_error(code, &reason))
        }
    }
}

/// 将 repository routing 的 fail-closed 分类转换为稳定错误码，保持 resume 与 compile
/// 对不成对 manifest/selection 状态的拒绝语义一致。
fn planning_resume_routing_error(code: RepositoryRoutingErrorCode, reason: &str) -> String {
    let stable_code = match code {
        RepositoryRoutingErrorCode::TargetMissing => "repository_routing_target_missing",
        RepositoryRoutingErrorCode::OrphanedSelection
        | RepositoryRoutingErrorCode::Inconsistent
        | RepositoryRoutingErrorCode::MemberRemoved
        | RepositoryRoutingErrorCode::SelectionInvalidated => "repository_routing_inconsistent",
        RepositoryRoutingErrorCode::TargetUnknown => "repository_routing_target_unknown",
        RepositoryRoutingErrorCode::TargetAmbiguous => "repository_routing_ambiguous",
    };
    format!("{stable_code}: {reason}")
}

/// 依据 planning resume 决策决定实际启动的 run kind（B3 修复）。
///
/// - `None`（传统单仓）/ `SameContext`：沿用 `fallback` run kind 续跑原中断 run。
/// - `StaleContext`：重建 —— 强制全新 `WorkItemPlanOutlineRebuild`（携带 rebuilt 规划
///   上下文：新建 OutlineRun 节点、使用 rebuilt cwd/inventory/policy），不使用可能复用
///   旧 provider 会话/prompt 内容的 revision run kind，也不沿用中断 OutlineRun 节点。
pub(crate) fn planning_resume_run_kind(
    decision: &Option<ResumeDecision>,
    fallback: ProviderRunKind,
) -> ProviderRunKind {
    match decision {
        None | Some(ResumeDecision::SameContext(_)) => fallback,
        Some(ResumeDecision::StaleContext { rebuilt, .. }) => {
            ProviderRunKind::WorkItemPlanOutlineRebuild {
                rebuilt: Box::new(rebuilt.clone()),
            }
        }
    }
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
        match ensure_workspace_context_message(&app_paths, &lifecycle, session_record).await {
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

    let is_logical_session = repository.logical_repository_id.is_some();

    let mut session = WorkspaceSession::from_record(session_record.clone());
    session.repository_path = Some(repository.path);
    if let Ok(checkpoints) = checkpoint_store.list_checkpoints(&session.session_id) {
        session.restore_checkpoint_ids(&checkpoints);
    }
    let mut engine_workspace =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle, engine_tx, session);
    if is_logical_session {
        let lc_id = match resolve_issue_logical_codebase_id(
            &app_paths,
            &session_record.project_id,
            &session_record.issue_id,
        ) {
            Ok(lc_id) => lc_id,
            Err(error) => {
                let err = WsOutMessage::Error {
                    message: format!("logical codebase resolution failed: {error}"),
                };
                if let Ok(json) = serde_json::to_string(&err) {
                    let _ = ws_sender.send(Message::Text(json.into())).await;
                }
                return;
            }
        };
        let gateway = match state.gateway_factory() {
            Some(factory) => {
                match factory.build_for_lc(&session_record.project_id, lc_id.as_deref()) {
                    Ok(gateway) => gateway,
                    Err(error) => {
                        let err = WsOutMessage::Error {
                            message: format!("logical gateway build failed: {error}"),
                        };
                        if let Ok(json) = serde_json::to_string(&err) {
                            let _ = ws_sender.send(Message::Text(json.into())).await;
                        }
                        return;
                    }
                }
            }
            None => {
                let err = WsOutMessage::Error {
                    message: "logical gateway factory unavailable".to_string(),
                };
                if let Ok(json) = serde_json::to_string(&err) {
                    let _ = ws_sender.send(Message::Text(json.into())).await;
                }
                return;
            }
        };
        engine_workspace = engine_workspace.with_logical_provider_gateway(Arc::new(gateway));
    }
    let engine = Arc::new(Mutex::new(engine_workspace));

    let (session_state, restored_choice_request) = {
        let mut engine = engine.lock().await;
        if let Err(error) = engine.ensure_plan_repair_artifacts().await {
            let err = WsOutMessage::Error {
                message: format!("plan repair artifact bootstrap failed: {error:?}"),
            };
            if let Ok(json) = serde_json::to_string(&err) {
                let _ = ws_sender.send(Message::Text(json.into())).await;
            }
            return;
        }
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
        app_state: state.clone(),
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

    let outline_resume_kind: Result<Option<ProviderRunKind>, String> = {
        let engine = engine.lock().await;
        if let Some(error) = engine.outline_revision_recovery_error() {
            Err(format!("outline revision recovery failed: {error}"))
        } else {
            let should_resume = engine.session().workspace_type == WorkspaceType::WorkItemPlan
                && engine.session().stage == WorkspaceStage::Running
                && engine.active_node_type()
                    == Some(
                        crate::web::workspace_ws_types::TimelineNodeType::WorkItemPlanOutlineRun,
                    )
                && engine.active_run_id().is_none();
            if !should_resume {
                Ok(None)
            } else if let Some(node_id) = engine.active_timeline_node_id() {
                match LifecycleStore::new(app_paths.clone()).load_node_detail(&session_id, &node_id)
                {
                    Ok(detail) => Ok(Some(if detail.is_revision {
                        ProviderRunKind::WorkItemPlanOutlineRevision {
                            feedback: detail.revision_feedback,
                        }
                    } else {
                        ProviderRunKind::WorkItemPlanAuthor
                    })),
                    Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => {
                        Ok(Some(ProviderRunKind::WorkItemPlanAuthor))
                    }
                    Err(error) => Err(format!(
                        "resume outline run detail failed for {node_id}: {error}"
                    )),
                }
            } else {
                Err("resume outline run detail failed: active node id unavailable".to_string())
            }
        }
    };
    match outline_resume_kind {
        Ok(Some(run_kind)) if state.workspace_runs.run(&session_id).await.is_none() => {
            // 逻辑代码库分支：resume 校验在 provider 启动前完成（REQ-PLN-03）。
            // - None（传统单仓）/ SameContext（指纹一致）：沿用现有 session 审计与
            //   prompt 上下文，照常续跑。
            // - StaleContext（指纹漂移）：以 `WorkItemPlanOutlineRebuild` 真正启动全新
            //   run —— 使用 rebuilt cwd/inventory/policy、新建 OutlineRun 节点，不沿用
            //   中断会话节点/旧内容。rebuilt snapshot 不在启动前落盘，而是由 run 在
            //   provider 成功启动后才 commit（新 BLOCKER 修复：provider 失败不落盘，
            //   重连仍 StaleContext）。
            // - 校验失败：fail-closed 拒绝续跑。
            match planning_resume_decision_with_fresh_index(
                &app_paths,
                &session_record.project_id,
                &session_record.issue_id,
            )
            .await
            {
                Ok(decision) => {
                    let run_kind = planning_resume_run_kind(&decision, run_kind);
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
                    let err = WsOutMessage::Error {
                        message: format!("planning resume check failed: {message}"),
                    };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
            }
        }
        Err(message) => {
            let err = WsOutMessage::Error { message };
            let _ = send_json_outbound(&outbound_tx, &err).await;
        }
        Ok(Some(_)) | Ok(None) => {}
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

        let stage_type_and_cancel_replay = if requires_stage_validation(&in_msg) {
            Some({
                let engine = engine.lock().await;
                let completed_cancel_replay = matches!(
                    &in_msg,
                    WsInMessage::CancelPlanAmendment { amendment_id, .. }
                        if engine.current_stage() == WorkspaceStage::Completed
                            && engine.is_cancelled_plan_amendment_replay(amendment_id)
                );
                (
                    engine.current_stage(),
                    engine.session().workspace_type.clone(),
                    completed_cancel_replay,
                )
            })
        } else {
            None
        };
        if let Some((stage, workspace_type, completed_cancel_replay)) =
            stage_type_and_cancel_replay.as_ref()
            && !is_message_valid_for_stage(&in_msg, stage)
            && !completed_cancel_replay
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
