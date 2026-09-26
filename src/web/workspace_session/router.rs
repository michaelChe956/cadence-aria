use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;

use crate::cross_cutting::streaming_provider::ChoiceRequestSource;
use crate::product::workspace_engine::EngineEvent;
use crate::web::workspace_ws_handler::{
    OutboundControl, map_engine_event, spawn_provider_run_from_event,
};
use crate::web::workspace_ws_types::WsOutMessage;

use super::manager::{WorkspaceSessionManager, inject_event_seq};

impl WorkspaceSessionManager {
    /// session-owned event router：只持有 manager 弱引用，registry 回收最后一个强引用后
    /// 即退出，避免 router 与 engine sender 构成自引用环。
    pub(super) fn spawn_event_router(
        self: &Arc<Self>,
        mut engine_rx: mpsc::Receiver<EngineEvent>,
        workspace_runs: crate::web::state::WorkspaceRunRegistry,
    ) {
        let manager = Arc::downgrade(self);
        tokio::spawn(async move {
            while let Some(event) = engine_rx.recv().await {
                let Some(manager) = manager.upgrade() else {
                    break;
                };
                match event {
                    EngineEvent::ProviderRunRequested { kind, node_id } => {
                        let outbound = manager
                            .state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .attachments
                            .values()
                            .next()
                            .map(|attachment| attachment.outbound_tx.clone())
                            .unwrap_or_else(|| {
                                // F-1：无活动 run 且无订阅者时仍以一次性出站通道接力 spawn。
                                // 事件照常入 journal，下一次 attach 经 cursor 回放或 snapshot 基线取回。
                                eprintln!(
                                    "[aria-broadcast] provider run requested without attachment; spawning with throwaway outbound session={}",
                                    manager.session_id
                                );
                                mpsc::channel(1).0
                            });
                        let run_context = manager.provider_run_context(workspace_runs.clone());
                        tokio::spawn(async move {
                            if let Err(message) = spawn_provider_run_from_event(
                                run_context,
                                kind,
                                node_id,
                                outbound.clone(),
                            )
                            .await
                            {
                                let _ = outbound.try_send(OutboundControl::Text(
                                    serde_json::to_string(&WsOutMessage::Error { message })
                                        .unwrap_or_else(|_| {
                                            "{\"type\":\"error\",\"message\":\"serialization failed\"}"
                                                .to_string()
                                        }),
                                ));
                            }
                        });
                    }
                    EngineEvent::HumanGateOpened { stage } => {
                        // C3/REQ-HTR-01：锁无关门开标志（Abort 矩阵在 engine 锁
                        // 被 in-flight 修订 run 持有时的非阻塞判定源）。
                        manager.set_human_confirm_gate_open(stage == "human_confirm");
                        let mut session_state = manager.engine.lock().await.build_session_state();
                        manager.project_automation_ownership(&mut session_state);
                        manager.broadcast(session_state);
                    }
                    EngineEvent::ArtifactBatchUpdate { mut updates } => {
                        updates.sort_by_key(|update| update.version);
                        for update in updates {
                            manager.broadcast(
                                crate::web::workspace_ws_handler::ws_artifact_update(
                                    update.version,
                                    update.payload,
                                ),
                            );
                        }
                    }
                    EngineEvent::ChoiceRequest {
                        id,
                        prompt,
                        options,
                        allow_multiple,
                        allow_free_text,
                        questions,
                        source,
                    } => {
                        let message = WsOutMessage::ChoiceRequest {
                            id,
                            prompt,
                            options: options
                                .into_iter()
                                .map(crate::web::workspace_ws_handler::ws_choice_option)
                                .collect(),
                            allow_multiple,
                            allow_free_text,
                            questions: questions
                                .into_iter()
                                .map(crate::web::workspace_ws_handler::ws_choice_question)
                                .collect(),
                            source: source.as_str().to_string(),
                        };
                        // F-24：广播前先登记挂起帧——本次广播若降级丢弃该帧，
                        // 挂起帧是 attach/重订阅/degraded 恢复补发的唯一来源。
                        if source != ChoiceRequestSource::TextFallback {
                            manager.register_pending_choice_frame(message.clone());
                        }
                        manager.broadcast(message);
                        // F-27R2（治本）：choice pending 变更即追加全量 session_state
                        // 广播——投影自带 pending_choice_requests，已连接 tab 的前端
                        // 对账（reconcilePendingChoiceRequests）是 choice_request 帧
                        // 降级丢失后唯一的补卡通道。帧序锚：choice_request 帧先入
                        // journal，随后的 session_state event_seq 恒更大；前端对
                        // session_state 无条件接受，无去重误杀。
                        manager.broadcast_choice_pending_state(
                            source != ChoiceRequestSource::TextFallback,
                        );
                    }
                    EngineEvent::ChoicePendingChanged => {
                        // F-27R2：应答命中挂起 choice（登记簿已同步摘除，durable
                        // 投影即时可见）——广播全量 session_state，前端对账
                        // 收敛已答卡。
                        manager.broadcast_choice_pending_state(true);
                    }
                    EngineEvent::HumanGateClosed { .. } => {
                        // C3/REQ-HTR-01：任何门关闭即摘门开标志（会话至多一个
                        // Active 门，见 enter_human_confirm 收口注释）。
                        manager.set_human_confirm_gate_open(false);
                        if let Some(message) = map_engine_event(event) {
                            manager.broadcast(message);
                        }
                    }
                    event => {
                        if let Some(message) = map_engine_event(event) {
                            manager.broadcast(message);
                        }
                    }
                }
            }
        });
    }

    /// 从序列化边界注入增量 wire 字段，避免侵入所有 `WsOutMessage` 变体。旧客户端
    /// 忽略未知字段；同一 stamped JSON 同时写 journal 并 fan-out 到所有 attachment。
    fn current_session_state(&self) -> WsOutMessage {
        if let Ok(engine) = self.engine.try_lock() {
            let mut session_state = engine.build_session_state();
            self.project_automation_ownership(&mut session_state);
            session_state
        } else {
            self.durable_projection().0
        }
    }
    pub(super) fn broadcast(self: &Arc<Self>, message: WsOutMessage) {
        let Ok(serialized) = serde_json::to_string(&message) else {
            eprintln!(
                "[aria-broadcast] serialize failed session={}",
                self.session_id
            );
            return;
        };
        let seq = self.next_event_seq.fetch_add(1, Ordering::Relaxed);
        let Some(json) = inject_event_seq(serialized, seq) else {
            eprintln!(
                "[aria-broadcast] event sequence injection failed session={}",
                self.session_id
            );
            return;
        };
        let keyframe = is_keyframe_message(&message);
        let attachments = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.journal.push(seq, json.clone());
            state
                .attachments
                .iter()
                .map(|(connection_id, attachment)| (connection_id.clone(), attachment.degraded))
                .collect::<Vec<_>>()
        };
        // C3/REQ-HTR-03：基线构造条件放宽——关键帧广播时即使暂无 degraded
        // 连接也预建（新降级连接的有界等待投递以基线恢复；非关键帧维持
        // 既有「存在 degraded 才建」的惰性）。
        let recovery_baseline = (keyframe || attachments.iter().any(|(_, degraded)| *degraded))
            .then(|| self.current_session_state())
            .and_then(|snapshot| serde_json::to_string(&snapshot).ok())
            .and_then(|baseline| inject_event_seq(baseline, seq));
        for (connection_id, degraded) in attachments {
            if degraded {
                if let Some(baseline) = recovery_baseline.as_deref() {
                    self.try_recover_degraded_attachment(&connection_id, baseline, keyframe, seq);
                }
            } else {
                self.try_send_live_event(&connection_id, &json, keyframe, seq, &recovery_baseline);
            }
        }
    }

    /// 直播发送只允许 `try_send`：任何 attachment 都不能使 provider/router 等待。
    /// 队列满时仅标记该 attachment；不发送可被同样丢弃的 `ResyncRequired`。下一次
    /// 广播会先试投递带当前 event_seq 的全量 session_state，成功后恢复直播。
    /// C3/REQ-HTR-03：关键帧（stage_change/session_state/human_gate_closed 族）
    /// 在本连接降级时额外起有界等待投递任务（两次有界退避后必发
    /// resync_required）——V0 现场「门开后引擎静默、下一次广播永不触发」的
    /// stale 停留由此收口。
    fn try_send_live_event(
        self: &Arc<Self>,
        connection_id: &str,
        json: &str,
        keyframe: bool,
        seq: u64,
        recovery_baseline: &Option<String>,
    ) {
        let sender = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.attachments.get(connection_id).and_then(|attachment| {
                (!attachment.degraded).then(|| attachment.outbound_tx.clone())
            })
        };
        let Some(sender) = sender else {
            return;
        };

        match sender.try_send(OutboundControl::Text(json.to_string())) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(attachment) = state.attachments.get_mut(connection_id)
                    && attachment.outbound_tx.same_channel(&sender)
                {
                    attachment.degraded = true;
                }
                drop(state);
                // 打点在状态锁外（lease-diagnostics 同款纪律）。
                self.record_degraded_enter(connection_id, seq);
                if keyframe && let Some(baseline) = recovery_baseline.as_deref() {
                    spawn_keyframe_delivery(
                        Arc::clone(self),
                        connection_id.to_string(),
                        sender,
                        baseline.to_string(),
                        seq,
                    );
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .attachments
                    .remove(connection_id);
            }
        }
    }

    /// 降级连接只尝试一次无等待投递；它收到的 session_state 即当前事件序号的基线，
    /// 因而即使溢出时的增量帧已丢失，也能安全继续接收后续单调 event_seq 帧。
    /// C3/REQ-HTR-03：关键帧的恢复尝试失败（队列仍满）时起有界等待投递——
    /// 引擎门开后静默、无后续广播触发恢复的窗口由此兜底。
    fn try_recover_degraded_attachment(
        self: &Arc<Self>,
        connection_id: &str,
        baseline: &str,
        keyframe: bool,
        seq: u64,
    ) {
        let sender = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state
                .attachments
                .get(connection_id)
                .and_then(|attachment| attachment.degraded.then(|| attachment.outbound_tx.clone()))
        };
        let Some(sender) = sender else {
            return;
        };

        match sender.try_send(OutboundControl::Text(baseline.to_string())) {
            Ok(()) => {
                {
                    let mut state = self
                        .state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if let Some(attachment) = state.attachments.get_mut(connection_id)
                        && attachment.degraded
                        && attachment.outbound_tx.same_channel(&sender)
                    {
                        attachment.degraded = false;
                    }
                }
                // 打点在状态锁外（lease-diagnostics 同款纪律）。
                self.record_degraded_exit(connection_id, seq);
                // F-24（0484 现场锚）：降级期间被 try_send 丢弃的 choice 帧不在
                // session_state 里——恢复时必须补发挂起帧，否则连接全程在线的
                // 用户永远看不到卡。等待式投递放独立任务：队列腾出即送达，
                // 不依赖后续广播、也绝不让 router 等待（pending_choice_frames
                // 内部会再取 state 锁，必须在上述锁释放后调用）。
                let pending_choices = self.pending_choice_frames();
                if !pending_choices.is_empty() {
                    tokio::spawn(async move {
                        for choice in pending_choices {
                            let Ok(json) = serde_json::to_string(&choice) else {
                                continue;
                            };
                            if sender.send(OutboundControl::Text(json)).await.is_err() {
                                break;
                            }
                        }
                    });
                }
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                if keyframe {
                    spawn_keyframe_delivery(
                        Arc::clone(self),
                        connection_id.to_string(),
                        sender,
                        baseline.to_string(),
                        seq,
                    );
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .attachments
                    .remove(connection_id);
            }
        }
    }

    /// F-23：恢复链整流后向全部连接广播当前快照（manager 创建时无订阅者，
    /// 广播仍入 journal 供后续 cursor 回放）。
    pub(crate) fn broadcast_current_session_state(self: &Arc<Self>) {
        self.broadcast(self.current_session_state());
    }

    /// F-27R2：choice 挂起集变更（登记/移除）的 session_state 广播面——已连接
    /// tab 的前端对账（reconcilePendingChoiceRequests）以 pending_choice_requests
    /// 投影补卡/收卡，这是 choice_request 帧降级丢失后唯一的恢复通道（投影
    /// 取法对照 F-25b broadcast_http_confirm 的 current_session_state/try_lock
    /// 回退先例）。
    ///
    /// - 引擎锁空闲：活引擎投影即时广播一帧。
    /// - run 持锁：`immediate_durable=true` 先按 durable 投影即时广播（provider
    ///   pending 经进程级登记簿跨实例可见，登记/移除均先于事件可达），并调度
    ///   锁释放后的活引擎补帧——收敛 durable 投影恒空的引擎内存面（如并发
    ///   挂起的 TextFallback pending）。`immediate_durable=false`（TextFallback
    ///   登记）跳过即时帧：该 pending 只活引擎内存，durable 帧必缺卡，只会
    ///   闪断（choice_request 帧已先行渲染）。
    fn broadcast_choice_pending_state(self: &Arc<Self>, immediate_durable: bool) {
        if let Ok(engine) = self.engine.try_lock() {
            let mut state = engine.build_session_state();
            drop(engine);
            self.project_automation_ownership(&mut state);
            self.broadcast(state);
            return;
        }
        if immediate_durable {
            self.broadcast(self.durable_projection().0);
        }
        let engine = self.engine.clone();
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let engine = engine.lock().await;
            let mut state = engine.build_session_state();
            drop(engine);
            manager.project_automation_ownership(&mut state);
            manager.broadcast(state);
        });
    }

    /// F-25b：HTTP confirm 端点完成 durable 写后的广播面（对照 F-09
    /// HumanGateOpened 的 engine 广播先例）。story/design AuthorConfirm 的确认
    /// 通路在 handler 层直写 durable，此前后不广播——发请求 tab 靠乐观
    /// setSessionStatus 收敛，其他 tab/重连只能整页 reload。现先按 record 同步
    /// 引擎内存投影，再向全部 attachment 广播全量 session_state（顺带入
    /// journal，后续 cursor 回放同样取到 confirmed 态）。
    pub(crate) async fn broadcast_http_confirm(
        self: &Arc<Self>,
        record: &crate::product::models::WorkspaceSessionRecord,
    ) {
        {
            let mut engine = self.engine.lock().await;
            engine.apply_external_confirm_record(record);
        }
        self.broadcast_current_session_state();
    }
}

// ── C3/REQ-HTR-03：degraded 连接关键帧投递 ──────────────────────────────────

/// 有界等待单次上限。
const KEYFRAME_DELIVER_WAIT: std::time::Duration = std::time::Duration::from_millis(750);
/// 两次尝试之间的退避。
const KEYFRAME_DELIVER_BACKOFF: std::time::Duration = std::time::Duration::from_millis(250);
/// 有界尝试次数（全局约束：有界（如 2 次退避）后必发 resync_required）。
const KEYFRAME_DELIVER_ATTEMPTS: usize = 2;

/// 关键帧白名单（design D3）：stage_change / session_state（含 HumanGateOpened
/// 触发的门开全量广播）/ human_gate_closed（abandon 终止后引擎同样静默）。
/// F-59：choice_request（非 text_fallback 源）入列——修订/生成运行发出
/// AskUserQuestion 后引擎静默等待应答，choice 帧被降级丢弃且无后续广播
/// 时同样「stale 至 provider_choice_wait_timeout」（issue_0002 现场）；
/// 恢复基线自带 pending_choice_requests 投影，前端对账即补弹卡。
/// text_fallback 不入列：该源无 provider 挂起等待界，投影
///（pending_author_choice）是其唯一恢复面，由 F-27 广播链覆盖。
/// 流式/增量帧不入列——它们可由恢复基线与后续广播覆盖，等待式投递只保留给
/// 「丢了它 + 引擎静默 = stale」的帧。
fn is_keyframe_message(message: &WsOutMessage) -> bool {
    let gate_frame = matches!(
        message,
        WsOutMessage::StageChange { .. }
            | WsOutMessage::SessionState { .. }
            | WsOutMessage::HumanGateClosed { .. }
    );
    let pending_choice_frame = matches!(
        message,
        WsOutMessage::ChoiceRequest { source, .. }
            if source != crate::cross_cutting::streaming_provider::ChoiceRequestSource::TextFallback
                .as_str()
    );
    gate_frame || pending_choice_frame
}

/// 关键帧有界等待投递：两次有界退避尝试送达恢复基线；均失败则必发
/// resync_required（等待不设界——它是「帧被吞且无任何信号」的最后防线；
/// 客户端恢复读取即送达、通道关闭即退出，parked 任务不占 router）。
/// 每次发送前复查 degraded 态：连接若已按后续广播恢复直播，本任务的旧
/// 基线不再投递（防 event_seq 回退覆盖新状态）。
fn spawn_keyframe_delivery(
    manager: Arc<WorkspaceSessionManager>,
    connection_id: String,
    sender: mpsc::Sender<OutboundControl>,
    baseline: String,
    seq: u64,
) {
    let resync = serde_json::to_string(&WsOutMessage::ResyncRequired { event_seq: seq })
        .unwrap_or_else(|_| "{\"type\":\"resync_required\",\"event_seq\":0}".to_string());
    tokio::spawn(async move {
        for attempt in 0..KEYFRAME_DELIVER_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(KEYFRAME_DELIVER_BACKOFF).await;
            }
            if !manager.attachment_is_degraded(&connection_id) {
                return;
            }
            if tokio::time::timeout(
                KEYFRAME_DELIVER_WAIT,
                sender.send(OutboundControl::Text(baseline.clone())),
            )
            .await
            .is_ok_and(|result| result.is_ok())
            {
                return;
            }
        }
        if !manager.attachment_is_degraded(&connection_id) {
            return;
        }
        let _ = sender.send(OutboundControl::Text(resync)).await;
    });
}

#[cfg(test)]
mod degraded_keyframe_tests {
    use super::*;

    /// REQ-HTR-03：白名单只覆盖门开/门关/全量基线帧。
    #[test]
    fn keyframe_whitelist_covers_gate_critical_frames_only() {
        assert!(is_keyframe_message(&WsOutMessage::StageChange {
            stage: "human_confirm".to_string()
        }));
        assert!(is_keyframe_message(&WsOutMessage::HumanGateClosed {
            decision: "terminate".to_string(),
            stage: "completed".to_string()
        }));
        assert!(!is_keyframe_message(&WsOutMessage::ProviderStatus {
            status: crate::web::workspace_ws_types::WsProviderStatus::Running
        }));
        assert!(!is_keyframe_message(&WsOutMessage::StreamChunk {
            role: "author".to_string(),
            content: "chunk".to_string(),
            node_id: None
        }));
        assert!(!is_keyframe_message(&WsOutMessage::TimelineNodeUpdated {
            node_id: "node".to_string(),
            status: crate::web::workspace_ws_types::TimelineNodeStatus::Active,
            summary: None,
            completed_at: None
        }));
    }
    /// F-59：choice_request（非 text_fallback 源）入关键帧白名单——degraded/
    /// 断线期间到达的 choice 卡与门开帧同级：「丢了它 + 引擎静默（等待应答）=
    /// stale 至 provider_choice_wait_timeout」。text_fallback 不入列：该源
    /// 无 provider 挂起等待界，且投影（pending_author_choice）是其恢复面。
    #[test]
    fn keyframe_whitelist_covers_pending_choice_frames() {
        for source in ["ask_user_question", "request_user_input", "provider_choice"] {
            assert!(
                is_keyframe_message(&choice_frame_with_source(source)),
                "source={source} 的 choice_request 必须按关键帧投递"
            );
        }
        assert!(
            !is_keyframe_message(&choice_frame_with_source("text_fallback")),
            "text_fallback 无等待界且以投影为恢复面，不入关键帧白名单"
        );
    }

    fn choice_frame_with_source(source: &str) -> WsOutMessage {
        WsOutMessage::ChoiceRequest {
            id: "choice_keyframe_probe".to_string(),
            prompt: "验收口径歧义需要用户裁定".to_string(),
            options: Vec::new(),
            allow_multiple: false,
            allow_free_text: false,
            questions: Vec::new(),
            source: source.to_string(),
        }
    }
}
