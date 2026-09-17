use super::*;
use crate::product::logical_codebase::{
    PlanningContextResolver, RepositoryRouting, RepositoryRoutingErrorCode, ResumeDecision,
};
use crate::web::workspace_session::WorkspaceSessionManager;

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

#[derive(Clone, Copy)]
pub(crate) struct ActivityTimestamp {
    pub(crate) instant: tokio::time::Instant,
    pub(crate) recorded_at: chrono::DateTime<chrono::Utc>,
}

impl ActivityTimestamp {
    pub(crate) fn now() -> Self {
        Self {
            instant: tokio::time::Instant::now(),
            recorded_at: chrono::Utc::now(),
        }
    }
}

#[derive(Debug)]
enum ReceiverExit {
    CloseFrame(Option<CloseFrame>),
    Eof,
    ReadError(String),
}

impl ReceiverExit {
    fn kind(&self, idle_timeout_triggered: bool) -> &'static str {
        if idle_timeout_triggered {
            "server_idle"
        } else {
            match self {
                Self::CloseFrame(_) => "close_frame",
                Self::Eof => "eof",
                Self::ReadError(error) => {
                    let _ = error;
                    "read_error"
                }
            }
        }
    }

    fn close_frame(&self) -> Option<&CloseFrame> {
        match self {
            Self::CloseFrame(frame) => frame.as_ref(),
            Self::Eof | Self::ReadError(_) => None,
        }
    }
}

#[derive(serde::Serialize)]
struct ConnectionDiagnostic {
    connection_id: String,
    session_id: String,
    role: String,
    receiver_exit: String,
    idle_timeout_triggered: bool,
    close_code: Option<u16>,
    close_reason: Option<String>,
    current_run_id: Option<u64>,
    current_run_token: Option<u64>,
    provider_drive_depth: u32,
    last_client_activity_at: String,
    last_server_activity_at: String,
    recorded_at: String,
}

impl WsOutMessage {
    pub(crate) fn with_connection_id(mut self, connection_id: &str) -> Self {
        if let Self::SessionState {
            connection_id: slot,
            ..
        } = &mut self
        {
            *slot = Some(connection_id.to_string());
        }
        self
    }
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
fn decorate_session_state_json(message: String, connection_id: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&message) else {
        return message;
    };
    if value.get("type").and_then(serde_json::Value::as_str) != Some("session_state") {
        return message;
    }
    value["connection_id"] = serde_json::Value::String(connection_id.to_string());
    serde_json::to_string(&value).unwrap_or(message)
}

async fn forward_connection_outbound_controls(
    mut outbound_rx: mpsc::Receiver<OutboundControl>,
    socket_outbound_tx: mpsc::Sender<OutboundControl>,
    connection_id: String,
) {
    while let Some(control) = outbound_rx.recv().await {
        let control = match control {
            OutboundControl::Text(message) => {
                OutboundControl::Text(decorate_session_state_json(message, &connection_id))
            }
            other => other,
        };
        if socket_outbound_tx.send(control).await.is_err() {
            break;
        }
    }
}

/// 出站写泵：socket 侧唯一的出站写入点。
pub(crate) async fn pump_outbound_controls<S>(
    mut outbound_rx: mpsc::Receiver<OutboundControl>,
    mut ws_sender: S,
    last_server_activity_at: Arc<Mutex<ActivityTimestamp>>,
) where
    S: futures_util::Sink<Message> + Unpin + Send + 'static,
    S::Error: Send + 'static,
{
    while let Some(control) = outbound_rx.recv().await {
        match control {
            OutboundControl::Text(msg) => {
                if ws_sender.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
                *last_server_activity_at.lock().await = ActivityTimestamp::now();
            }
            OutboundControl::CloseDueToIdleTimeout => {
                let _ = ws_sender.send(Message::Close(None)).await;
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
}

pub(crate) fn spawn_idle_timeout_task(
    last_client_activity_at: Arc<Mutex<ActivityTimestamp>>,
    last_server_activity_at: Arc<Mutex<ActivityTimestamp>>,
    idle_timeout_triggered: Arc<std::sync::atomic::AtomicBool>,
    outbound_tx: mpsc::Sender<OutboundControl>,
    is_active_run: Arc<dyn Fn() -> bool + Send + Sync>,
    timeout_after: std::time::Duration,
    tick_every: std::time::Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick_every);
        loop {
            interval.tick().await;
            let last_client_activity = last_client_activity_at.lock().await.instant;
            let last_server_activity = last_server_activity_at.lock().await.instant;
            let last_activity = last_client_activity.max(last_server_activity);
            if last_activity.elapsed() > timeout_after && !is_active_run() {
                idle_timeout_triggered.store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = outbound_tx
                    .send(OutboundControl::CloseDueToIdleTimeout)
                    .await;
                break;
            }
        }
    })
}

/// idle 关闭守卫：manager 的 active run 与独立 provider drive 均阻止主动关闭。
pub(crate) fn workspace_idle_activity_guard(
    manager: Arc<WorkspaceSessionManager>,
    workspace_runs: WorkspaceRunRegistry,
    session_id: String,
) -> Arc<dyn Fn() -> bool + Send + Sync> {
    Arc::new(move || {
        manager.is_active_run() || workspace_runs.provider_drive_in_progress(&session_id)
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
    // SingleCandidate 已在 durable run kind 中固化 markdown 链路；逻辑 planning
    // snapshot 漂移不能将其静默改写为 legacy OutlineRebuild。
    if fallback.is_single_candidate_work_item_plan_author() {
        return fallback;
    }
    match decision {
        None | Some(ResumeDecision::SameContext(_)) => fallback,
        Some(ResumeDecision::StaleContext { rebuilt, .. }) => {
            ProviderRunKind::WorkItemPlanOutlineRebuild {
                rebuilt: Box::new(rebuilt.clone()),
            }
        }
    }
}

/// Parse one websocket inbound message without discarding unknown top-level fields.
///
/// `WsInMessage` intentionally remains the typed business payload. The envelope records
/// only field names (never client values) so protocol handlers can reject fields that are
/// forbidden for a durable flow before any state mutation occurs.
pub(crate) fn single_candidate_generation_decision_bypasses_stage_validation(
    flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind,
    message: &WsInMessage,
) -> bool {
    flow_kind == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
        && matches!(
            message,
            WsInMessage::SelectWorkItemGenerationMode { .. }
                | WsInMessage::WorkItemDraftDecision { .. }
                | WsInMessage::WorkItemBatchDecision { .. }
        )
}

pub(crate) fn parse_workspace_inbound_text(
    text: &str,
) -> Result<WorkspaceInboundEnvelope, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    let submitted_fields = value
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default();
    let message = serde_json::from_value(value)?;
    Ok(WorkspaceInboundEnvelope {
        message,
        submitted_fields,
    })
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
    use crate::web::workspace_ws_types::{
        WorkItemBatchDecisionDto, WorkItemDraftDecisionDto, WorkItemGenerationModeDto,
    };

    #[test]
    fn single_candidate_legacy_generation_decisions_bypass_stage_validation_only_for_precise_rejection()
     {
        let messages = [
            WsInMessage::SelectWorkItemGenerationMode {
                mode: WorkItemGenerationModeDto::Serial,
            },
            WsInMessage::WorkItemDraftDecision {
                outline_id: "outline_client_supplied".to_string(),
                decision: WorkItemDraftDecisionDto::Accept,
                feedback: None,
            },
            WsInMessage::WorkItemBatchDecision {
                decision: WorkItemBatchDecisionDto::AcceptAll,
                feedback: None,
                first_affected_outline_id: None,
            },
        ];

        for message in messages {
            assert!(requires_stage_validation(&message));
            assert!(
                single_candidate_generation_decision_bypasses_stage_validation(
                    WorkItemPlanFlowKind::SingleCandidate,
                    &message,
                )
            );
            assert!(
                !single_candidate_generation_decision_bypasses_stage_validation(
                    WorkItemPlanFlowKind::Legacy,
                    &message,
                )
            );
        }
        assert!(
            !single_candidate_generation_decision_bypasses_stage_validation(
                WorkItemPlanFlowKind::SingleCandidate,
                &WsInMessage::RequestOutlineRevision { feedback: None },
            )
        );
    }

    #[tokio::test]
    async fn workspace_idle_activity_guard_covers_active_run_and_provider_drive() {
        let registry = WorkspaceRunRegistry::default();
        let manager = WorkspaceSessionManager::test_fixture("session_a");
        let guard = workspace_idle_activity_guard(
            manager.clone(),
            registry.clone(),
            "session_a".to_string(),
        );
        assert!(
            !guard(),
            "无 active run 且无 provider drive：允许 idle 回收"
        );

        let drive = registry.begin_provider_drive("session_a");
        assert!(
            guard(),
            "provider drive 进行中（含断连后跨 socket 存活的 run）：不得主动关连接"
        );
        drop(drive);
        assert!(!guard(), "drive 结束后恢复 idle 回收");

        let _run = manager
            .start_run(ProviderRunKind::ReviewOnly, None)
            .await
            .expect("active run");
        assert!(guard(), "manager active run 进行中：不得主动关连接");
    }

    #[tokio::test]
    async fn cancelled_initial_flush_still_activates_attachment() {
        let manager = WorkspaceSessionManager::test_fixture("session_initial_flush");
        let (outbound_tx, mut outbound_rx) = mpsc::channel(1);
        manager.register_attachment("connection", outbound_tx.clone());
        outbound_tx
            .send(OutboundControl::Text("occupied".to_string()))
            .await
            .expect("occupy outbound channel");

        let manager_for_flush = manager.clone();
        let outbound_for_flush = outbound_tx.clone();
        let initial_flush = tokio::spawn(async move {
            flush_initial_attachment(
                manager_for_flush.as_ref(),
                "connection",
                &outbound_for_flush,
                Some(("baseline".to_string(), None)),
            )
            .await;
        });
        tokio::task::yield_now().await;
        initial_flush.abort();
        let _ = initial_flush.await;

        assert!(
            matches!(outbound_rx.recv().await, Some(OutboundControl::Text(text)) if text == "occupied")
        );
        manager
            .broadcast_test_event(WsProviderStatus::Running)
            .await;
        assert!(
            matches!(outbound_rx.recv().await, Some(OutboundControl::Text(text)) if text.contains("provider_status")),
            "取消初始帧写入后连接仍必须接收直播帧"
        );
    }
}

async fn flush_initial_attachment(
    manager: &WorkspaceSessionManager,
    connection_id: &str,
    outbound_tx: &mpsc::Sender<OutboundControl>,
    initial: Option<(String, Option<WsOutMessage>)>,
) {
    struct ActivationGuard<'a> {
        manager: &'a WorkspaceSessionManager,
        connection_id: &'a str,
    }

    impl Drop for ActivationGuard<'_> {
        fn drop(&mut self) {
            self.manager.activate_attachment(self.connection_id);
        }
    }

    let _activation_guard = ActivationGuard {
        manager,
        connection_id,
    };
    if let Some((session_state, choice)) = initial {
        let _ = outbound_tx.send(OutboundControl::Text(session_state)).await;
        if let Some(choice) = choice
            && let Ok(json) = serde_json::to_string(&choice)
        {
            let _ = outbound_tx.send(OutboundControl::Text(json)).await;
        }
    }
}

pub(crate) async fn handle_workspace_socket(
    socket: WebSocket,
    session_id: String,
    state: WebAppState,
) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let connection_id = uuid::Uuid::new_v4().to_string();

    let (outbound_tx, connection_outbound_rx) = mpsc::channel::<OutboundControl>(64);
    let manager = match state
        .workspace_sessions
        .get_or_create_and_attach(&session_id, &connection_id, outbound_tx.clone(), || {
            WorkspaceSessionManager::create(&state, &session_id)
        })
        .await
    {
        Ok(manager) => manager,
        Err(message) => {
            let err = WsOutMessage::Error { message };
            if let Ok(json) = serde_json::to_string(&err) {
                let _ = ws_sender.send(Message::Text(json.into())).await;
            }
            return;
        }
    };
    let session_record = manager.session_record.clone();
    let engine = manager.engine();

    let (socket_outbound_tx, outbound_rx) = mpsc::channel::<OutboundControl>(64);
    let connection_outbound_task = tokio::spawn(forward_connection_outbound_controls(
        connection_outbound_rx,
        socket_outbound_tx,
        connection_id.clone(),
    ));
    let (session_state, restored_choice_request) = manager.attached_session_state().await;
    let pending_initial = Arc::new(Mutex::new(
        manager
            .serialize_attach_session_state(session_state.with_connection_id(&connection_id))
            .map(|session_state| (session_state, restored_choice_request)),
    ));

    let (socket_control_tx, mut socket_control_rx) = mpsc::channel::<WorkspaceSocketControl>(4);
    state
        .test_controls
        .register_workspace_socket(session_id.clone(), socket_control_tx)
        .await;
    // 客户端入站和服务器成功出站由独立时钟记录；idle 仅在双方都静默时触发。
    let last_client_activity_at = Arc::new(Mutex::new(ActivityTimestamp::now()));
    let last_server_activity_at = Arc::new(Mutex::new(ActivityTimestamp::now()));
    let idle_timeout_triggered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let send_task = tokio::spawn(pump_outbound_controls(
        outbound_rx,
        ws_sender,
        last_server_activity_at.clone(),
    ));

    let outbound_for_socket_controls = outbound_tx.clone();
    let socket_control_task = tokio::spawn(async move {
        if let Some(WorkspaceSocketControl::CloseForTestDrop) = socket_control_rx.recv().await {
            let _ = outbound_for_socket_controls
                .send(OutboundControl::CloseForTestDrop)
                .await;
        }
    });

    let mut run_context = manager.provider_run_context(state.workspace_runs.clone());
    run_context.connection_id = Some(connection_id.clone());
    run_context.lease_epoch = manager.lease_epoch_for_connection(&connection_id);
    let inbound_context = WorkspaceInboundContext {
        app_state: state.clone(),
        engine: engine.clone(),
        run_context: run_context.clone(),
        outbound_tx: outbound_tx.clone(),
        session_id: session_id.clone(),
    };
    let configured_idle_timeout = state.test_controls.server_idle_timeout();
    let idle_timeout_task = spawn_idle_timeout_task(
        last_client_activity_at.clone(),
        last_server_activity_at.clone(),
        idle_timeout_triggered.clone(),
        outbound_tx.clone(),
        workspace_idle_activity_guard(
            manager.clone(),
            state.workspace_runs.clone(),
            session_id.clone(),
        ),
        configured_idle_timeout,
        std::time::Duration::from_secs(5),
    );
    let idle_receiver_exit = idle_timeout_triggered.clone();
    let initial_for_inbound = pending_initial.clone();
    let manager_for_grace = manager.clone();
    let connection_for_grace = connection_id.clone();
    let initial_for_grace = pending_initial;
    let outbound_for_grace = outbound_tx.clone();
    let initial_push_grace_task = tokio::spawn(async move {
        tokio::time::sleep(
            if configured_idle_timeout < std::time::Duration::from_millis(500) {
                std::time::Duration::ZERO
            } else {
                std::time::Duration::from_millis(500)
            },
        )
        .await;
        let initial = initial_for_grace.lock().await.take();
        if initial.is_some() {
            eprintln!(
                "[aria-cursor-resubscribe] initial attach grace elapsed; sending baseline before live registration"
            );
        }
        flush_initial_attachment(
            manager_for_grace.as_ref(),
            &connection_for_grace,
            &outbound_for_grace,
            initial,
        )
        .await;
    });

    let receiver_exit = tokio::select! {
        receiver_exit = async {
            loop {
                let msg = match ws_receiver.next().await {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => {
                        if error
                            .to_string()
                            .contains("reset without closing handshake")
                        {
                            break ReceiverExit::Eof;
                        }
                        break ReceiverExit::ReadError(error.to_string());
                    }
                    None => break ReceiverExit::Eof,
                };
                let text = match msg {
                    Message::Text(text) => text.to_string(),
                    Message::Close(frame) => break ReceiverExit::CloseFrame(frame),
                    _ => continue,
                };

                let envelope = match parse_workspace_inbound_text(&text) {
                    Ok(envelope) => {
                        *last_client_activity_at.lock().await = ActivityTimestamp::now();
                        envelope
                    }
                    Err(e) => {
                        let err = WsOutMessage::Error {
                            message: format!("invalid message: {e}"),
                        };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                        continue;
                    }
                };
                let in_msg = &envelope.message;
                let is_cursor_hello = matches!(
                    in_msg,
                    WsInMessage::Hello {
                        after_event_seq: Some(_),
                        ..
                    }
                );
                let initial = initial_for_inbound.lock().await.take();
                if is_cursor_hello {
                    if initial.is_none() {
                        eprintln!("[aria-cursor-resubscribe] cursor Hello arrived after initial attach grace; client cursor retry absorbs disclosed narrow window");
                    }
                    manager.activate_attachment(&connection_id);
                } else {
                    flush_initial_attachment(
                        manager.as_ref(),
                        &connection_id,
                        &outbound_tx,
                        initial,
                    )
                    .await;
                }
                initial_push_grace_task.abort();
                if let Err(role) = manager.arbitrate(&connection_id, in_msg) {
                    let stale_driver = role == crate::web::workspace_session::ConnectionRole::Driver;
                    let err = WsOutMessage::ProtocolError {
                        code: if stale_driver {
                            "STALE_DRIVER_LEASE".to_string()
                        } else {
                            "OBSERVER_WRITE_REJECTED".to_string()
                        },
                        message: if stale_driver {
                            format!(
                                "driver connection no longer holds the lease for write message {}",
                                message_type(in_msg)
                            )
                        } else {
                            format!(
                                "observer connection cannot send write message {}",
                                message_type(in_msg)
                            )
                        },
                        context: Some(serde_json::json!({
                            "role": role.as_str(),
                            "received": message_type(in_msg),
                        })),
                    };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                    continue;
                }

                let stage_type_and_cancel_replay = if requires_stage_validation(in_msg)
                    && !single_candidate_generation_decision_bypasses_stage_validation(
                        session_record.flow_kind,
                        in_msg,
                    ) {
                    Some({
                        let engine = engine.lock().await;
                        let completed_cancel_replay = matches!(
                            in_msg,
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
                    && !is_message_valid_for_stage_with_flow(session_record.flow_kind, in_msg, stage)
                    && !completed_cancel_replay
                    && !(matches!(in_msg, WsInMessage::RequestRevision { .. })
                        && *stage == WorkspaceStage::AuthorConfirm
                        && *workspace_type == WorkspaceType::WorkItemPlan)
                {
                    let err = if let Some(err) = human_gate_message_boundary_error(
                        session_record.flow_kind,
                        stage.clone(),
                        in_msg,
                    ) {
                        err
                    } else {
                        match in_msg {
                            WsInMessage::HumanGateFeedback { .. } => conversational_gate_stage_error(
                                session_record.flow_kind,
                                stage,
                                in_msg,
                            ),
                            WsInMessage::Advance { command_id } => advance_stage_error(
                                command_id.clone(),
                                stage,
                                session_record.flow_kind,
                            ),
                            _ => WsOutMessage::ProtocolError {
                                code: "INVALID_MESSAGE_FOR_STAGE".to_string(),
                                message: format!(
                                    "message {} not allowed in stage {}",
                                    message_type(in_msg),
                                    stage.as_str()
                                ),
                                context: Some(serde_json::json!({
                                    "stage": stage.as_str(),
                                    "received": message_type(in_msg),
                                })),
                            },
                        }
                    };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                    continue;
                }

                handle_workspace_inbound_message(inbound_context.clone(), envelope).await;
            }
        } => receiver_exit,
        _ = async {
            loop {
                if idle_receiver_exit.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        } => ReceiverExit::Eof,
    };

    let idle_timeout_triggered = idle_timeout_triggered.load(std::sync::atomic::Ordering::SeqCst);
    let active_for_diagnostic = manager.active_run().await;
    let close_frame = receiver_exit.close_frame();
    let last_client_activity_at = last_client_activity_at.lock().await.recorded_at;
    let last_server_activity_at = last_server_activity_at.lock().await.recorded_at;
    let diagnostic = ConnectionDiagnostic {
        connection_id: connection_id.clone(),
        session_id: session_id.clone(),
        role: manager.connection_role(&connection_id).as_str().to_string(),
        receiver_exit: receiver_exit.kind(idle_timeout_triggered).to_string(),
        idle_timeout_triggered,
        close_code: close_frame.map(|frame| frame.code),
        close_reason: close_frame.map(|frame| frame.reason.to_string()),
        current_run_id: active_for_diagnostic.as_ref().map(|run| run.id),
        current_run_token: active_for_diagnostic.as_ref().map(|run| run.token),
        provider_drive_depth: state.workspace_runs.provider_drive_depth(&session_id),
        last_client_activity_at: last_client_activity_at.to_rfc3339(),
        last_server_activity_at: last_server_activity_at.to_rfc3339(),
        recorded_at: chrono::Utc::now().to_rfc3339(),
    };
    let diagnostic_json =
        serde_json::to_value(&diagnostic).expect("connection diagnostic serializes");
    eprintln!("[aria-connection-diagnostic] {diagnostic_json}");
    state
        .test_controls
        .record_connection_diagnostic(&session_id, diagnostic_json)
        .await;

    manager.handle_connection_closed(&connection_id).await;
    drop(outbound_tx);
    connection_outbound_task.abort();
    idle_timeout_task.abort();
    socket_control_task.abort();
    initial_push_grace_task.abort();
    send_task.abort();
    let _ = connection_outbound_task.await;
    let _ = socket_control_task.await;
    let _ = send_task.await;
}
