use super::*;
use crate::product::workspace_engine::run_boundary;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

// ─────────────────── 四段时刻采集（TTFT 归因，零行为变更） ───────────────────
//
// 每个物理轮记四个时刻，供事后判断「慢在哪」：
//   ① 轮启动：entry 在真实 provider start **之前**登记（登记时刻 = epoch）；
//   ② 首个非空模型增量：`ProviderEvent::TextDelta` 的非空分支；
//   ③ 首个可显示内容：`emit_work_item_plan_display_chunk` 的非空分支（真正进入
//      stream buffer / WS chunk 的那一下）；
//   ④ 客户端接收：观测点在 WS 发送层（socket 写），不在本模块可读范围 ⇒
//      恒输出 `"unknown"` + 显式原因，绝不伪造 0 或用墙钟猜。
//
// 采集点全部是进程内原子写：零 IO、零 await；持久化只在 entry 侧的
// `finish_work_item_plan_round_timing`，调用点恒在 `drive_*` 返回**之后**（首字节
// 早已发生，或本轮从未启动）⇒ 首字节之前零新增持久化 await。
//
// 载体：会话 timeline 根下的 append-only sidecar `round-timing.jsonl`——不往既有
// durable schema（RoundBound / split run / node detail）加字段，既有校验零改动，
// sidecar 可缺可弃、损坏不影响任何生产读路径。
//
// 缺值一律字符串 `"unknown"`（不是 0）：0 ms 是合法值，写成 0 会伪装成「瞬时命中」。

/// 时刻槽位「未观测」哨兵：`AtomicU64::fetch_min` 语义要求 `u64::MAX`。
const ROUND_TIMING_SLOT_UNSET: u64 = u64::MAX;
/// sidecar 缺值的唯一表示（不是 0、不是 null）。
const ROUND_TIMING_UNKNOWN: &str = "unknown";
/// 一次 provider start 的观测态：未观测。
const ROUND_TIMING_START_UNOBSERVED: u8 = 0;
/// `Ok(session)`：start 已被接受。
const ROUND_TIMING_START_STARTED: u8 = 1;
/// `Err`：start 被拒——本轮永远不会产生首字节，TTFT 必须 unknown。
const ROUND_TIMING_START_REFUSED: u8 = 2;
/// sidecar 行 schema（`run_arms.py` 的解析契约）。
const ROUND_TIMING_SCHEMA: &str = "wig-round-timing/1";
/// 会话 timeline 根下的 append-only sidecar 文件名。
const ROUND_TIMING_SIDECAR_FILE: &str = "round-timing.jsonl";
/// ④ 不可观测的显式原因（区分「没测」与「测得 0」）。
const ROUND_TIMING_CLIENT_RECEIVE_UNOBSERVED: &str = "ws_egress_not_instrumented";

/// 轮类型：初轮与教学轮必须分账。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoundTimingKind {
    Initial,
    Teaching,
    Unknown,
}

impl RoundTimingKind {
    fn parse(raw: &str) -> Self {
        match raw {
            "initial" => Self::Initial,
            "teaching" => Self::Teaching,
            _ => Self::Unknown,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Teaching => "teaching",
            Self::Unknown => "unknown",
        }
    }
}

/// 一个物理轮的四段时刻（相对 `epoch` = 轮启动，单位毫秒）。
struct WorkItemPlanRoundTiming {
    node_id: String,
    round_id: String,
    role_run_seq: u64,
    kind: RoundTimingKind,
    workspace_session_id: String,
    /// 墙钟起点（仅用于与 daemon.log 等外部日志对齐；取不到即 unknown）。
    round_start_unix_ms: Option<u64>,
    epoch: std::time::Instant,
    start_state: AtomicU8,
    /// ①b：provider session 实际到手（start 返回 `Ok`）的时刻——与轮启动分开记，
    /// 「握手慢」与「模型慢」才能分账。
    session_ready_ms: AtomicU64,
    first_model_delta_ms: AtomicU64,
    first_display_chunk_ms: AtomicU64,
}

impl WorkItemPlanRoundTiming {
    fn elapsed_ms(&self) -> u64 {
        // 一轮不可能跨越 u64 毫秒上界；不可表示时退到 UNSET-1（仍是已知值）。
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(ROUND_TIMING_SLOT_UNSET - 1)
    }

    fn mark_start_state(&self, state: u8) {
        self.start_state.store(state, AtomicOrdering::Release);
    }

    /// ①b「provider session 到手」只记第一次。
    fn mark_session_ready(&self) {
        self.session_ready_ms
            .fetch_min(self.elapsed_ms(), AtomicOrdering::AcqRel);
    }

    /// ②「首个非空模型增量」只记第一次（`fetch_min` 单调）。
    fn mark_first_model_delta(&self) {
        self.first_model_delta_ms
            .fetch_min(self.elapsed_ms(), AtomicOrdering::AcqRel);
    }

    /// ③「首个可显示内容」只记第一次。
    fn mark_first_display_chunk(&self) {
        self.first_display_chunk_ms
            .fetch_min(self.elapsed_ms(), AtomicOrdering::AcqRel);
    }

    /// 一条 sidecar 记录：缺值一律 `"unknown"`。
    fn snapshot(&self, outcome: &str) -> serde_json::Value {
        let state = self.start_state.load(AtomicOrdering::Acquire);
        let (start_state, started) = match state {
            ROUND_TIMING_START_STARTED => ("started", true),
            ROUND_TIMING_START_REFUSED => ("refused", false),
            _ => ("unobserved", false),
        };
        // 拒启/未观测 ⇒ 无 TTFT：绝不把「从未启动」写成 0 ms。
        let outcome = if state == ROUND_TIMING_START_REFUSED {
            "refused"
        } else {
            outcome
        };
        let delta = self.first_model_delta_ms.load(AtomicOrdering::Acquire);
        let display = self.first_display_chunk_ms.load(AtomicOrdering::Acquire);
        let ttft = if started {
            round_timing_slot(delta)
        } else {
            serde_json::Value::from(ROUND_TIMING_UNKNOWN)
        };
        let round_start_unix_ms = self.round_start_unix_ms.map_or_else(
            || serde_json::Value::from(ROUND_TIMING_UNKNOWN),
            serde_json::Value::from,
        );
        serde_json::json!({
            "schema": ROUND_TIMING_SCHEMA,
            "workspace_session_id": self.workspace_session_id,
            "node_id": self.node_id,
            "round_id": self.round_id,
            "role_run_seq": self.role_run_seq,
            "round_kind": self.kind.as_str(),
            "start_state": start_state,
            "drive_outcome": outcome,
            "round_start_unix_ms": round_start_unix_ms,
            "provider_session_ready_ms": round_timing_slot(
                self.session_ready_ms.load(AtomicOrdering::Acquire),
            ),
            "first_model_delta_ms": round_timing_slot(delta),
            "first_display_chunk_ms": round_timing_slot(display),
            "client_receive_ms": ROUND_TIMING_UNKNOWN,
            "client_receive_reason": ROUND_TIMING_CLIENT_RECEIVE_UNOBSERVED,
            "ttft_ms": ttft,
            "drive_terminal_ms": self.elapsed_ms(),
        })
    }
}

fn round_timing_slot(millis: u64) -> serde_json::Value {
    if millis == ROUND_TIMING_SLOT_UNSET {
        serde_json::Value::from(ROUND_TIMING_UNKNOWN)
    } else {
        serde_json::Value::from(millis)
    }
}

/// 活跃轮采集登记簿（进程内，key = `node_id`）。只在短临界区加锁、绝不跨 await；
/// 中毒视作「没有登记」（观测面永不改变行为）。
static ROUND_TIMING_REGISTRY: Mutex<BTreeMap<String, Arc<WorkItemPlanRoundTiming>>> =
    Mutex::new(BTreeMap::new());
/// sidecar 追加的进程内串行化（先例：`usage-diagnostics.jsonl` 的日志 Mutex）。
static ROUND_TIMING_APPEND_MUTEX: Mutex<()> = Mutex::new(());

/// 登记簿里「未收口」条目的存活上界：被 abort/panic 打断的轮不再无限占用登记表。
const ROUND_TIMING_STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(60 * 60);

fn register_round_timing(node_id: &str, timing: Arc<WorkItemPlanRoundTiming>) {
    if let Ok(mut registry) = ROUND_TIMING_REGISTRY.lock() {
        registry.retain(|_, existing| existing.epoch.elapsed() < ROUND_TIMING_STALE_AFTER);
        registry.insert(node_id.to_string(), timing);
    }
}

fn take_round_timing(node_id: &str) -> Option<Arc<WorkItemPlanRoundTiming>> {
    ROUND_TIMING_REGISTRY
        .lock()
        .ok()
        .and_then(|mut registry| registry.remove(node_id))
}

/// 采集点用：当前 node 的活跃轮记录（无登记/中毒 ⇒ `None`）。
fn active_round_timing(node_id: &str) -> Option<Arc<WorkItemPlanRoundTiming>> {
    ROUND_TIMING_REGISTRY
        .lock()
        .ok()?
        .get(node_id)
        .map(Arc::clone)
}

fn unix_now_ms() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
}

/// 把一条轮时刻记录追加到会话 timeline 根的 sidecar。
///
/// 只追加、从不读取（既有文件损坏不影响后续追加）；写失败只留结构化 warn——
/// 观测面永不改变 gate/provider 行为。写点恒在 `drive_*` 返回之后。
fn append_round_timing_record(
    store: &crate::product::lifecycle_store::LifecycleStore,
    session_id: &str,
    record: &serde_json::Value,
) {
    use std::io::Write as _;
    let written = store.with_workspace_session_live_write(session_id, || {
        let _guard = ROUND_TIMING_APPEND_MUTEX.lock().map_err(|error| {
            crate::product::json_store::ProductStoreError::Io(format!("round timing lock: {error}"))
        })?;
        let path = store
            .workspace_timeline_root_for_session(session_id)?
            .join(ROUND_TIMING_SIDECAR_FILE);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                crate::product::json_store::ProductStoreError::Io(format!(
                    "create {}: {error}",
                    parent.display()
                ))
            })?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| {
                crate::product::json_store::ProductStoreError::Io(format!(
                    "open {}: {error}",
                    path.display()
                ))
            })?;
        serde_json::to_writer(&mut file, record).map_err(|error| {
            crate::product::json_store::ProductStoreError::Json(error.to_string())
        })?;
        file.write_all(b"\n").map_err(|error| {
            crate::product::json_store::ProductStoreError::Io(format!(
                "write {}: {error}",
                path.display()
            ))
        })?;
        file.flush().map_err(|error| {
            crate::product::json_store::ProductStoreError::Io(format!(
                "flush {}: {error}",
                path.display()
            ))
        })
    });
    if let Err(error) = written {
        tracing::warn!(
            target: "workspace_engine",
            session_id,
            error = %error,
            "round timing sidecar append failed"
        );
    }
}

impl WorkspaceEngine {
    /// 登记一轮的四段时刻采集。**必须在真实 provider start 之前**调用：登记时刻
    /// 即「轮启动」，②③ 都相对它计。零 await、零 IO。
    pub(crate) fn begin_work_item_plan_round_timing(
        &self,
        node_id: &str,
        round_id: &str,
        role_run_seq: u64,
        round_kind: &str,
    ) {
        if node_id.is_empty() {
            return;
        }
        let timing = Arc::new(WorkItemPlanRoundTiming {
            node_id: node_id.to_string(),
            round_id: round_id.to_string(),
            role_run_seq,
            kind: RoundTimingKind::parse(round_kind),
            workspace_session_id: self.session.session_id.clone(),
            round_start_unix_ms: unix_now_ms(),
            epoch: std::time::Instant::now(),
            start_state: AtomicU8::new(ROUND_TIMING_START_UNOBSERVED),
            session_ready_ms: AtomicU64::new(ROUND_TIMING_SLOT_UNSET),
            first_model_delta_ms: AtomicU64::new(ROUND_TIMING_SLOT_UNSET),
            first_display_chunk_ms: AtomicU64::new(ROUND_TIMING_SLOT_UNSET),
        });
        register_round_timing(node_id, timing);
    }

    /// 收口一轮：摘登记 → 生成记录 → 追加 sidecar。
    ///
    /// **只在 `drive_*` 返回之后调用**。无 store（内存态 engine）时只摘登记、不落盘。
    pub(crate) fn finish_work_item_plan_round_timing(&self, node_id: &str, outcome: &str) {
        let Some(timing) = take_round_timing(node_id) else {
            return;
        };
        let record = timing.snapshot(outcome);
        if let Some(store) = self.lifecycle_store.as_ref() {
            append_round_timing_record(store, &timing.workspace_session_id, &record);
        }
    }
}

impl WorkspaceEngine {
    pub async fn drive_work_item_plan_provider_session_to_output(
        &mut self,
        session: Result<
            ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        >,
        command_rx: &mut mpsc::Receiver<ProviderCommand>,
        node_id: String,
        agent: ProviderName,
    ) -> Result<String, String> {
        // 四段时刻采集：本轮无登记（非 SC 轮 / 内存态测试）⇒ `None`，全程零副作用。
        let round_timing = active_round_timing(&node_id);
        let mut session = match session {
            Ok(session) => {
                if let Some(timing) = round_timing.as_deref() {
                    timing.mark_start_state(ROUND_TIMING_START_STARTED);
                    timing.mark_session_ready();
                }
                session
            }
            Err(error) => {
                // 拒启 = 本轮永不会有首字节：先落观测态，再走既有失败收口。
                if let Some(timing) = round_timing.as_deref() {
                    timing.mark_start_state(ROUND_TIMING_START_REFUSED);
                }
                // 诊断直通（claude×轻 握手谜团第 2 轮）：workitem author 启动失败的
                // stderr 尾部（有界）并入消息——随 EngineEvent::Error → WS error
                // 消息上浮，并在驱动 result（Err 返回值）中同源携带。
                let mut message = error.details.clone();
                crate::cross_cutting::provider_adapter::ProviderAdapterError::append_bounded_stderr_tail(
                    &mut message,
                    &error.stderr,
                    crate::cross_cutting::provider_adapter::PROVIDER_ERROR_STDERR_TAIL_BYTES,
                );
                let _ = self
                    .event_tx
                    .send(EngineEvent::Error {
                        message: message.clone(),
                    })
                    .await;
                self.update_timeline_node(
                    &node_id,
                    TimelineNodeStatus::Failed,
                    Some("Provider 启动失败".to_string()),
                )
                .await;
                self.finish_failed_run().await;
                return Err(message);
            }
        };

        let cancel = self.cancel.clone();
        let mut full_content = String::new();
        let mut events_open = true;
        let mut commands_open = true;
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();
        let mut display_filter = StructuredOutputDisplayFilter::new();
        // F-27R3：挂起集经 PendingChoiceRequests 镜像到进程级登记簿（对照主驱动
        // provider_drive 先例）——session_state 全量投影据此补挂本子驱动挂起的
        // choice 卡；guard Drop（本函数任意出口）只摘本 run 登记的 id，防泄漏。
        let mut pending_choice_requests = PendingChoiceRequests::new(&self.session.session_id);
        // r29 问题2 根修(r28 现场):本循环此前缺 F-19 零活动看门狗(主驱动
        // drive_provider_session 有)——SC 门修订 turn 的 provider 流式 1min 后
        // 静默挂死,驱动永不收口,harness 侧 90min 空等零信号。同构补齐:
        // 事件/命令任一活动重置;等待人工权限/选择应答期间挂起(与主驱动
        // 同语义——人工等待由 ApprovalBridge PERMISSION_TIMEOUT 与
        // choice_wait 界收口,不是 provider 楔死)。
        // r32 校准:plan 文档生成的合法静默可超 10min(r28 实测 648s),本循环
        // 用 1800s 专用界(主驱动保持 600s——story/design 逐 token 流式形态
        // 不同);r32 现场 600s 界曾误杀合法慢生成。
        let idle_watchdog_timeout = PROVIDER_WORK_ITEM_PLAN_IDLE_WATCHDOG_TIMEOUT;
        let idle_watchdog =
            tokio::time::sleep_until(tokio::time::Instant::now() + idle_watchdog_timeout);
        tokio::pin!(idle_watchdog);
        let choice_wait_timeout = PROVIDER_CHOICE_WAIT_TIMEOUT;
        let choice_wait_timer =
            tokio::time::sleep_until(tokio::time::Instant::now() + choice_wait_timeout);
        tokio::pin!(choice_wait_timer);
        let mut waiting_for_permission = false;
        while events_open {
            tokio::select! {
                _ = &mut idle_watchdog,
                if !waiting_for_permission && pending_choice_requests.is_empty() =>
                {
                    // F-19 同构:provider 会话零活动楔死——Abort kill 链 + cancel +
                    // 失败节点带稳定原因码 + Error 事件 + finish_failed_run
                    //(workspace 会话回 prepare_context 可重跑)。
                    eprintln!(
                        "[aria-cancellation] workspace work_item_plan_drive idle_watchdog trigger=provider_idle_watchdog session_id={} role=author timeout_secs={}",
                        self.session.session_id,
                        idle_watchdog_timeout.as_secs()
                    );
                    let message = format!(
                        "provider_idle_watchdog: provider 会话 {} 秒零活动（无事件/命令），疑似楔死，运行已由看门狗中止；可重新开始生成",
                        idle_watchdog_timeout.as_secs()
                    );
                    let _ = session.commands.send(ProviderCommand::Abort).await;
                    cancel.cancel();
                    let display_content = display_filter.finish();
                    self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                    self.update_timeline_node(
                        &node_id,
                        TimelineNodeStatus::Failed,
                        Some(message.clone()),
                    )
                    .await;
                    let _ = self
                        .event_tx
                        .send(EngineEvent::Error { message })
                        .await;
                    self.finish_failed_run().await;
                    return Err("provider_idle_watchdog: work item plan provider 会话零活动楔死，运行已中止".to_string());
                }
                _ = &mut choice_wait_timer,
                if !pending_choice_requests.is_empty() =>
                {
                    // F-22/F-19b 同构:choice 卡丢失/无人应答超界。
                    let pending_ids: Vec<&str> = pending_choice_requests.ids();
                    eprintln!(
                        "[aria-cancellation] workspace work_item_plan_drive choice_wait_timeout trigger=provider_choice_wait_timeout session_id={} role=author pending={:?} timeout_secs={}",
                        self.session.session_id,
                        pending_ids,
                        choice_wait_timeout.as_secs()
                    );
                    let message = format!(
                        "provider_choice_wait_timeout: 等待回答超时——{} 秒内未收到选择应答，运行已中止；choice 卡可能未送达或已送达但无人应答（pending={:?}）。请重新提交反馈重新发起本轮",
                        choice_wait_timeout.as_secs(),
                        pending_ids
                    );
                    let _ = session.commands.send(ProviderCommand::Abort).await;
                    cancel.cancel();
                    let display_content = display_filter.finish();
                    self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                    self.update_timeline_node(
                        &node_id,
                        TimelineNodeStatus::Failed,
                        Some(message.clone()),
                    )
                    .await;
                    let _ = self
                        .event_tx
                        .send(EngineEvent::Error { message })
                        .await;
                    self.finish_failed_run().await;
                    return Err("provider_choice_wait_timeout: work item plan provider 等待选择应答超时，运行已中止".to_string());
                }
                _ = cancel.cancelled() => {
                    // 诊断打点（claude×轻 握手谜团第 2 轮，不改行为）：workitem author
                    // 驱动循环观察到 engine/run token 被外部取消。
                    eprintln!(
                        "[aria-cancellation] workspace work_item_plan_drive select_cancelled trigger=engine_cancelled_observed session_id={} role=author agent={agent:?}",
                        self.session.session_id
                    );
                    let display_content = display_filter.finish();
                    self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                    let _ = self.flush_stream_buffer(&node_id).await;
                    self.finish_aborted_run().await;
                    return Err(crate::product::workspace_engine::conversational_gate::cancel::PROVIDER_RUN_ABORTED_MESSAGE.to_string());
                }
                command = command_rx.recv(), if commands_open => {
                    // F-19 同构:任何命令活动(含人工权限/选择应答)重置看门狗。
                    idle_watchdog
                        .as_mut()
                        .reset(tokio::time::Instant::now() + idle_watchdog_timeout);
                    match command {
                        Some(ProviderCommand::Abort) => {
                            // 诊断打点：Abort 命令到达 workitem author 驱动循环。
                            eprintln!(
                                "[aria-cancellation] workspace work_item_plan_drive abort_command trigger=abort_command session_id={} role=author agent={agent:?}",
                                self.session.session_id
                            );
                            let _ = session.commands.send(ProviderCommand::Abort).await;
                            cancel.cancel();
                            let display_content = display_filter.finish();
                            self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                            let _ = self.flush_stream_buffer(&node_id).await;
                            self.finish_aborted_run().await;
                            return Err(crate::product::workspace_engine::conversational_gate::cancel::PROVIDER_RUN_ABORTED_MESSAGE.to_string());
                        }
                        Some(ProviderCommand::PermissionResponse {
                            id,
                            approved,
                            reason,
                        }) => {
                            // 人工权限应答到达:解除看门狗挂起(与主驱动同语义)。
                            waiting_for_permission = false;
                            let _ = self
                                .persist_permission_response(
                                    &node_id,
                                    id.clone(),
                                    serde_json::json!({
                                        "approved": approved,
                                        "reason": reason.clone(),
                                    }),
                                )
                                .await;
                            if session.commands.send(ProviderCommand::PermissionResponse {
                                id,
                                approved,
                                reason,
                            }).await.is_err() {
                                commands_open = false;
                            }
                        }
                        Some(ProviderCommand::ChoiceResponse {
                            id,
                            selected_option_ids,
                            free_text,
                            answers,
                            receipt,
                        }) => {
                            // F-27R3：应答命中挂起 choice（登记簿同步摘除）——
                            // 通知 Web runtime 广播全量 session_state 收敛已答卡
                            //（对照主驱动先例）。
                            if pending_choice_requests.remove(&id).is_some() {
                                let _ = self
                                    .event_tx
                                    .send(EngineEvent::ChoicePendingChanged)
                                    .await;
                            }
                            if session.commands.send(ProviderCommand::ChoiceResponse {
                                id,
                                selected_option_ids,
                                free_text,
                                answers,
                            receipt,
                            }).await.is_err() {
                                commands_open = false;
                            }
                        }
                        Some(ProviderCommand::ToolResult(_)) => {}
                        None => commands_open = false,
                    }
                }
                event = session.events.recv() => {
                    // F-19 同构:事件活动重置看门狗。
                    idle_watchdog
                        .as_mut()
                        .reset(tokio::time::Instant::now() + idle_watchdog_timeout);
                    let Some(event) = event else {
                        events_open = false;
                        continue;
                    };

                    match event {
                        ProviderEvent::TextDelta { content } => {
                            // ② 首个**非空**模型增量（空增量不算「模型开口」）。
                            if !content.is_empty()
                                && let Some(timing) = round_timing.as_deref()
                            {
                                timing.mark_first_model_delta();
                            }
                            full_content.push_str(&content);
                            let display_content = display_filter.push(&content);
                            self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                        }
                        ProviderEvent::PermissionRequest(request) => {
                            // F-19 同构:等待人工权限应答期间看门狗不计时。
                            waiting_for_permission = true;
                            let _ = self
                                .persist_permission_request(
                                    &node_id,
                                    request.id.clone(),
                                    serde_json::json!({
                                        "tool_name": request.tool_name.clone(),
                                        "description": request.description.clone(),
                                        "risk_level": risk_level_text(&request.risk_level),
                                    }),
                                )
                                .await;
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ExecutionEvent {
                                    event: ProviderExecutionEvent {
                                        event_id: format!("permission_{}", request.id),
                                        kind: ProviderExecutionEventKind::Command,
                                        status: ProviderExecutionEventStatus::WaitingApproval,
                                        title: "Waiting for permission".to_string(),
                                        detail: Some(request.description.clone()),
                                        command: Some(request.tool_name.clone()),
                                        cwd: self
                                            .session
                                            .repository_path
                                            .as_ref()
                                            .map(|path| path.display().to_string()),
                                        output: None,
                                        exit_code: None,
                                    },
                                    node_id: Some(node_id.clone()),
                                    agent: Some(agent.clone()),
                                })
                                .await;
                            let _ = self
                                .event_tx
                                .send(EngineEvent::PermissionRequest {
                                    id: request.id,
                                    tool_name: request.tool_name,
                                    description: request.description,
                                    risk_level: request.risk_level,
                                })
                                .await;
                        }
                        ProviderEvent::ChoiceRequest(request) => {
                            let questions = request.effective_questions();
                            // F-27R3：登记进程级挂起集（三驱动统一登记簿）——
                            // session_state 投影经登记簿携带本卡。
                            pending_choice_requests.insert(request.clone(), "author");
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ChoiceRequest {
                                    id: request.id,
                                    prompt: request.prompt,
                                    options: request.options,
                                    allow_multiple: request.allow_multiple,
                                    allow_free_text: request.allow_free_text,
                                    questions,
                                    source: request.source,
                                })
                                .await;
                        }
                        ProviderEvent::StatusChanged(status) => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ProviderStatus { status })
                                .await;
                        }
                        ProviderEvent::Execution(event) => {
                            self
                                .emit_execution_event(
                                    event,
                                    Some(node_id.clone()),
                                    Some(agent.clone()),
                                )
                                .await;
                        }
                        ProviderEvent::ToolCall(call) => {
                            tool_call_titles.insert(call.id.clone(), call.tool_name.clone());
                            if let Some(command) = extract_tool_command(&call.input) {
                                tool_call_commands.insert(call.id.clone(), command);
                            }
                            self
                                .emit_execution_event(
                                    execution_event_from_tool_call(call),
                                    Some(node_id.clone()),
                                    Some(agent.clone()),
                                )
                                .await;
                        }
                        ProviderEvent::ToolResult(result) => {
                            let title = tool_call_titles
                                .get(&result.tool_use_id)
                                .cloned()
                                .unwrap_or_else(|| "Tool result".to_string());
                            let command = tool_call_commands.get(&result.tool_use_id).cloned();
                            self
                                .emit_execution_event(
                                    execution_event_from_tool_result(result, title, command),
                                    Some(node_id.clone()),
                                    Some(agent.clone()),
                                )
                                .await;
                        }
                        ProviderEvent::Completed(completion) => {
                            let full_output = completion.full_output;
                            let provider_session_id = completion.provider_session_id;
                            let display_content = display_filter.finish();
                            self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                            let _ = self.flush_stream_buffer(&node_id).await;
                            self
                                .record_provider_session(
                                    ProviderConversationRole::Author,
                                    agent,
                                    provider_session_id,
                                    Some(node_id),
                                )
                                .await;
                            return Ok(full_output);
                        }
                        ProviderEvent::Failed { message } => {
                            let display_content = display_filter.finish();
                            self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                            let _ = self.flush_stream_buffer(&node_id).await;
                            let _ = self
                                .event_tx
                                .send(EngineEvent::Error {
                                    message: message.clone(),
                                })
                                .await;
                            self.update_timeline_node(
                                &node_id,
                                TimelineNodeStatus::Failed,
                                Some("Provider 运行失败".to_string()),
                            )
                            .await;
                            self.finish_failed_run().await;
                            return Err(message);
                        }
                        ProviderEvent::ProtocolError {
                            code,
                            message,
                            context,
                        } => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ProtocolError {
                                    code,
                                    message,
                                    context,
                                })
                                .await;
                        }
                        ProviderEvent::PermissionTimeout { permission_id } => {
                            self
                                .handle_permission_timeout(
                                    permission_id.clone(),
                                    Some(node_id.clone()),
                                )
                                .await;
                            return Err(format!("permission timeout: {permission_id}"));
                        }
                        // token 用量（F-40）：与主循环 / reviewer 驱动 / coding 引擎同构，
                        // 映射为 kind=usage 的 execution event——同时落 node detail 与 WS
                        // 事件流（此前本链路是空分支：计划拆分 author 的 token 在数据面
                        // 整体缺失，author 阶段卡/气泡因此无读数，仅 Review 卡有）。
                        ProviderEvent::UsageReport(report) => {
                            self.emit_execution_event(
                                execution_event_from_usage_report(report),
                                Some(node_id.clone()),
                                Some(agent.clone()),
                            )
                            .await;
                        }
                        // 策略审计出口（approval_decision/protocol_warning/
                        // session_terminated）：观测性事件，engine 主循环不消费；
                        // durable 落盘在 Task 3.2 的 LifecycleStore sink 注入。
                        ProviderEvent::ToolPolicyDecision(_)
                        | ProviderEvent::ToolPolicyWarning(_)
                        | ProviderEvent::ToolPolicyTerminated(_)
                        // 批2 Task 7：终端活动快照——观测性事件，计划驱动不消费
                        // （快照消费在 provider_drive/coding 两层看门狗）。
                        | ProviderEvent::TerminalActivity(_) => {}
                    }
                }
            }
        }

        let display_content = display_filter.finish();
        self.emit_work_item_plan_display_chunk(&node_id, display_content)
            .await;
        let _ = self.flush_stream_buffer(&node_id).await;
        if full_content.is_empty() {
            self.finish_empty_assistant_output().await;
            Err(crate::product::workspace_engine::conversational_gate::cancel::PROVIDER_EMPTY_OUTPUT_MESSAGE.to_string())
        } else {
            Ok(full_content)
        }
    }

    pub(crate) async fn emit_work_item_plan_display_chunk(
        &mut self,
        node_id: &str,
        content: String,
    ) {
        if content.is_empty() {
            return;
        }
        // ③ 首个可显示内容：真正进入 stream buffer / WS chunk 的那一下。
        if let Some(timing) = active_round_timing(node_id) {
            timing.mark_first_display_chunk();
        }
        let _ = self.buffer_stream_chunk(node_id, content.clone()).await;
        let _ = self
            .event_tx
            .send(EngineEvent::StreamChunk {
                role: "assistant".to_string(),
                content,
                node_id: Some(node_id.to_string()),
            })
            .await;
    }

    pub(crate) async fn emit_execution_event(
        &mut self,
        event: ProviderExecutionEvent,
        node_id: Option<String>,
        agent: Option<ProviderName>,
    ) {
        if let Some(node_id) = node_id.as_deref() {
            let event_json = execution_event_json(&event);
            let persisted = self
                .update_node_detail(node_id, |detail| {
                    upsert_execution_event_json(&mut detail.execution_events, event_json);
                })
                .await;
            // REQ-NDR-05：usage 事件的 durable 落盘失败此前被 `let _ =` 静默吞掉，导致
            // 「节点无 usage」无法区分「本来没有」与「落盘失败」。这里按档位留痕
            // （节点 detail 诊断事件 → 会话级 usage-diagnostics.jsonl → 结构化 warn），
            // 不改变事件广播与 gate/provider 行为。
            if let Err(error) = persisted
                && event.kind == ProviderExecutionEventKind::Usage
            {
                self.record_usage_persist_diagnostic(node_id, &event.event_id, &error)
                    .await;
            }
        }
        // REQ-PRB-10/11：原事件已照原样持久化 → 此处扫描（只扫作者侧 `Command` 事件）
        // → 广播原事件 → 最后追加告警事件（只告警，不阻断 run）。
        let source_event_id = event.event_id.clone();
        let scan = self.run_boundary_scan_request(&event, node_id.as_deref());
        let _ = self
            .event_tx
            .send(EngineEvent::ExecutionEvent {
                event,
                node_id: node_id.clone(),
                agent: agent.clone(),
            })
            .await;
        if let Some((node_id, findings)) = scan {
            self.append_run_boundary_alerts(&node_id, &source_event_id, &findings, agent)
                .await;
        }
    }

    /// REQ-PRB-10：扫描请求 = 作者侧节点 + `Command` 类事件；其余整段跳过。
    ///
    /// 作者侧判定按 `TimelineNodeType` 白名单（不按 provider 名或 role）：
    /// 先取事件所属节点的类型，内存时间线查不到时回落 `active_node_type()`。
    fn run_boundary_scan_request(
        &self,
        event: &ProviderExecutionEvent,
        node_id: Option<&str>,
    ) -> Option<(String, Vec<run_boundary::BoundaryFinding>)> {
        let node_id = node_id?;
        if event.kind != ProviderExecutionEventKind::Command {
            return None;
        }
        let node_type = self
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .map(|node| node.node_type.clone())
            .or_else(|| self.active_node_type())?;
        if !run_boundary::is_author_side_node(&node_type) {
            return None;
        }
        let context = self.run_boundary_context()?;
        let findings = run_boundary::scan_event(&context, event);
        if findings.is_empty() {
            return None;
        }
        Some((node_id.to_string(), findings))
    }

    /// REQ-PRB-10：扫描上下文（scratch / 状态根 / provider 历史目录表）。
    ///
    /// 无持久化 store（内存态 engine）时整段跳过——检测路径不参与成败判定。
    fn run_boundary_context(&self) -> Option<run_boundary::RunBoundary> {
        let store = self.lifecycle_store.as_ref()?;
        let dir = store.run_scratch_dir(&self.session.session_id);
        let scratch = std::fs::symlink_metadata(&dir)
            .ok()
            .filter(|metadata| metadata.file_type().is_dir())
            .map(|_| dir);
        Some(run_boundary::RunBoundary {
            state_root: store.app_paths().root().to_path_buf(),
            scratch,
            tmp_roots: vec![
                std::path::PathBuf::from("/tmp"),
                std::path::PathBuf::from("/var/tmp"),
            ],
            history_dirs: run_boundary::default_history_dirs(),
        })
    }

    /// REQ-PRB-10/11：把每个 finding 作为独立告警事件写进 node detail 并广播。
    ///
    /// 去重：`upsert_execution_event_json` 按 `event_id` 覆盖同 id 事件；
    /// 上界：每 node 至多 `MAX_RUN_BOUNDARY_ALERTS_PER_NODE` 条告警；
    /// 只告警：落盘/广播失败只留 `tracing::warn` 痕迹（与 usage 落盘失败同口径，
    /// 「没有告警」与「告警没落地」必须可区分），不向上传播。
    async fn append_run_boundary_alerts(
        &mut self,
        node_id: &str,
        source_event_id: &str,
        findings: &[run_boundary::BoundaryFinding],
        agent: Option<ProviderName>,
    ) {
        let mut inserted: Vec<ProviderExecutionEvent> = Vec::new();
        let persisted = self
            .update_node_detail(node_id, |detail| {
                let mut existing = detail
                    .execution_events
                    .iter()
                    .filter(|event| {
                        event
                            .get("event_id")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|id| id.starts_with("run_boundary_"))
                    })
                    .count();
                for finding in findings {
                    if existing >= run_boundary::MAX_RUN_BOUNDARY_ALERTS_PER_NODE {
                        break;
                    }
                    let alert = run_boundary::run_boundary_alert_event(finding, source_event_id);
                    let json = execution_event_json(&alert);
                    let duplicated = detail
                        .execution_events
                        .iter()
                        .any(|event| event.get("event_id") == json.get("event_id"));
                    upsert_execution_event_json(&mut detail.execution_events, json);
                    if !duplicated {
                        existing += 1;
                        inserted.push(alert);
                    }
                }
            })
            .await;
        if let Err(error) = persisted {
            tracing::warn!(
                target: "workspace_engine",
                node_id,
                error = %error,
                "run boundary alert persist failed"
            );
        }
        for alert in inserted {
            let event_id = alert.event_id.clone();
            if let Err(error) = self
                .event_tx
                .send(EngineEvent::ExecutionEvent {
                    event: alert,
                    node_id: Some(node_id.to_string()),
                    agent: agent.clone(),
                })
                .await
            {
                tracing::warn!(
                    target: "workspace_engine",
                    node_id,
                    event_id,
                    error = %error,
                    "run boundary alert broadcast failed"
                );
            }
        }
    }

    pub async fn emit_provider_prompt_event(
        &mut self,
        node_id: &str,
        prompt: String,
        detail: &'static str,
        agent: Option<ProviderName>,
    ) {
        let _ = self.persist_prompt_snapshot(node_id, prompt.clone()).await;
        self.emit_execution_event(
            provider_prompt_event(node_id, prompt, detail),
            Some(node_id.to_string()),
            agent,
        )
        .await;
    }
}

#[cfg(test)]
mod round_timing_tests {
    use super::*;

    fn timing(
        node_id: &str,
        round_id: &str,
        role_run_seq: u64,
        kind: RoundTimingKind,
    ) -> Arc<WorkItemPlanRoundTiming> {
        Arc::new(WorkItemPlanRoundTiming {
            node_id: node_id.to_string(),
            round_id: round_id.to_string(),
            role_run_seq,
            kind,
            workspace_session_id: "ws-round-timing-test".to_string(),
            round_start_unix_ms: Some(1_700_000_000_000),
            epoch: std::time::Instant::now(),
            start_state: AtomicU8::new(ROUND_TIMING_START_UNOBSERVED),
            session_ready_ms: AtomicU64::new(ROUND_TIMING_SLOT_UNSET),
            first_model_delta_ms: AtomicU64::new(ROUND_TIMING_SLOT_UNSET),
            first_display_chunk_ms: AtomicU64::new(ROUND_TIMING_SLOT_UNSET),
        })
    }

    /// (a) 缺值 ⇒ `"unknown"`，**不是** 0：0 ms 是合法时刻，写成 0 会伪装成瞬时命中。
    #[test]
    fn missing_slots_serialize_as_unknown_not_zero() {
        let record =
            timing("node-missing", "round-missing", 7, RoundTimingKind::Initial).snapshot("failed");
        for slot in [
            "provider_session_ready_ms",
            "first_model_delta_ms",
            "first_display_chunk_ms",
            "ttft_ms",
            "client_receive_ms",
        ] {
            assert_eq!(
                record[slot],
                serde_json::Value::from(ROUND_TIMING_UNKNOWN),
                "{slot} 缺值必须是 unknown"
            );
            assert_ne!(
                record[slot],
                serde_json::Value::from(0),
                "{slot} 缺值不得退化成 0"
            );
        }
        assert_eq!(
            record["client_receive_reason"],
            serde_json::Value::from(ROUND_TIMING_CLIENT_RECEIVE_UNOBSERVED)
        );
        // 反证：真正的 0 ms 仍原样输出 0（哨兵不是 0）。
        assert_eq!(round_timing_slot(0), serde_json::Value::from(0));
    }

    /// (b) 拒启 ⇒ 本轮没有 TTFT 记录（不是 0、不是「已完成」）。
    #[test]
    fn refused_start_records_no_ttft() {
        let timing = timing(
            "node-refused",
            "round-refused",
            3,
            RoundTimingKind::Teaching,
        );
        timing.mark_start_state(ROUND_TIMING_START_REFUSED);
        let record = timing.snapshot("failed");
        assert_eq!(record["start_state"], serde_json::Value::from("refused"));
        assert_eq!(record["drive_outcome"], serde_json::Value::from("refused"));
        assert_eq!(
            record["ttft_ms"],
            serde_json::Value::from(ROUND_TIMING_UNKNOWN)
        );
        assert_ne!(record["ttft_ms"], serde_json::Value::from(0));
        assert_eq!(
            record["first_model_delta_ms"],
            serde_json::Value::from(ROUND_TIMING_UNKNOWN)
        );
        // 拒启 ⇒ 从未拿到 session：该槽位也不得出现数字。
        assert_eq!(
            record["provider_session_ready_ms"],
            serde_json::Value::from(ROUND_TIMING_UNKNOWN)
        );
    }

    /// (c) 首个非空增量时刻必须早于节点完成（`drive_terminal_ms`），且只记第一次。
    #[test]
    fn first_delta_precedes_terminal_and_is_sticky() {
        let timing = timing("node-order", "round-order", 0, RoundTimingKind::Initial);
        timing.mark_start_state(ROUND_TIMING_START_STARTED);
        timing.mark_session_ready();
        std::thread::sleep(std::time::Duration::from_millis(10));
        timing.mark_first_model_delta();
        std::thread::sleep(std::time::Duration::from_millis(20));
        // 第二次打点不得覆盖第一次（`fetch_min` 单调）。
        timing.mark_first_model_delta();
        timing.mark_first_display_chunk();
        let record = timing.snapshot("completed");

        let session_ready = record["provider_session_ready_ms"]
            .as_u64()
            .expect("session 到手时刻已观测");
        let delta = record["first_model_delta_ms"]
            .as_u64()
            .expect("首个增量时刻已观测");
        let display = record["first_display_chunk_ms"]
            .as_u64()
            .expect("首个显示时刻已观测");
        let terminal = record["drive_terminal_ms"]
            .as_u64()
            .expect("节点完成时刻恒已知");
        assert!(
            session_ready <= delta,
            "session 到手不得晚于首个增量: session={session_ready} delta={delta}"
        );
        assert!(
            delta < terminal,
            "首增量必须早于节点完成: delta={delta} terminal={terminal}"
        );
        assert!(
            delta <= display,
            "首个可显示内容不得早于首个模型增量: delta={delta} display={display}"
        );
        // 首增量与首个显示之间隔了一次 20ms sleep：差值 < 10 说明首增量被后一次打点
        // 覆盖过（不单调）。
        assert!(
            display - delta >= 10,
            "首增量时刻必须只记第一次（不得被后续打点推后）: delta={delta} display={display}"
        );
        assert_eq!(record["ttft_ms"], serde_json::Value::from(delta));
    }

    /// (d) 教学轮与初轮可区分：轮类型 / round_id / role_run_seq 都独立成账。
    #[test]
    fn initial_and_teaching_rounds_are_distinguishable() {
        let initial = timing("node-ledger", "round-initial", 0, RoundTimingKind::Initial)
            .snapshot("completed");
        let teaching = timing(
            "node-ledger",
            "round-teaching",
            4,
            RoundTimingKind::Teaching,
        )
        .snapshot("completed");
        assert_eq!(initial["round_kind"], serde_json::Value::from("initial"));
        assert_eq!(teaching["round_kind"], serde_json::Value::from("teaching"));
        assert_ne!(initial["round_id"], teaching["round_id"]);
        assert_ne!(initial["role_run_seq"], teaching["role_run_seq"]);
        // 同一 node 上两轮仍是两条独立记录（不得互相覆盖身份）。
        assert_eq!(initial["node_id"], teaching["node_id"]);
        assert_eq!(initial["role_run_seq"], serde_json::Value::from(0));
        assert_eq!(teaching["role_run_seq"], serde_json::Value::from(4));
    }

    /// 登记簿契约：按 node_id 登记/取用/收口摘除，采集点只看到本轮的记录。
    #[test]
    fn registry_scopes_timing_by_node_id() {
        let registered = timing(
            "node-registry",
            "round-registry",
            1,
            RoundTimingKind::Teaching,
        );
        register_round_timing("node-registry", Arc::clone(&registered));
        assert!(active_round_timing("node-other").is_none());
        let active = active_round_timing("node-registry").expect("本轮已登记");
        active.mark_start_state(ROUND_TIMING_START_STARTED);
        active.mark_first_model_delta();
        let taken = take_round_timing("node-registry").expect("收口取到本轮记录");
        assert!(Arc::ptr_eq(&taken, &registered));
        assert!(
            take_round_timing("node-registry").is_none(),
            "收口必须摘登记（不得重复落盘）"
        );
        let record = taken.snapshot("completed");
        assert_eq!(record["node_id"], serde_json::Value::from("node-registry"));
        assert_eq!(record["start_state"], serde_json::Value::from("started"));
        assert!(record["ttft_ms"].as_u64().is_some());
    }
}
