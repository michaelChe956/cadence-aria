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
                    EngineEvent::HumanGateOpened { stage: _ } => {
                        let session_state = manager.engine.lock().await.build_session_state();
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
            engine.build_session_state()
        } else {
            self.durable_projection().0
        }
    }
    pub(super) fn broadcast(&self, message: WsOutMessage) {
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
        let recovery_baseline = attachments
            .iter()
            .any(|(_, degraded)| *degraded)
            .then(|| self.current_session_state())
            .and_then(|snapshot| serde_json::to_string(&snapshot).ok())
            .and_then(|baseline| inject_event_seq(baseline, seq));
        for (connection_id, degraded) in attachments {
            if degraded {
                if let Some(baseline) = recovery_baseline.as_deref() {
                    self.try_recover_degraded_attachment(&connection_id, baseline);
                }
            } else {
                self.try_send_live_event(&connection_id, &json);
            }
        }
    }

    /// 直播发送只允许 `try_send`：任何 attachment 都不能使 provider/router 等待。
    /// 队列满时仅标记该 attachment；不发送可被同样丢弃的 `ResyncRequired`。下一次
    /// 广播会先试投递带当前 event_seq 的全量 session_state，成功后恢复直播。
    fn try_send_live_event(&self, connection_id: &str, json: &str) {
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
    fn try_recover_degraded_attachment(&self, connection_id: &str, baseline: &str) {
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
            Err(mpsc::error::TrySendError::Full(_)) => {}
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
    pub(crate) fn broadcast_current_session_state(&self) {
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
            let state = engine.build_session_state();
            drop(engine);
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
            let state = engine.build_session_state();
            drop(engine);
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
        &self,
        record: &crate::product::models::WorkspaceSessionRecord,
    ) {
        {
            let mut engine = self.engine.lock().await;
            engine.apply_external_confirm_record(record);
        }
        self.broadcast_current_session_state();
    }
}
