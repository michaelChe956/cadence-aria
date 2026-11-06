//! C 层真实 WS 场景共享件（任务10）：场景替身、帧收集器与 WS 辅助。
//!
//! 经 `provider_run_small_stack.rs` 顶层 `#[path]` 声明挂载（同
//! `basic_sc_arm_cases/sc_author_stop_cases.rs` 先例）——`use super::*` 可达
//! 父 harness 模块私有 helper（真实 fixture / WS 收发工具）。本文件只放
//! **共享**件；三个场景执行体在 `scenario_cases.rs`。
use super::ws_small_stack::WsSmokeEvidence;
use super::*;

/// 真实 WS 流类型（与父 recv_json/send_json 签名一致）。
pub(super) type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// 场景内收帧：完整 JSON 解析 + **有界**墙钟超时。
///
/// 父 harness 的 `recv_json` 用 3s 超时且超时即 panic——SC 人工门反馈会触发
/// 真实 provider 启动与门内派发，3s 内无帧属正常（门内 turn 异步收口）。
/// 本地收帧给足 30s；超时/断连返回 `None`，由调用方带着**已收帧序列**转成
/// 可诊断的显式失败（而非 panic 掩盖现场）。
pub(super) async fn recv_frame_bounded(ws: &mut WsStream) -> Option<WsOutMessage> {
    use tokio_tungstenite::tungstenite::Message;
    match tokio::time::timeout(std::time::Duration::from_secs(30), ws.next()).await {
        Err(_) => None,
        Ok(None) => None,
        Ok(Some(Err(error))) => panic!("C 层场景收到非法帧: {error}"),
        Ok(Some(Ok(Message::Text(text)))) => {
            Some(serde_json::from_str(&text).expect("C 层场景 JSON 帧解析失败"))
        }
        Ok(Some(Ok(other))) => panic!("C 层场景期望文本帧，得到 {other:?}"),
    }
}

/// C 层场景替身：按调用序脚本化（挂起供取消 / 立即完成输出），并把每次
/// validated 启动记为可信 provider-start 探针（真实 start_validated 调用；
/// locked/登记不算）。结构化输出契约经 `reviewer_contract_from_prompt`
/// 从生产 prompt 的输出模板复刻（与父 ScriptedStreamingProvider 同手法）。
pub(super) enum ScenarioStep {
    /// 发一个 TextDelta 后挂起（select 取消），供 Abort 取消窗口。
    HangAfterFirstDelta,
    /// 立即发出完整输出并完成（输出文本经契约包装收口）。
    Complete(&'static str),
}

/// 运行类时间线节点（durable Completed 探针只锚定这些——StartGeneration
/// 等「出生即 Completed」的标记节点不算 run 输出收口证据）。
fn is_run_node(node_type: &TimelineNodeType) -> bool {
    matches!(
        node_type,
        TimelineNodeType::AuthorRun
            | TimelineNodeType::ReviewerRun
            | TimelineNodeType::Revision
            | TimelineNodeType::WorkItemPlanOutlineRun
            | TimelineNodeType::WorkItemDraftRun
            | TimelineNodeType::WorkItemBatchRun
    )
}

pub(super) struct ScenarioScriptedProvider {
    pub(super) steps: Mutex<VecDeque<ScenarioStep>>,
    pub(super) prompts: Arc<Mutex<Vec<String>>>,
}

impl ScenarioScriptedProvider {
    pub(super) fn session(
        &self,
        step: ScenarioStep,
        contract: Option<
            cadence_aria::cross_cutting::structured_output::StructuredOutputContract,
        >,
        cancel: CancellationToken,
    ) -> ProviderSession {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        match step {
            ScenarioStep::HangAfterFirstDelta => {
                tokio::spawn(async move {
                    let _ = event_tx
                        .send(ProviderEvent::TextDelta {
                            content: "scenario hanging delta（等待取消）".to_string(),
                        })
                        .await;
                    // 挂起直到驱动取消（Abort → start_validated 收到的
                    // CancellationToken）：不自行完成，供取消窗口观察收口。
                    tokio::select! {
                        _ = cancel.cancelled() => {}
                        _ = std::future::pending::<()>() => unreachable!(),
                    }
                });
                ProviderSession {
                    native_session_id: None,
                    events: event_rx,
                    commands: command_tx,
                }
            }
            ScenarioStep::Complete(output) => {
                tokio::spawn(async move {
                    let _ = event_tx
                        .send(ProviderEvent::TextDelta {
                            content: output.to_string(),
                        })
                        .await;
                    let _ = event_tx
                        .send(ProviderEvent::Completed(scripted_provider_completion(
                            output.to_string(),
                            contract.as_ref(),
                        )))
                        .await;
                });
                ProviderSession {
                    native_session_id: None,
                    events: event_rx,
                    commands: command_tx,
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScenarioScriptedProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.prompts.lock().unwrap().push(input.prompt);
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("scenario step");
        Ok(self.session(step, None, cancel))
    }

    /// LC 会话经 gateway validated 启动（父夹具同手法：从 launch_view 复刻
    /// prompt/role，结构化契约从 prompt 输出模板恢复；按 trait 契约不回调裸
    /// start）。真实调用即可信 provider-start 探针。
    async fn start_validated(
        &self,
        validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let view = validated.launch_view();
        self.prompts.lock().unwrap().push(view.prompt.to_string());
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("scenario step");
        let contract = reviewer_contract_from_prompt(view.role, view.prompt);
        Ok(self.session(step, contract, cancel))
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

/// 帧收集器：`recv_json` 逐帧完整解析（serde 失败即 panic——不完整 JSON 帧
/// 不是可接受证据）；每帧后同步读 durable 检查点，记录统一事件序上的因果
/// 偏序观察点（locked/首输出/durable Completed/completed 回显/取消回显）。
pub(super) struct FrameCollector {
    pub(super) app_paths: ProductAppPaths,
    pub(super) session_id: String,
    /// 可信 provider-start 探针（与 provider 替身共享同一计数器）——用于在
    /// **首个输出帧被观察到的当下**快照 start 次数，形成跨 sender 的因果偏序
    /// 锚点（start 必须先于输出），而非事后比较两个独立计数器的终值。
    pub(super) start_probe: Arc<Mutex<Vec<String>>>,
    /// 首个输出帧被观察到时已发生的真实 validated start 次数。
    pub(super) provider_starts_before_first_output: usize,
    /// 本 run 期间创建的运行类时间线节点（TimelineNodeCreated 帧捕获）。
    pub(super) run_node_ids: Vec<String>,
    pub(super) frames: Vec<String>,
    pub(super) provider_locked_at: Option<usize>,
    pub(super) first_output_at: Option<usize>,
    pub(super) durable_completed_at: Option<usize>,
    pub(super) completed_echo_at: Option<usize>,
    pub(super) cancel_failure_echoes: usize,
    pub(super) abort_sent: bool,
    pub(super) baseline_timeline_nodes: usize,
    pub(super) baseline_messages: usize,
    pub(super) durable_progress: bool,
}

impl FrameCollector {
    pub(super) fn new(
        app_paths: ProductAppPaths,
        session_id: &str,
        start_probe: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let baseline_timeline_nodes = lifecycle
            .load_timeline_nodes(session_id)
            .map(|nodes| nodes.len())
            .unwrap_or(0);
        let baseline_messages = lifecycle
            .get_workspace_session(session_id)
            .map(|record| record.messages.len())
            .unwrap_or(0);
        Self {
            app_paths,
            session_id: session_id.to_string(),
            start_probe,
            provider_starts_before_first_output: 0,
            run_node_ids: Vec::new(),
            frames: Vec::new(),
            provider_locked_at: None,
            first_output_at: None,
            durable_completed_at: None,
            completed_echo_at: None,
            cancel_failure_echoes: 0,
            abort_sent: false,
            baseline_timeline_nodes,
            baseline_messages,
            durable_progress: false,
        }
    }

    /// durable Completed 探针：本 run 期间创建的运行类节点进入 Completed。
    pub(super) fn durable_run_node_completed(&self) -> bool {
        let lifecycle = LifecycleStore::new(self.app_paths.clone());
        let Ok(nodes) = lifecycle.load_timeline_nodes(&self.session_id) else {
            return false;
        };
        nodes.iter().any(|node| {
            self.run_node_ids.contains(&node.node_id)
                && node.status == TimelineNodeStatus::Completed
        })
    }

    /// durable 进展探针：timeline 节点数或持久消息数相对基线推进。
    pub(super) fn refresh_durable_progress(&mut self) {
        let lifecycle = LifecycleStore::new(self.app_paths.clone());
        let timeline_now = lifecycle
            .load_timeline_nodes(&self.session_id)
            .map(|nodes| nodes.len())
            .unwrap_or(self.baseline_timeline_nodes);
        let messages_now = lifecycle
            .get_workspace_session(&self.session_id)
            .map(|record| record.messages.len())
            .unwrap_or(self.baseline_messages);
        self.durable_progress = timeline_now > self.baseline_timeline_nodes
            || messages_now > self.baseline_messages;
    }

    /// 取消收口的 durable 证据：本 run 节点出现 Failed 终态。
    pub(super) fn durable_run_node_failed(&self) -> bool {
        let lifecycle = LifecycleStore::new(self.app_paths.clone());
        let Ok(nodes) = lifecycle.load_timeline_nodes(&self.session_id) else {
            return false;
        };
        nodes.iter().any(|node| {
            self.run_node_ids.contains(&node.node_id)
                && node.status == TimelineNodeStatus::Failed
        })
    }

    /// 收一帧（完整 JSON 解析）并记录统一事件序检查点。
    pub(super) async fn next(&mut self, ws: &mut WsStream) -> WsOutMessage {
        let Some(frame) = recv_frame_bounded(ws).await else {
            panic!(
                "C 层场景收帧超时/断连（30s 无新帧；已收帧序列={:?}）",
                self.frames
            );
        };
        let index = self.frames.len();
        match &frame {
            WsOutMessage::ProviderLocked { .. } => {
                self.provider_locked_at.get_or_insert(index);
                self.frames.push("provider_locked".to_string());
            }
            WsOutMessage::StreamChunk { .. } => {
                if self.first_output_at.is_none() {
                    // 首个输出帧的当下快照 start 探针——因果偏序锚点。
                    self.provider_starts_before_first_output =
                        self.start_probe.lock().unwrap().len();
                }
                self.first_output_at.get_or_insert(index);
                self.frames.push("stream_chunk".to_string());
            }
            WsOutMessage::MessageComplete { .. } => {
                self.completed_echo_at.get_or_insert(index);
                self.frames.push("message_complete".to_string());
            }
            WsOutMessage::TimelineNodeCreated { node } => {
                if is_run_node(&node.node_type) {
                    self.run_node_ids.push(node.node_id.clone());
                }
                self.frames.push("timeline_node_created".to_string());
            }
            WsOutMessage::TimelineNodeUpdated { status, .. } => {
                if self.abort_sent && *status == TimelineNodeStatus::Failed {
                    self.cancel_failure_echoes += 1;
                }
                self.frames.push("timeline_node_updated".to_string());
            }
            WsOutMessage::Error { message } => {
                if self.abort_sent {
                    self.cancel_failure_echoes += 1;
                }
                self.frames.push(format!("error:{message}"));
            }
            WsOutMessage::ProtocolError { code, message, .. } => {
                self.frames.push(format!("protocol_error:{code}:{message}"));
            }
            WsOutMessage::HumanGateTurnRejected {
                rejection_reason,
                rejection_diagnostics,
                ..
            } => {
                self.frames.push(format!(
                    "human_gate_turn_rejected:{rejection_reason}:{rejection_diagnostics:?}"
                ));
            }
            WsOutMessage::Pong => self.frames.push("pong".to_string()),
            // 其余变体：记录**真实 wire type 名**（枚举按 `#[serde(tag="type")]`
            // 序列化）——诊断时能看出到底是哪一帧，而不必退化成 "other"。
            _ => self.frames.push(
                serde_json::to_value(&frame)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("type")
                            .and_then(|kind| kind.as_str())
                            .map(std::borrow::ToOwned::to_owned)
                    })
                    .unwrap_or_else(|| "<unserializable>".to_string()),
            ),
        }
        // 帧后 durable 检查点（同序）：Completed 首次可观察的位置。
        if self.durable_completed_at.is_none() && self.durable_run_node_completed() {
            self.durable_completed_at = Some(index);
        }
        self.refresh_durable_progress();
        frame
    }

    /// 收帧直到**两个谓词同时**命中（有界，防挂死）。
    ///
    /// 人工门族路径的帧序不保证「输出帧先于终态帧」——SC manual revision 的
    /// typed 拒收帧（`human_gate_turn_rejected` / `protocol_error`）可能**先于**
    /// provider 输出帧到达。分两次 `until` 会把先到的终态帧吃掉、第二个
    /// `until` 永远等不到——故此处用「两个条件都满足才算命中」的单次收集，
    /// 保持同一事件序上的完整因果偏序。
    pub(super) async fn until_both(
        &mut self,
        ws: &mut WsStream,
        first: &(dyn Fn(&WsOutMessage) -> bool + Send + Sync),
        second: &(dyn Fn(&WsOutMessage) -> bool + Send + Sync),
        label: &str,
    ) -> (WsOutMessage, WsOutMessage) {
        let mut first_hit: Option<WsOutMessage> = None;
        let mut second_hit: Option<WsOutMessage> = None;
        for _ in 0..600 {
            let frame = self.next(ws).await;
            if first_hit.is_none() && first(&frame) {
                first_hit = Some(frame.clone());
            }
            if second_hit.is_none() && second(&frame) {
                second_hit = Some(frame.clone());
            }
            if first_hit.is_some() && second_hit.is_some() {
                return (first_hit.expect("first hit"), second_hit.expect("second hit"));
            }
        }
        panic!(
            "frames {label} not both received (frames so far: {:?})",
            self.frames
        );
    }

    /// 跳过无关帧直到谓词命中（有界，防挂死；命中帧也经 `next` 记录）。
    pub(super) async fn until(
        &mut self,
        ws: &mut WsStream,
        predicate: &(dyn Fn(&WsOutMessage) -> bool + Send + Sync),
        label: &str,
    ) -> WsOutMessage {
        for _ in 0..600 {
            let frame = self.next(ws).await;
            if predicate(&frame) {
                return frame;
            }
            if let WsOutMessage::Error { message } = &frame {
                panic!("unexpected ws error while waiting {label}: {message}");
            }
        }
        panic!(
            "frame {label} not received (frames so far: {:?})",
            self.frames
        );
    }

    pub(super) fn into_evidence(
        self,
        scenario: &str,
        provider_starts: usize,
        expects_provider_locked: bool,
    ) -> WsSmokeEvidence {
        WsSmokeEvidence {
            scenario: scenario.to_string(),
            client_closed: false,
            client_timeout: false,
            provider_locked_at: self.provider_locked_at,
            provider_starts,
            provider_starts_before_first_output: self.provider_starts_before_first_output,
            expects_provider_locked,
            first_output_at: self.first_output_at,
            durable_completed_at: self.durable_completed_at,
            completed_echo_at: self.completed_echo_at,
            cancel_failure_echoes: self.cancel_failure_echoes,
            durable_progress: self.durable_progress,
        }
    }
}

/// 起真实 axum 服务（accept 发生在 2 MiB worker 上的 tokio::spawn）。
pub(super) async fn serve_ws_app(
    app: axum::Router,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (addr, server)
}

/// 带可信同源 Origin 的 WS 连接（生产 api_auth 的受信精确 Origin 合同；
/// C 层显式携带同一来源）。
pub(super) async fn connect_ws_with_origin(url: &str, origin: &str) -> WsStream {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = url.into_client_request().expect("ws request");
    request.headers_mut().insert(
        "origin",
        tokio_tungstenite::tungstenite::http::HeaderValue::from_str(origin)
            .expect("origin header"),
    );
    connect_async(request).await.expect("connect ws").0
}

/// 同一真实服务 socket 上的 `/api/health` HTTP 探测（服务未 abort 的独立
/// 证据；进程内 router 未装 api_auth，health 公开直通）。
pub(super) async fn http_health_check(addr: std::net::SocketAddr) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .map_err(|error| format!("health connect failed: {error}"))?;
    let request = format!("GET /api/health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| format!("health write failed: {error}"))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .map_err(|error| format!("health read failed: {error}"))?;
    let status_line = response.lines().next().unwrap_or("");
    if !status_line.contains(" 200 ") {
        return Err(format!("health status not 200: {status_line}"));
    }
    Ok(())
}

pub(super) fn start_generation_message() -> WsInMessage {
    WsInMessage::StartGeneration {
        provider_config: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: None,
            review_rounds: 0,
            permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
        },
        reviewer_enabled: false,
        resume_recovery: None,
    }
}

/// Hello Driver 准入（last_seen_node_id=None、after_event_seq=None），以
/// Ping→Pong 作为 bind_role 生效屏障（父 harness 同手法）。
pub(super) async fn hello_driver_admission(ws: &mut WsStream, session_id: &str) {
    send_json(
        ws,
        &WsInMessage::Hello {
            session_id: session_id.to_string(),
            last_seen_node_id: None,
            role: Some(HelloRole::Driver),
            after_event_seq: None,
        },
    )
    .await;
    send_json(ws, &WsInMessage::Ping).await;
    for _ in 0..40 {
        if matches!(recv_frame_bounded(ws).await, Some(WsOutMessage::Pong)) {
            return;
        }
    }
    panic!("driver admission pong not received (session={session_id})");
}

pub(super) fn author_terminal_variant(frame: &WsOutMessage) -> &'static str {
    match frame {
        WsOutMessage::MessageComplete { .. } => "message_complete",
        WsOutMessage::StageChange { .. } => "stage_change",
        WsOutMessage::Error { .. } => "error",
        WsOutMessage::HumanGateTurnCompleted { .. } => "human_gate_turn_completed",
        WsOutMessage::HumanGateTurnFailed { .. } => "human_gate_turn_failed",
        WsOutMessage::HumanGateTurnRejected { .. } => "human_gate_turn_rejected",
        WsOutMessage::HumanGateBusy { .. } => "human_gate_busy",
        _ => "other",
    }
}

/// plan outline author 的结构化输出全文。
///
/// 生产解析面是 `parse_work_item_split_structured_output`（sentinel 解析，
/// 非 Markdown 围栏）：开始标签 nonce 必须与 JSON 顶层 envelope nonce 一致，
/// 且解析后 nonce 字段被剥离（`strip_and_validate_json_nonce`）。故此处必须
/// 直接给 sentinel 全文——outline author 角色（WorkItemSplitter）不持有
/// reviewer 契约，`scripted_provider_completion` 不再包 sentinel。
///
/// 字段形状与 `legacy_outline_cases::case_valid_outline_output`（既有
/// 通过 `WorkItemPlanOutlineValidator` 的合法样例）同构：
/// - `source_story_spec_ids` / `source_design_spec_ids` 非空（traceability 硬要求）；
/// - `estimated_context_tokens` ∈ [1, 50000]、`session_fit=fits_single_agent_session`；
/// - `exclusive_write_scopes` 非空；
/// - **不得**出现 `dependency_graph` 等禁用键（`forbidden_outline_field`）。
pub(super) const PLAN_OUTLINE_AUTHOR_OUTPUT: &str = "完成拆分规划：\n\
     单候选先落后端，一条 outline。\n\
     <ARIA_STRUCTURED_OUTPUT nonce=\"ws2miboutline1\">\n\
     {\n\
       \"nonce\": \"ws2miboutline1\",\n\
       \"outline\": {\n\
         \"id\": \"outline_ws_2mib_1\",\n\
         \"project_id\": \"project_0001\",\n\
         \"issue_id\": \"issue_0001\",\n\
         \"source_story_spec_ids\": [\"story_0001\"],\n\
         \"source_design_spec_ids\": [\"design_0001\"],\n\
         \"strategy_summary\": \"先后端\",\n\
         \"work_item_outlines\": [\n\
           {\n\
             \"outline_id\": \"outline_backend\",\n\
             \"logical_work_item_id\": \"wi_backend\",\n\
             \"title\": \"后端 API\",\n\
             \"kind\": \"backend\",\n\
             \"goal\": \"实现 API\",\n\
             \"scope\": [\"src/product\"],\n\
             \"non_goals\": [],\n\
             \"estimated_context_tokens\": 12000,\n\
             \"session_fit\": \"fits_single_agent_session\",\n\
             \"source_story_spec_ids\": [\"story_0001\"],\n\
             \"source_design_spec_ids\": [\"design_0001\"],\n\
             \"exclusive_write_scopes\": [\"src/product/**\"],\n\
             \"forbidden_write_scopes\": [\"web/**\"],\n\
             \"depends_on\": [],\n\
             \"verification_intent\": [\"cargo test --locked --lib api\"],\n\
             \"handoff_notes\": \"提供 API contract\"\n\
           }\n\
         ],\n\
         \"risks\": [],\n\
         \"handoff_strategy\": \"后端输出 contract\",\n\
         \"status\": \"draft\"\n\
       },\n\
       \"context_blockers\": []\n\
     }\n\
     </ARIA_STRUCTURED_OUTPUT>";

/// plan reviewer 的结构化 review 输出（verdict=approved；契约包装同上）。
pub(super) const PLAN_REVIEW_OUTPUT: &str = "评审意见：拆分合理。\n\
     ```json\n\
     {\n\
       \"verdict\": \"approved\",\n\
       \"comments\": \"拆分与依赖合理，无需返修。\",\n\
       \"summary\": \"通过\",\n\
       \"findings\": []\n\
     }\n\
     ```";
