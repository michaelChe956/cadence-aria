use super::*;
use crate::product::lifecycle_store::LcFreshClaimOutcome;
use crate::product::logical_codebase::{
    PlanningContextResolver, RepositoryRouting, RepositoryRoutingErrorCode, ResumeDecision,
};
use crate::product::models::{
    LcRecoveryArtifactRef, LcRecoveryRunContext, LcResumeRecoveryRecord, LcResumeRecoveryState,
};
use crate::web::workspace_session::WorkspaceSessionManager;
use crate::web::workspace_ws_types::{
    ArtifactPayload, LcResumeRecoveryDelivery, LcResumeRecoveryIntent, ProviderConfigSnapshot,
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

/// B2a（Task 5.1，tasks.md 5.1 / 设计 §4.3）：planning resume 决策的启动归类。
#[derive(Debug, Clone)]
pub(crate) enum PlanningResumeLaunchDecision {
    /// 沿用原 run kind 续跑（同 native resume 链路，行为与 B3 前一致）。
    ResumeExisting(ProviderRunKind),
    /// 漂移等待：0 自动 start——落 `planning_context_stale` durable 等待
    ///（run_context 捕获 feedback 在任何 take 前固化），重建仅显式 fresh
    /// 授权后（B2a dispatch 面，`dispatch_lc_fresh_recovery_run`）。
    SupersededWaiting,
}

/// B2a（Task 5.1）：原 B3 `planning_resume_run_kind` 的 Stale 自动 spawn
/// clean-cutover。
/// - `None`（传统单仓）/ `SameContext`：`ResumeExisting` 沿用 fallback 续跑。
/// - `StaleContext`：`SupersededWaiting`——B3 旧行为（自动转
///   `WorkItemPlanOutlineRebuild` spawn）删除，漂移不再自动启动任何
///   provider run，由 manager 恢复入口捕获 run_context 落等待。
/// - SC author 透传保留：planning snapshot 漂移不得将其静默改写为 legacy
///   OutlineRebuild（SC stale 等待语义归 B2c/5.5 接管）。
pub(crate) fn planning_resume_launch_decision(
    decision: &Option<ResumeDecision>,
    fallback: ProviderRunKind,
) -> PlanningResumeLaunchDecision {
    // SingleCandidate 已在 durable run kind 中固化 markdown 链路；逻辑
    // planning snapshot 漂移不能将其静默改写为 legacy OutlineRebuild。
    if fallback.is_single_candidate_work_item_plan_author() {
        return PlanningResumeLaunchDecision::ResumeExisting(fallback);
    }
    match decision {
        None | Some(ResumeDecision::SameContext(_)) => {
            PlanningResumeLaunchDecision::ResumeExisting(fallback)
        }
        Some(ResumeDecision::StaleContext { .. }) => {
            PlanningResumeLaunchDecision::SupersededWaiting
        }
    }
}

/// Parse one websocket inbound message without discarding unknown top-level fields.
///
/// `WsInMessage` intentionally remains the typed business payload. The envelope records
/// only field names (never client values) so protocol handlers can reject fields that are
/// forbidden for a durable flow before any state mutation occurs.
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
    // 退役留档（T5/REQ-RET-02）：原「SC 精确拒绝 legacy 逐段决策」单测与
    // `single_candidate_generation_decision_bypasses_stage_validation` 辅助随
    // wire 变体删除退役——逐段决策消息在 parse 面即被拒收（retired 消息
    // protocol error 承接，见 protocol.rs tests 与 it_core part_07），不再有
    // stage 校验旁路需求。

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
                Some(InitialAttach {
                    session_state: WsOutMessage::Pong,
                    choice: None,
                    attach_seq: 0,
                }),
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

    // ------------------------------------------------------------------
    // B1b（Task 4.2，tasks.md 4.2 / 设计 §5.1）：fresh 协议 Replay 与真实
    // 发送闭包。同 ID 三态 typed Replay（engine 持锁零等待）、显式 fresh 单
    // claim（零 spawn、workspace 保留、旧 audit/native 不复活）、claim 前置
    // 三负例（空 operation_id / 配置漂移 / recovery 替换或 source hash 变化）
    // 与 RequestRevision 活动等待拒收。
    // ------------------------------------------------------------------

    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::product::lifecycle_store::{CreateStorySpecInput, CreateWorkspaceSessionInput};
    use crate::product::models::{
        LcFreshOperationResult, LcFreshOperationState, LcRecoveryArtifactRef,
        LcRecoveryBusinessContext, LcRecoveryRunContext, LcResumeRecoveryRecord,
        LcResumeRecoveryState, ProviderConversationRef, ProviderConversationRole,
        WorkspaceRolePermissionModes, WorkspaceType,
    };
    use crate::web::workspace_ws_types::{
        ArtifactVersion, LcResumeRecoveryIntentKind, ProviderConfigSnapshot, StructuredFeedback,
    };

    struct AnchorT12Fixture {
        root: tempfile::TempDir,
        app_paths: ProductAppPaths,
        lifecycle: LifecycleStore,
        record: WorkspaceSessionRecord,
    }

    fn anchor_t12_fixture() -> AnchorT12Fixture {
        let root = tempfile::tempdir().expect("app root");
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        crate::product::project_store::ProjectStore::new(app_paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "anchor t12 lc resume".to_string(),
                description: None,
            })
            .expect("create project");
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                title: "Anchor T12 Story".to_string(),
                aggregate_codebase: None,
            })
            .expect("story spec");
        let record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: story.id,
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("workspace session");
        AnchorT12Fixture {
            root,
            app_paths,
            lifecycle,
            record,
        }
    }

    fn anchor_t12_seed(
        fixture: &AnchorT12Fixture,
        state: LcResumeRecoveryState,
        operations: Vec<LcFreshOperationResult>,
        artifact_refs: Vec<LcRecoveryArtifactRef>,
    ) {
        let expected = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("expected record");
        let recovery = LcResumeRecoveryRecord {
            schema_version: 1,
            recovery_id: "recovery_anchor_t12".to_string(),
            failed_provider_node_id: "timeline_node_004".to_string(),
            reason_code: "resume_fingerprint_mismatch".to_string(),
            role: ProviderConversationRole::Reviewer,
            provider: ProviderName::Codex,
            superseded_native_session_id: Some("native-t12-old".to_string()),
            previous_fingerprint_digest: Some("sha256:t12".to_string()),
            previous_session_status: expected.status.clone(),
            previous_stage: "cross_review".to_string(),
            run_context: LcRecoveryRunContext::ReviewerRevision {
                business: LcRecoveryBusinessContext {
                    entity_id: fixture.record.entity_id.clone(),
                    plan_id: None,
                    workspace_type: WorkspaceType::Story,
                    business_repository_physical_id: None,
                    business_repository_logical_id: None,
                    business_checkout_id: None,
                    business_repository_path: None,
                    source_story_ids: Vec::new(),
                    source_design_ids: Vec::new(),
                    plan_options_ref: None,
                    artifact_refs,
                    review_refs: Vec::new(),
                    author_provider: fixture.record.author_provider.clone(),
                    effective_reviewer_provider: fixture.record.reviewer_provider.clone(),
                    original_node_id: Some("timeline_node_004".to_string()),
                    original_role_run_seq: None,
                },
                original_feedback: Some("原修订反馈".to_string()),
            },
            state,
            operation_id: operations
                .last()
                .map(|operation| operation.operation_id.clone()),
            fresh_node_id: None,
            fresh_native_session_id: None,
            fresh_role_run_seq: None,
            fresh_operations: operations,
            detected_at: "2026-10-11T00:00:00Z".to_string(),
            updated_at: "2026-10-11T00:00:00Z".to_string(),
        };
        fixture
            .lifecycle
            .compare_and_save_lc_resume_state(&expected, recovery)
            .expect("seed lc resume recovery");
    }

    fn anchor_t12_operation(
        operation_id: &str,
        state: LcFreshOperationState,
    ) -> LcFreshOperationResult {
        LcFreshOperationResult {
            operation_id: operation_id.to_string(),
            state,
            fresh_node_id: None,
            reason_code: None,
            role_run_seq: None,
            accepted_at: "2026-10-11T00:00:00Z".to_string(),
            updated_at: "2026-10-11T00:00:00Z".to_string(),
        }
    }

    fn anchor_t12_engine(
        fixture: &AnchorT12Fixture,
    ) -> (Arc<Mutex<WorkspaceEngine>>, mpsc::Receiver<EngineEvent>) {
        let record = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("latest record");
        let checkpoint_store = Arc::new(CheckpointStore::new(
            fixture.root.path().join("checkpoints"),
        ));
        let (engine_tx, engine_rx) = mpsc::channel::<EngineEvent>(64);
        let engine = WorkspaceEngine::new_persistent(
            checkpoint_store,
            fixture.lifecycle.clone(),
            engine_tx,
            WorkspaceSession::from_record(record),
        );
        (Arc::new(Mutex::new(engine)), engine_rx)
    }

    fn anchor_t12_run_context(
        fixture: &AnchorT12Fixture,
        engine: Arc<Mutex<WorkspaceEngine>>,
        workspace_runs: WorkspaceRunRegistry,
    ) -> ProviderRunContext {
        let record = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("latest record");
        ProviderRunContext::test_fixture(
            Arc::new(ProviderRegistry::new()),
            engine,
            workspace_runs,
            record.id.clone(),
            fixture.app_paths.clone(),
            record,
        )
    }

    fn anchor_t12_provider_config() -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
            permission_modes: WorkspaceRolePermissionModes::default(),
        }
    }

    fn anchor_t12_fresh_message(operation_id: &str, recovery_id: &str) -> WsInMessage {
        WsInMessage::StartGeneration {
            provider_config: anchor_t12_provider_config(),
            reviewer_enabled: true,
            resume_recovery: Some(LcResumeRecoveryIntent {
                recovery_id: recovery_id.to_string(),
                intent: LcResumeRecoveryIntentKind::Fresh,
                operation_id: operation_id.to_string(),
            }),
        }
    }

    fn anchor_t12_wire(frame: &WsOutMessage) -> serde_json::Value {
        serde_json::to_value(frame).expect("frame serializes")
    }

    fn anchor_t12_protocol_code(frame: &WsOutMessage) -> String {
        match frame {
            WsOutMessage::ProtocolError { code, .. } => code.clone(),
            other => panic!("expected protocol error, got {}", anchor_t12_wire(other)),
        }
    }

    fn anchor_t12_recovery_of(frame_record: &WorkspaceSessionRecord) -> LcResumeRecoveryRecord {
        frame_record
            .lc_resume_recovery
            .clone()
            .expect("recovery present")
    }

    #[tokio::test]
    async fn anchor_t12_wire_replay_running_resolved_and_failed_waiting() {
        let fixture = anchor_t12_fixture();
        let workspace_runs = WorkspaceRunRegistry::default();

        // 三态之 FreshRunning：同 ID op-1（Running）在 engine 锁被持有期间
        // 必须立即 typed Replay——不等待 engine mutex（store 层纯读），零 spawn。
        anchor_t12_seed(
            &fixture,
            LcResumeRecoveryState::FreshRunning,
            vec![anchor_t12_operation("op-1", LcFreshOperationState::Running)],
            Vec::new(),
        );
        let (engine, _engine_rx) = anchor_t12_engine(&fixture);
        let run_context = anchor_t12_run_context(&fixture, engine.clone(), workspace_runs.clone());
        let lock_guard = engine.lock().await;
        let running_replay = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            lc_resume_recovery_inbound_gate(
                &run_context,
                &anchor_t12_fresh_message("op-1", "recovery_anchor_t12"),
            )
        })
        .await
        .expect("同 ID Replay 不得等待 engine mutex（1s 内返回）")
        .expect("同 ID 已受理必须 Replay");
        drop(lock_guard);
        let response = anchor_t12_wire(&running_replay);
        assert_eq!(response["type"], "lc_resume_recovery_operation_result");
        assert_eq!(response["delivery"], "replayed");
        assert_eq!(response["operation_id"], "op-1");
        assert_eq!(response["operation_state"], "running");

        // 新 ID 撞 FreshRunning：typed busy（claim 前拒，不造第二受理）。
        let busy_error = lc_resume_recovery_inbound_gate(
            &run_context,
            &anchor_t12_fresh_message("op-2", "recovery_anchor_t12"),
        )
        .expect("新 ID 在 FreshRunning 必须被拒收");
        assert_eq!(
            anchor_t12_protocol_code(&busy_error),
            "lc_resume_recovery_busy"
        );

        // durable 事实不变：FreshRunning 保持，fresh_operations 仍只有 op-1。
        let after_running = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("record after running replay");
        let recovery = anchor_t12_recovery_of(&after_running);
        assert_eq!(recovery.state, LcResumeRecoveryState::FreshRunning);
        assert_eq!(recovery.fresh_operations.len(), 1);

        // 三态之 Resolved：同 ID op-1（Completed）只 Replay（不拒、不复活）；
        // 新 ID 拒第二会话（typed resolved stale）；普通 generation 门放行
        //（resolved 只保留证据，不拦后续新 native）。
        anchor_t12_seed(
            &fixture,
            LcResumeRecoveryState::Resolved,
            vec![anchor_t12_operation(
                "op-1",
                LcFreshOperationState::Completed,
            )],
            Vec::new(),
        );
        let resolved_replay = lc_resume_recovery_inbound_gate(
            &run_context,
            &anchor_t12_fresh_message("op-1", "recovery_anchor_t12"),
        )
        .expect("resolved 同 ID 仍必须 Replay（保留证据）");
        let resolved_response = anchor_t12_wire(&resolved_replay);
        assert_eq!(resolved_response["delivery"], "replayed");
        assert_eq!(resolved_response["operation_state"], "completed");
        let resolved_new_id = lc_resume_recovery_inbound_gate(
            &run_context,
            &anchor_t12_fresh_message("op-2", "recovery_anchor_t12"),
        )
        .expect("resolved 新 ID 必须被拒收（不造第二会话）");
        assert_eq!(
            anchor_t12_protocol_code(&resolved_new_id),
            "lc_resume_recovery_resolved"
        );
        let plain_generation = WsInMessage::StartGeneration {
            provider_config: anchor_t12_provider_config(),
            reviewer_enabled: true,
            resume_recovery: None,
        };
        assert!(
            lc_resume_recovery_inbound_gate(&run_context, &plain_generation).is_none(),
            "resolved 后普通 generation 不得被恢复门拦截"
        );
        let plain_request_revision_after_resolve = lc_resume_recovery_inbound_gate(
            &run_context,
            &WsInMessage::RequestRevision {
                feedback: StructuredFeedback {
                    feedback_types: vec!["clarification".to_string()],
                    description: "Resolved 后普通修订".to_string(),
                    target_artifact_version: None,
                },
            },
        )
        .is_none();
        assert!(
            plain_request_revision_after_resolve,
            "resolved 后普通 RequestRevision 不得被恢复门拦截（一次 fresh 即解锁，非永久锁）"
        );

        // 三态之 prepare 失败回 Waiting：同 ID op-1（NotStarted）只 Replay。
        anchor_t12_seed(
            &fixture,
            LcResumeRecoveryState::Waiting,
            vec![anchor_t12_operation(
                "op-1",
                LcFreshOperationState::NotStarted,
            )],
            Vec::new(),
        );
        let failed_waiting_replay = lc_resume_recovery_inbound_gate(
            &run_context,
            &anchor_t12_fresh_message("op-1", "recovery_anchor_t12"),
        )
        .expect("prepare 失败回 waiting 的同 ID 仍必须 Replay（不删 ID）");
        let waiting_response = anchor_t12_wire(&failed_waiting_replay);
        assert_eq!(waiting_response["delivery"], "replayed");
        assert_eq!(waiting_response["operation_state"], "not_started");

        // 普通旁路：RequestRevision 在活动 Waiting 期间必须被 typed 拒收。
        let plain_request_revision = lc_resume_recovery_inbound_gate(
            &run_context,
            &WsInMessage::RequestRevision {
                feedback: StructuredFeedback {
                    feedback_types: vec!["clarification".to_string()],
                    description: "普通旁路反馈".to_string(),
                    target_artifact_version: None,
                },
            },
        )
        .expect("活动恢复期间普通 RequestRevision 必须被拒收");
        assert_eq!(
            anchor_t12_protocol_code(&plain_request_revision),
            "LC_RESUME_RECOVERY_ACTIVE"
        );

        // 零 spawn：无 provider drive、无新 timeline 节点（Replay/拒收全程
        // 不进 engine/run 面产 run）。
        assert_eq!(workspace_runs.provider_drive_depth(&fixture.record.id), 0);
        let durable_nodes = fixture
            .lifecycle
            .load_timeline_nodes(&fixture.record.id)
            .expect("durable timeline nodes");
        assert!(
            durable_nodes.is_empty(),
            "Replay/拒收不得创建任何 timeline 节点"
        );
    }

    #[tokio::test]
    async fn anchor_t12_explicit_fresh_keeps_workspace_and_ignores_old_audit() {
        let fixture = anchor_t12_fixture();
        anchor_t12_seed(
            &fixture,
            LcResumeRecoveryState::Waiting,
            Vec::new(),
            Vec::new(),
        );
        // 旧 native 引用在案（durable conversation）——显式 fresh 不得复活。
        fixture
            .lifecycle
            .replace_workspace_provider_conversations(
                &fixture.record.id,
                vec![ProviderConversationRef {
                    role: ProviderConversationRole::Author,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "native-t12-old".to_string(),
                    updated_at: "2026-10-11T00:00:00Z".to_string(),
                    last_node_id: Some("timeline_node_002".to_string()),
                }],
            )
            .expect("seed old conversation");

        let before = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("record before claim");
        let (engine, _engine_rx) = anchor_t12_engine(&fixture);
        let workspace_runs = WorkspaceRunRegistry::default();
        let run_context = anchor_t12_run_context(&fixture, engine.clone(), workspace_runs.clone());
        let (outbound_tx, mut outbound_rx) = mpsc::channel::<OutboundControl>(64);
        let inbound_context = WorkspaceInboundContext {
            app_state: WebAppState::new(
                fixture.root.path().to_path_buf(),
                crate::web::runtime::WebRuntime::new_fake(fixture.root.path().to_path_buf()),
            ),
            engine: engine.clone(),
            run_context: run_context.clone(),
            outbound_tx: outbound_tx.clone(),
            session_id: fixture.record.id.clone(),
        };

        handle_workspace_inbound_message(
            inbound_context.clone(),
            anchor_t12_fresh_message("op-fresh", "recovery_anchor_t12"),
        )
        .await;

        let accepted = tokio::time::timeout(std::time::Duration::from_secs(2), outbound_rx.recv())
            .await
            .expect("accepted frame sent")
            .expect("outbound channel open");
        let OutboundControl::Text(accepted_json) = accepted else {
            panic!("expected text frame, got {accepted:?}");
        };
        let response: serde_json::Value =
            serde_json::from_str(&accepted_json).expect("accepted frame json");
        assert_eq!(response["type"], "lc_resume_recovery_operation_result");
        assert_eq!(response["delivery"], "accepted");
        assert_eq!(response["operation_id"], "op-fresh");
        assert_eq!(response["operation_state"], "reserved");
        assert_eq!(response["fresh_node_id"], serde_json::Value::Null);

        // durable：FreshReserved 单 claim，operation 绑定 op-fresh；fresh 面
        // 零 native/节点绑定（新 input None——不复活旧 audit/native）。
        let after = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("record after claim");
        let recovery = anchor_t12_recovery_of(&after);
        assert_eq!(recovery.state, LcResumeRecoveryState::FreshReserved);
        assert_eq!(recovery.operation_id.as_deref(), Some("op-fresh"));
        assert_eq!(
            recovery.fresh_operations.len(),
            1,
            "新 ID waiting 恰一次 claim"
        );
        let operation = &recovery.fresh_operations[0];
        assert_eq!(operation.operation_id, "op-fresh");
        assert_eq!(operation.state, LcFreshOperationState::Reserved);
        assert_eq!(operation.fresh_node_id, None);
        assert_eq!(operation.role_run_seq, None);
        assert_eq!(recovery.fresh_native_session_id, None);
        assert_eq!(
            recovery.superseded_native_session_id.as_deref(),
            Some("native-t12-old"),
            "claim 不得清理/复活旧 native 引用"
        );

        // workspace 保留：非恢复字段逐项不变（entity/type/status/provider/
        // permission/messages/旧 conversation）。
        assert_eq!(after.entity_id, before.entity_id);
        assert_eq!(after.workspace_type, before.workspace_type);
        assert_eq!(after.status, before.status);
        assert_eq!(after.author_provider, before.author_provider);
        assert_eq!(after.reviewer_provider, before.reviewer_provider);
        assert_eq!(after.permission_modes, before.permission_modes);
        assert_eq!(after.messages, before.messages);
        assert_eq!(
            after.provider_conversations, before.provider_conversations,
            "显式 fresh 忽略旧 audit/conversation（不读取、不复活、不清理）"
        );

        // 零 spawn：无 provider drive、无新 timeline 节点、engine 锁即刻可取。
        assert_eq!(workspace_runs.provider_drive_depth(&fixture.record.id), 0);
        let _engine_probe = engine.clone().lock_owned().await;
        let durable_nodes = fixture
            .lifecycle
            .load_timeline_nodes(&fixture.record.id)
            .expect("durable timeline nodes");
        assert!(durable_nodes.is_empty(), "claim 不得创建 timeline 节点");

        // 同 ID 重发：只 Replay 不二claim（waiting 单 claim 语义）。
        handle_workspace_inbound_message(
            inbound_context,
            anchor_t12_fresh_message("op-fresh", "recovery_anchor_t12"),
        )
        .await;
        let replayed = tokio::time::timeout(std::time::Duration::from_secs(2), outbound_rx.recv())
            .await
            .expect("replayed frame sent")
            .expect("outbound channel open");
        let OutboundControl::Text(replayed_json) = replayed else {
            panic!("expected text frame, got {replayed:?}");
        };
        let replay_response: serde_json::Value =
            serde_json::from_str(&replayed_json).expect("replayed frame json");
        assert_eq!(replay_response["delivery"], "replayed");
        assert_eq!(replay_response["operation_state"], "reserved");
        let after_resend = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("record after resend");
        let recovery_after = anchor_t12_recovery_of(&after_resend);
        assert_eq!(
            recovery_after.fresh_operations.len(),
            1,
            "同 ID 重发不得产生第二次 claim"
        );
    }

    #[tokio::test]
    async fn anchor_t12_empty_operation_id_rejected_before_claim() {
        let fixture = anchor_t12_fixture();
        anchor_t12_seed(
            &fixture,
            LcResumeRecoveryState::Waiting,
            Vec::new(),
            Vec::new(),
        );
        let (engine, _engine_rx) = anchor_t12_engine(&fixture);
        let workspace_runs = WorkspaceRunRegistry::default();
        let run_context = anchor_t12_run_context(&fixture, engine, workspace_runs);

        for empty_operation_id in ["", "   "] {
            let error = lc_resume_recovery_inbound_gate(
                &run_context,
                &anchor_t12_fresh_message(empty_operation_id, "recovery_anchor_t12"),
            )
            .expect("空/空白 operation_id 必须被拒收");
            assert_eq!(anchor_t12_protocol_code(&error), "INVALID_COMMAND_ID");
        }

        // durable 事实不变：Waiting 保持、零受理。
        let after = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("record after rejections");
        let recovery = anchor_t12_recovery_of(&after);
        assert_eq!(recovery.state, LcResumeRecoveryState::Waiting);
        assert!(
            recovery.fresh_operations.is_empty(),
            "空 operation_id 拒收必须发生在 claim 之前"
        );
    }

    #[tokio::test]
    async fn anchor_t12_provider_config_change_rejected_before_claim() {
        let fixture = anchor_t12_fixture();
        anchor_t12_seed(
            &fixture,
            LcResumeRecoveryState::Waiting,
            Vec::new(),
            Vec::new(),
        );
        let (engine, _engine_rx) = anchor_t12_engine(&fixture);
        let workspace_runs = WorkspaceRunRegistry::default();
        let run_context = anchor_t12_run_context(&fixture, engine, workspace_runs);

        let mut changed_author = anchor_t12_fresh_message("op-cfg-1", "recovery_anchor_t12");
        let WsInMessage::StartGeneration {
            provider_config, ..
        } = &mut changed_author
        else {
            unreachable!();
        };
        provider_config.author = ProviderName::Codex;
        let changed_author_error = lc_resume_recovery_inbound_gate(&run_context, &changed_author)
            .expect("author 配置漂移必须被拒收");
        assert_eq!(
            anchor_t12_protocol_code(&changed_author_error),
            "lc_resume_recovery_configuration_changed"
        );

        let mut changed_reviewer = anchor_t12_fresh_message("op-cfg-2", "recovery_anchor_t12");
        let WsInMessage::StartGeneration {
            provider_config, ..
        } = &mut changed_reviewer
        else {
            unreachable!();
        };
        provider_config.reviewer = None;
        let changed_reviewer_error =
            lc_resume_recovery_inbound_gate(&run_context, &changed_reviewer)
                .expect("reviewer 配置漂移必须被拒收");
        assert_eq!(
            anchor_t12_protocol_code(&changed_reviewer_error),
            "lc_resume_recovery_configuration_changed"
        );

        let mut changed_permission = anchor_t12_fresh_message("op-cfg-3", "recovery_anchor_t12");
        let WsInMessage::StartGeneration {
            provider_config, ..
        } = &mut changed_permission
        else {
            unreachable!();
        };
        provider_config.permission_modes = WorkspaceRolePermissionModes {
            author: ProviderPermissionMode::Supervised,
            reviewer: ProviderPermissionMode::Auto,
        };
        let changed_permission_error =
            lc_resume_recovery_inbound_gate(&run_context, &changed_permission)
                .expect("permission 配置漂移必须被拒收");
        assert_eq!(
            anchor_t12_protocol_code(&changed_permission_error),
            "lc_resume_recovery_configuration_changed"
        );

        // durable 事实不变：配置漂移拒收发生在 claim 之前。
        let after = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("record after rejections");
        let recovery = anchor_t12_recovery_of(&after);
        assert_eq!(recovery.state, LcResumeRecoveryState::Waiting);
        assert!(
            recovery.fresh_operations.is_empty(),
            "配置漂移拒收必须发生在 claim 之前"
        );
    }

    #[tokio::test]
    async fn anchor_t12_stale_recovery_or_changed_source_rejected_before_claim() {
        let fixture = anchor_t12_fixture();
        // source 引用冻结：artifact v1 内容 hash 记录进 run_context。
        let artifact = ArtifactVersion {
            version: 1,
            payload: ArtifactPayload::Markdown {
                markdown: "# T12 源稿\n".to_string(),
                diff: None,
            },
            generated_by: ProviderName::ClaudeCode,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: "2026-10-11T00:00:00Z".to_string(),
            source_node_id: "timeline_node_002".to_string(),
        };
        let frozen_hash = lc_artifact_payload_digest(&artifact.payload).expect("digest computable");
        fixture
            .lifecycle
            .save_artifact_versions(&fixture.record.id, &[artifact])
            .expect("seed artifact");
        anchor_t12_seed(
            &fixture,
            LcResumeRecoveryState::Waiting,
            Vec::new(),
            vec![LcRecoveryArtifactRef {
                ref_id: "artifact_current".to_string(),
                version: Some(1),
                hash: Some(frozen_hash.clone()),
            }],
        );
        let (engine, _engine_rx) = anchor_t12_engine(&fixture);
        let workspace_runs = WorkspaceRunRegistry::default();
        let run_context = anchor_t12_run_context(&fixture, engine, workspace_runs);

        // 已换 recovery_id（被新漂移替换）：typed stale。
        let stale_recovery_error = lc_resume_recovery_inbound_gate(
            &run_context,
            &anchor_t12_fresh_message("op-stale-1", "recovery_replaced_by_new_drift"),
        )
        .expect("recovery 替换后旧 ID 必须被拒收");
        assert_eq!(
            anchor_t12_protocol_code(&stale_recovery_error),
            "lc_resume_recovery_stale"
        );

        // 正控：recovery_id 正确且 source hash 未变 → 门放行（None，进入
        // stage+mutex+claim 路径）。
        assert!(
            lc_resume_recovery_inbound_gate(
                &run_context,
                &anchor_t12_fresh_message("op-stale-2", "recovery_anchor_t12"),
            )
            .is_none(),
            "身份/配置/source 一致时必须放行 claim 路径"
        );

        // 原 source hash 变化（artifact v1 内容被改写）：typed stale。
        let tampered = ArtifactVersion {
            version: 1,
            payload: ArtifactPayload::Markdown {
                markdown: "# T12 源稿（被改写）\n".to_string(),
                diff: None,
            },
            generated_by: ProviderName::ClaudeCode,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: "2026-10-11T00:00:00Z".to_string(),
            source_node_id: "timeline_node_002".to_string(),
        };
        fixture
            .lifecycle
            .save_artifact_versions(&fixture.record.id, &[tampered])
            .expect("tamper artifact");
        let changed_source_error = lc_resume_recovery_inbound_gate(
            &run_context,
            &anchor_t12_fresh_message("op-stale-3", "recovery_anchor_t12"),
        )
        .expect("source hash 变化必须被拒收");
        assert_eq!(
            anchor_t12_protocol_code(&changed_source_error),
            "lc_resume_recovery_stale"
        );

        // durable 事实不变：全部拒收发生在 claim 之前（仍 Waiting、零受理、
        // 冻结 hash 原样）。
        let after = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("record after rejections");
        let recovery = anchor_t12_recovery_of(&after);
        assert_eq!(recovery.state, LcResumeRecoveryState::Waiting);
        assert!(
            recovery.fresh_operations.is_empty(),
            "stale/source 拒收必须发生在 claim 之前"
        );
        let LcRecoveryRunContext::ReviewerRevision { business, .. } = &recovery.run_context else {
            unreachable!("seeded reviewer revision context");
        };
        let frozen_ref = business
            .artifact_refs
            .first()
            .cloned()
            .expect("frozen artifact ref intact");
        assert_eq!(frozen_ref.hash.as_deref(), Some(frozen_hash.as_str()));
    }
}

/// 初帧载荷：未注入 `event_seq` 的 snapshot、可选补发 choice，以及 snapshot 构建
/// 时刻的基线序号（投递时若存在活跃 run 补发窗口，基线会被压低到窗口首事件
/// 之前，见 `activate_attachment_with_initial_frames`）。
struct InitialAttach {
    session_state: WsOutMessage,
    choice: Option<WsOutMessage>,
    attach_seq: u64,
}

async fn flush_initial_attachment(
    manager: &WorkspaceSessionManager,
    connection_id: &str,
    outbound_tx: &mpsc::Sender<OutboundControl>,
    initial: Option<InitialAttach>,
) {
    // 激活与补发窗口冻结在投递前完成：中途取消也保持已激活，直播帧不会丢失。
    let frames = match initial {
        Some(initial) => manager.activate_attachment_with_initial_frames(
            connection_id,
            initial.session_state,
            initial.choice,
            initial.attach_seq,
        ),
        // 初帧已被更早路径投递（或本就没有）：仅保底完成激活。
        None => {
            manager.activate_attachment(connection_id);
            Vec::new()
        }
    };
    for frame in frames {
        if outbound_tx
            .send(OutboundControl::Text(frame))
            .await
            .is_err()
        {
            return;
        }
    }
}

/// C3/REQ-HTR-01：Abort 矩阵校验的 stage 读取（非阻塞）。
/// - engine 锁空闲：活 stage 参与矩阵判定（门开/终态拒、run 族放行）。
/// - 锁被 run/compile 持有：若人工确认门开着（router 维护的锁无关门开标志，
///   覆盖 human_confirm 门内 in-flight 修订 run 场景），按 human_confirm 拒收
///   （design D1：in-flight 修订期 Abort 窄 escape 不实现，watchdog 600s 兜底）；
///   否则返回 None 放行进入取消臂——存在 active run 时 Abort 的既有语义
///   （取消并回执 Aborted）必须原样可达。
async fn abort_stage_type(
    engine: std::sync::Arc<tokio::sync::Mutex<WorkspaceEngine>>,
    manager: &WorkspaceSessionManager,
) -> Option<(WorkspaceStage, WorkspaceType, bool)> {
    match engine.try_lock() {
        Ok(engine) => Some((
            engine.current_stage(),
            engine.session().workspace_type.clone(),
            false,
        )),
        Err(_) => {
            if manager.human_confirm_gate_open() {
                Some((
                    WorkspaceStage::HumanConfirm,
                    manager.session_record.workspace_type.clone(),
                    false,
                ))
            } else {
                None
            }
        }
    }
}

/// B1b（Task 4.2）：recovery 冻结 artifact 引用的当前内容指纹——serde 规范
/// 序列化后 sha256（`sha256:<hex>`）。与 `LcRecoveryArtifactRef.hash` 同式
/// 比对：内容变化即 `lc_resume_recovery_stale`，不复活旧 source。
fn lc_artifact_payload_digest(payload: &ArtifactPayload) -> Option<String> {
    use sha2::Digest as _;
    let canonical = serde_json::to_string(payload).ok()?;
    Some(format!(
        "sha256:{:x}",
        sha2::Sha256::digest(canonical.as_bytes())
    ))
}

/// B1b（Task 4.2，设计 §5.1）：LC 恢复入站门——arbitrate（auth/lease/scope）
/// 之后、stage 校验/engine mutex 之前的固定判定顺序，全部只读 durable 事实
///（store 层已保证），不等待 engine mutex：
/// ① `StartGeneration` 携带显式 `resume_recovery`：
///    operation_id 空/空白 typed `INVALID_COMMAND_ID` → recovery 身份（缺失/
///    已被新漂移替换 → `lc_resume_recovery_stale` 族）→ 同 ID 已受理先 typed
///    Replay（零 spawn、不重核配置——原受理已核，先于 stage waiting/busy）→
///    新 ID 才核状态机（FreshReserved/FreshRunning → busy、Resolved → 不造
///    第二会话）、无活 provider drive、冻结配置（author/reviewer/permission
///    与 durable session record 相等）、run_context 冻结 artifact 引用与当前
///    durable 产物一致；全部通过返回 `None` 放行原 stage 校验 + engine
///    mutex/CAS claim 路径（decisions 的 `claim_lc_fresh` 消费）。
/// ② `RequestRevision`：active 恢复（Waiting/FreshReserved/FreshRunning）期间
///    普通旁路 typed 拒收（不得绕过显式 fresh 等待）；Resolved 只保留证据
///    不拦截，后续新 native 正常。
/// 其余消息一律返回 `None` 原路放行。
pub(crate) fn lc_resume_recovery_inbound_gate(
    run_context: &ProviderRunContext,
    message: &WsInMessage,
) -> Option<WsOutMessage> {
    match message {
        WsInMessage::StartGeneration {
            provider_config,
            resume_recovery: Some(intent),
            ..
        } => lc_resume_fresh_admission(run_context, intent, provider_config),
        WsInMessage::RequestRevision { .. } => lc_request_revision_recovery_guard(run_context),
        _ => None,
    }
}

/// B1b（Task 4.2）新增稳定错误码（tasks.md 4.2 逐项负例；socket 门与
/// decisions claim 消费面共用，修改须双向同步）。
pub(crate) const LC_RESUME_RECOVERY_CONFIGURATION_CHANGED_CODE: &str =
    "lc_resume_recovery_configuration_changed";
pub(crate) const LC_RESUME_RECOVERY_BUSY_CODE: &str = "lc_resume_recovery_busy";
/// B2a（Task 5.1，tasks.md 5.1）：LC 恢复 run_context 未知/非法 tag 的 typed
/// 拒收码（设计 §5.3：不 fallback Author/plan_author，不 silently 当普通
/// 生成处理）。
pub(crate) const LC_RESUME_RECOVERY_CONTEXT_UNSUPPORTED_CODE: &str =
    "lc_resume_recovery_context_unsupported";
/// 与 engine `start_generation` 的 `LC_RESUME_RECOVERY_ACTIVE` 稳定码同串
///（Task 2.6 anchor_t07 已把该串定为等待期普通旁路拒收码）。
const LC_RESUME_RECOVERY_ACTIVE_CODE: &str = "LC_RESUME_RECOVERY_ACTIVE";
/// durable store 读写失败稳定码——与 manager 侧
/// `DURABLE_PROJECTION_UNAVAILABLE_CODE`（模块私有常量）同值；按 wire 值
/// 对齐，两处修改必须同步。
pub(crate) const LC_RESUME_STORE_UNAVAILABLE_CODE: &str =
    "workspace_durable_projection_unavailable";

fn lc_resume_protocol_error(
    code: &str,
    message: String,
    context: serde_json::Value,
) -> WsOutMessage {
    WsOutMessage::ProtocolError {
        code: code.to_string(),
        message,
        context: Some(context),
    }
}

fn lc_resume_store_unavailable(
    session_id: &str,
    error: crate::product::json_store::ProductStoreError,
) -> WsOutMessage {
    lc_resume_protocol_error(
        LC_RESUME_STORE_UNAVAILABLE_CODE,
        format!("durable projection unavailable for session {session_id}: {error}"),
        serde_json::json!({ "session_id": session_id }),
    )
}

/// ②（设计 §5.1）：`RequestRevision` 普通旁路守卫——active 恢复期间一律
/// typed 拒收，不得绕过显式 fresh 等待；Resolved 只保留证据放行。
fn lc_request_revision_recovery_guard(run_context: &ProviderRunContext) -> Option<WsOutMessage> {
    let store = LifecycleStore::new(run_context.app_paths.clone());
    let record = match store.get_workspace_session(&run_context.session_id) {
        Ok(record) => record,
        Err(error) => {
            return Some(lc_resume_store_unavailable(&run_context.session_id, error));
        }
    };
    let recovery = record.lc_resume_recovery?;
    if recovery.state == LcResumeRecoveryState::Resolved {
        return None;
    }
    Some(lc_resume_protocol_error(
        LC_RESUME_RECOVERY_ACTIVE_CODE,
        format!(
            "LC_RESUME_RECOVERY_ACTIVE: 会话 {} 存在进行中的 LC 恢复（recovery_id={}，state={:?}）；RequestRevision 普通旁路已拒收，请以显式 fresh（StartGeneration.resume_recovery）继续",
            run_context.session_id, recovery.recovery_id, recovery.state
        ),
        serde_json::json!({
            "recovery_id": recovery.recovery_id,
            "state": recovery.state,
        }),
    ))
}

/// ①③④⑤⑥⑦（设计 §5.1）：显式 fresh 的前置核验。`None` = 放行原
/// stage 校验 + engine mutex + decisions CAS claim 路径。
fn lc_resume_fresh_admission(
    run_context: &ProviderRunContext,
    intent: &LcResumeRecoveryIntent,
    provider_config: &ProviderConfigSnapshot,
) -> Option<WsOutMessage> {
    // ① 幂等/审计键先于一切（与 HumanGateFeedback/Advance 同族 typed 命令键）。
    if let Err(error) = validate_command_id(&intent.operation_id) {
        return Some(error);
    }
    let store = LifecycleStore::new(run_context.app_paths.clone());
    let record = match store.get_workspace_session(&run_context.session_id) {
        Ok(record) => record,
        Err(error) => {
            return Some(lc_resume_store_unavailable(&run_context.session_id, error));
        }
    };
    // ② recovery 身份：缺失/已被新漂移替换 → stale（不造第二会话、不猜最新）。
    let recovery = match record.lc_resume_recovery.as_ref() {
        Some(recovery) if recovery.recovery_id == intent.recovery_id => recovery,
        Some(recovery) => {
            return Some(lc_resume_protocol_error(
                LcFreshClaimOutcome::STALE_SUPERSEDED,
                format!(
                    "lc resume recovery {} is superseded; the durable recovery is {}",
                    intent.recovery_id, recovery.recovery_id
                ),
                serde_json::json!({ "recovery_id": recovery.recovery_id }),
            ));
        }
        None => {
            return Some(lc_resume_protocol_error(
                LcFreshClaimOutcome::STALE_NO_RECOVERY,
                "no lc resume recovery is waiting for this session".to_string(),
                serde_json::Value::Null,
            ));
        }
    };
    // ③ 同 ID 已受理先 Replay：零 spawn、不重核配置（原受理已核）、不等待
    //    engine mutex（durable 纯读）；resolved 同 ID 也只 Replay。
    if let Some(operation) = recovery
        .fresh_operations
        .iter()
        .find(|operation| operation.operation_id == intent.operation_id)
    {
        return Some(WsOutMessage::LcResumeRecoveryOperationResult {
            recovery_id: recovery.recovery_id.clone(),
            operation_id: operation.operation_id.clone(),
            delivery: LcResumeRecoveryDelivery::Replayed,
            operation_state: operation.state,
            fresh_node_id: operation.fresh_node_id.clone(),
            reason_code: operation.reason_code.clone(),
        });
    }
    // ④ 新 ID 才核状态机：reserved/running → busy；resolved → 不造第二会话。
    match recovery.state {
        LcResumeRecoveryState::Waiting => {}
        LcResumeRecoveryState::FreshReserved | LcResumeRecoveryState::FreshRunning => {
            return Some(lc_resume_fresh_busy_error(recovery));
        }
        LcResumeRecoveryState::Resolved => {
            return Some(lc_resume_protocol_error(
                LcFreshClaimOutcome::STALE_RESOLVED,
                format!(
                    "lc resume recovery {} is resolved; a new fresh operation would create a second session",
                    recovery.recovery_id
                ),
                serde_json::json!({ "recovery_id": recovery.recovery_id }),
            ));
        }
    }
    // ⑤ 无活 provider drive（run registry 计数，锁无关）。
    if run_context
        .workspace_runs
        .provider_drive_depth(&run_context.session_id)
        > 0
    {
        return Some(lc_resume_fresh_busy_error(recovery));
    }
    // ⑥ 冻结配置：author/reviewer/permission 与 durable session record 相等。
    if provider_config.author != record.author_provider
        || provider_config.reviewer != record.reviewer_provider
        || provider_config.permission_modes != record.permission_modes
    {
        return Some(lc_resume_protocol_error(
            LC_RESUME_RECOVERY_CONFIGURATION_CHANGED_CODE,
            "the submitted provider configuration differs from the frozen configuration of the waiting recovery"
                .to_string(),
            serde_json::json!({
                "frozen_author": record.author_provider,
                "frozen_reviewer": record.reviewer_provider,
            }),
        ));
    }
    // ⑦ run_context 冻结 artifact 引用与当前 durable 产物一致。
    match lc_recovery_source_stale(&store, &run_context.session_id, recovery) {
        Ok(false) => None,
        Ok(true) => Some(lc_resume_protocol_error(
            LcFreshClaimOutcome::STALE_SUPERSEDED,
            "the source artifacts referenced by the waiting recovery changed since freeze"
                .to_string(),
            serde_json::json!({ "recovery_id": recovery.recovery_id }),
        )),
        Err(error) => Some(lc_resume_store_unavailable(&run_context.session_id, error)),
    }
}

fn lc_resume_fresh_busy_error(recovery: &LcResumeRecoveryRecord) -> WsOutMessage {
    lc_resume_protocol_error(
        LC_RESUME_RECOVERY_BUSY_CODE,
        format!(
            "lc resume recovery {} already holds an accepted fresh operation; a new operation id is rejected and running/unknown operations are never auto-resent",
            recovery.recovery_id
        ),
        serde_json::json!({ "active_operation_id": recovery.operation_id }),
    )
}

/// ⑦（设计 §5.1）：冻结 source 引用 vs 当前 durable artifact 版本。按
/// `version` 定位；`hash` 严格比对（未冻结 hash/version 的引用跳过——不
/// 倒算）。冻结版本消失或内容指纹不等 → stale。
fn lc_recovery_source_stale(
    store: &LifecycleStore,
    session_id: &str,
    recovery: &LcResumeRecoveryRecord,
) -> Result<bool, crate::product::json_store::ProductStoreError> {
    let artifact_refs = lc_recovery_business_artifact_refs(recovery);
    if artifact_refs.is_empty() {
        return Ok(false);
    }
    let versions = store.list_artifact_versions(session_id)?;
    for artifact_ref in artifact_refs {
        let Some(version) = artifact_ref.version else {
            continue;
        };
        let Some(current) = versions
            .iter()
            .find(|candidate| candidate.version == version)
        else {
            return Ok(true);
        };
        if let Some(frozen_hash) = artifact_ref.hash.as_deref()
            && lc_artifact_payload_digest(&current.payload).as_deref() != Some(frozen_hash)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// run_context 各 tag 的公共 business artifact 引用（A1a 模型纪律：全部
/// tag 均携带 `business.artifact_refs`）。
fn lc_recovery_business_artifact_refs(
    recovery: &LcResumeRecoveryRecord,
) -> &[LcRecoveryArtifactRef] {
    match &recovery.run_context {
        LcRecoveryRunContext::Author { business, .. }
        | LcRecoveryRunContext::AuthorChoiceFollowup { business, .. }
        | LcRecoveryRunContext::AuthorFeedback { business, .. }
        | LcRecoveryRunContext::ReviewerRevision { business, .. }
        | LcRecoveryRunContext::LegacyOutlineAuthor { business, .. }
        | LcRecoveryRunContext::LegacyOutlineRevision { business, .. }
        | LcRecoveryRunContext::LegacyOutlineRebuild { business, .. }
        | LcRecoveryRunContext::LegacyDraft { business, .. }
        | LcRecoveryRunContext::LegacyBatch { business, .. }
        | LcRecoveryRunContext::LegacyPlanRevision { business, .. }
        | LcRecoveryRunContext::ScAuthor { business, .. }
        | LcRecoveryRunContext::ScManualRevision { business, .. } => {
            business.artifact_refs.as_slice()
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
            // S6（2026-10-10 E2E）可观测性：create 失败臂原先发一帧 Error 即静默
            // 关连接，服务端零痕迹（连接诊断只在 receiver 循环完整走完后记录），
            // 现场只能看到前端 60s 重连循环。此处补一条 stderr 痕迹对齐
            // aria-connection-diagnostic 惯例。
            eprintln!(
                "[aria-workspace-session-create-failed] session={session_id} connection={connection_id}: {message}"
            );
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
    // A1b2（设计 §4.4）：初帧 durable 纯读失败——立即发 typed ProtocolError
    //（workspace_durable_projection_unavailable），不排队伪 snapshot/旧可
    // 启动 baseline；连接仍按原宽限/首条入站路径激活直播，客户端经
    // cursor Hello 重订阅恢复。
    let pending_initial = match manager.attached_session_state().await {
        Ok((session_state, restored_choice_request)) => Arc::new(Mutex::new(Some(InitialAttach {
            session_state: session_state.with_connection_id(&connection_id),
            choice: restored_choice_request,
            attach_seq: manager.attach_baseline_seq(),
        }))),
        Err(error) => {
            let frame = error.unavailable_frame(&session_id);
            if let Ok(json) = serde_json::to_string(&frame) {
                let _ = outbound_tx.send(OutboundControl::Text(json)).await;
            }
            Arc::new(Mutex::new(None))
        }
    };

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
                        // L2 退役（REQ-WSC-08/REQ-RET-03）：已删除的 legacy 决策
                        // 消息在 parse 面拒收（serde 未知变体）；退役 wire 名识别
                        // 出来后返回 stage-specific protocol error（含 stage 上下
                        // 文），其余 parse 失败维持通用 invalid message 错误。
                        // 识别只读原始文本——零副作用（会话状态与事件流不变）。
                        let err = if let Some(retired_name) =
                            retired_inbound_message_type(&text)
                        {
                            let stage = engine.lock().await.current_stage();
                            retired_message_protocol_error(&stage, retired_name)
                        } else {
                            WsOutMessage::Error {
                                message: format!("invalid message: {e}"),
                            }
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

                // B1b（Task 4.2，设计 §5.1）：LC 恢复入站门——auth/lease/scope
                //（arbitrate）之后、stage 校验/engine mutex 之前。同 ID 已受理
                // 立即 typed Replay（run 持锁期间不等待 engine mutex，store 层
                // 纯读）；RequestRevision 活动等待期拒收；新 ID 前置核验通过才
                // 放行 stage+mutex+CAS claim 路径。
                if let Some(frame) = lc_resume_recovery_inbound_gate(&run_context, in_msg) {
                    let _ = send_json_outbound(&outbound_tx, &frame).await;
                    continue;
                }

                let stage_type_and_cancel_replay = if requires_stage_validation(in_msg) {
                    if matches!(in_msg, WsInMessage::Abort) {
                        // C3/REQ-HTR-01：Abort 的矩阵校验用非阻塞 stage 读——
                        // provider run 全程持有 engine 锁，阻塞等锁会让 Abort
                        // （run 级取消的逃生口）在最需要它的时刻楔死整个读循环。
                        abort_stage_type(engine.clone(), manager.as_ref()).await
                    } else {
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
                    }
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

                let is_lc_fresh_intent = matches!(
                    in_msg,
                    WsInMessage::StartGeneration {
                        resume_recovery: Some(_),
                        ..
                    }
                );
                handle_workspace_inbound_message(inbound_context.clone(), envelope).await;
                // B2a（Task 5.1，设计 §5.1/§5.3）：显式 fresh claim（decisions
                // `claim_lc_fresh` 已受理并发 typed Accepted/Reserved 帧）之后的
                // tag 精确 dispatch——runner 侧消费 FreshReserved 启动 fresh
                // native run（唯一启动口经 `start_work_item_plan_author`
                // Logical 分支，B0 full prepare；旧 audit 不回捞）；未接管 tag
                // typed 拒（context_unsupported），零 spawn、不 plan_author 兜底。
                // 同 ID Replay/Busy/Stale 已由入站门先收口，此处仅推进刚受理的
                // FreshReserved（幂等：非 FreshReserved 直接返回）。
                if is_lc_fresh_intent {
                    dispatch_lc_fresh_recovery_run(run_context.clone(), outbound_tx.clone())
                        .await;
                }
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
