use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;

use crate::cross_cutting::streaming_provider::ChoiceRequestSource;
use crate::product::workspace_engine::EngineEvent;
use crate::web::workspace_ws_handler::{
    OutboundControl, map_engine_event, spawn_provider_run_from_event,
};
use crate::web::workspace_ws_types::WsOutMessage;

use super::manager::{DurableProjectionError, WorkspaceSessionManager, inject_event_seq};

// ─── 批 2 P4（REQ-WIGA-12）：plan 阶段停点/失败事实写清（router 观察面）───

/// 批 2 P4：精确绑定 enabled auto 授权的 plan 会话才写事实（手动/未绑定/
/// 禁用/旧代零写入）；键由写点从 durable 事实派生（稳定 key，重复不增项）。
/// router 无调用方可回传写失败——打点留痕、不吞静默；写者上抛语义由
/// continue/control 调用方承担。
fn upsert_plan_stop_fact(
    manager: &WorkspaceSessionManager,
    key: String,
    kind: crate::product::models::automation::AutomationAttentionKind,
    reason_code: &str,
    choice_id: Option<String>,
) {
    use crate::product::models::automation::AutomationAttentionFact;
    let record = &manager.session_record;
    let Ok(Some((_enrollment, binding))) =
        crate::web::plan_generation::bound_auto_enrollment_for_plan_session(
            &manager.app_paths,
            &record.project_id,
            &record.issue_id,
            &manager.session_id,
        )
    else {
        return;
    };
    let fact = AutomationAttentionFact {
        key,
        kind,
        stage: crate::product::models::automation::AutomationAttentionStage::Plan,
        plan_id: Some(binding.plan_id.clone()),
        session_id: Some(binding.session_id.clone()),
        attempt_id: None,
        reason_code: reason_code.to_string(),
        occurred_at: chrono::Utc::now().to_rfc3339(),
        choice_id,
        gate_id: None,
    };
    if let Err(message) = crate::web::plan_generation::upsert_plan_attention_fact(
        &manager.app_paths,
        &record.project_id,
        &record.issue_id,
        &binding,
        fact,
    ) {
        eprintln!(
            "[aria-plan-attention] upsert failed session={}: {message}",
            manager.session_id
        );
    }
}

/// 批 2 N1（REQ-WIGA-12）：答后清题的挂起集判据（choice id 当前仍挂起 =
/// 事实保留）。活引擎空闲时直读引擎投影；run 持锁时走 durable 投影
///（provider 挂起集在进程级登记簿，跨引擎实例可见、登记/移除均先于事件
/// 可达，见 pending_choices.rs）；两路读取均失败返回 `None`——fail-closed
/// 不清题（宁留待办通知，不误清可答题）。
fn pending_choice_ids_for_fact_clearing(
    manager: &WorkspaceSessionManager,
) -> Option<std::collections::HashSet<String>> {
    let state = if let Ok(engine) = manager.engine.try_lock() {
        let state = engine.build_session_state();
        drop(engine);
        state
    } else {
        manager.durable_projection().ok()?.session_state
    };
    match state {
        WsOutMessage::SessionState {
            pending_choice_requests,
            ..
        } => Some(
            pending_choice_requests
                .into_iter()
                .map(|request| request.id)
                .collect(),
        ),
        _ => Some(std::collections::HashSet::new()),
    }
}

/// 批 2 P4：关门/继续成功后按稳定 key 清事实（幂等；清失败打点留痕）。
fn resolve_plan_stop_fact(manager: &WorkspaceSessionManager, key: &str) {
    let record = &manager.session_record;
    let Ok(Some((_enrollment, binding))) =
        crate::web::plan_generation::bound_auto_enrollment_for_plan_session(
            &manager.app_paths,
            &record.project_id,
            &record.issue_id,
            &manager.session_id,
        )
    else {
        return;
    };
    if let Err(message) = crate::web::plan_generation::resolve_plan_attention_fact(
        &manager.app_paths,
        &record.project_id,
        &record.issue_id,
        &binding,
        key,
    ) {
        eprintln!(
            "[aria-plan-attention] resolve failed session={}: {message}",
            manager.session_id
        );
    }
}

/// HumanGateOpened 的 durable 门快照预算读取：manual 预算已耗尽 →
/// review_limit（评审超限停点），否则 plan_question（choice/主动人工门同族
/// 的对话停点）；读不到快照时按 plan_question 通知（fail-open）。
fn plan_gate_fact_kind(
    manager: &WorkspaceSessionManager,
) -> crate::product::models::automation::AutomationAttentionKind {
    let snapshot_budget =
        crate::product::lifecycle_store::LifecycleStore::new(manager.app_paths.clone())
            .get_workspace_session(&manager.session_id)
            .ok()
            .and_then(|record| record.human_gate_snapshot)
            .map(|snapshot| snapshot.manual_repairs_remaining);
    match snapshot_budget {
        Some(0) => crate::product::models::automation::AutomationAttentionKind::ReviewLimit,
        _ => crate::product::models::automation::AutomationAttentionKind::PlanQuestion,
    }
}

/// TimelineNodeUpdated(Failed) 的 durable Failed 收敛核验：仅当会话已 durable
/// Failed（SC phase Failed）才写 provider 失败事实（键不含节点 id——单会话
/// 一条，重复停点 upsert 不增项、occurred_at 首写冻结）。
fn record_plan_provider_failure_if_durable(manager: &WorkspaceSessionManager) {
    let Ok(record) =
        crate::product::lifecycle_store::LifecycleStore::new(manager.app_paths.clone())
            .get_workspace_session(&manager.session_id)
    else {
        return;
    };
    if record.workspace_type != crate::product::models::WorkspaceType::WorkItemPlan
        || record.status != crate::product::models::WorkspaceSessionStatus::Failed
        || record.single_candidate_phase
            != Some(crate::product::models::SingleCandidatePhase::Failed)
    {
        return;
    }
    upsert_plan_stop_fact(
        manager,
        crate::web::plan_generation::plan_failed_node_fact_key(&manager.session_id),
        crate::product::models::automation::AutomationAttentionKind::AutomationFailed,
        "plan_provider_failed",
        None,
    );
}
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
                        // 批 2 P4（REQ-WIGA-12）：plan 人工门开 = durable 对话停点
                        // 事实（review_limit/plan_question 按 durable 预算判别；
                        // 精确绑定 auto 授权才写，手动零写入）。
                        if stage == "human_confirm" {
                            upsert_plan_stop_fact(
                                &manager,
                                crate::web::plan_generation::plan_gate_fact_key(
                                    &manager.session_id,
                                ),
                                plan_gate_fact_kind(&manager),
                                "plan_human_gate_open",
                                None,
                            );
                        }
                        // T22 r3 根因⑤(b45 现场 r2/r3,r16 死锁家族 router 臂):
                        // 本臂曾以 `engine.lock().await` 阻塞等锁——provider run
                        // 任务全程持锁,SC 修订→复评循环内门重开事件在锁内发出,
                        // router 阻塞即停止消费,engine 事件通道(64)被后续流帧
                        // 填满后 run 的 send 也阻塞,循环等待令 WS 零帧(r3 帧
                        // 日志:reviewer 已 Completed 而连接零出帧含 pong)。改走
                        // F-27R2/F-09 同款非阻塞面:try_lock 活引擎即时广播;锁
                        // 被持时按 durable 投影即时广播+锁释放后活引擎补帧。
                        manager.broadcast_gate_opened_state();
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
                        // 批 2 P4（REQ-WIGA-12）：挂起 choice = durable plan 对话
                        // 停点事实（逐 choice 一条；text_fallback 的恢复面是
                        // pending_author_choice 投影，不写事实）。
                        if source != ChoiceRequestSource::TextFallback {
                            upsert_plan_stop_fact(
                                &manager,
                                crate::web::plan_generation::plan_choice_fact_key(
                                    &manager.session_id,
                                    &id,
                                ),
                                crate::product::models::automation::AutomationAttentionKind::PlanQuestion,
                                "plan_choice_pending",
                                Some(id.clone()),
                            );
                        }
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
                        // 批 2 N1（REQ-WIGA-12）：答后清题——挂起集（引擎进程
                        // 级登记簿，移除先于事件可达）外的本会话 plan choice
                        // 事实按稳定 key resolve；durable Failed/review_limit
                        // 停点事实不碰。
                        if let Some(still_pending) = pending_choice_ids_for_fact_clearing(&manager)
                        {
                            manager.resolve_answered_plan_choice_facts(&still_pending);
                        }
                    }
                    // A1b2（设计 §4.4 / A2.2 移交）：durable LC 恢复事实变化的
                    // authoritative 广播接线——立即 try_lock 活引擎，忙时按最新
                    // durable record 纯投影（不出生 record）；沿 journal/
                    // event_seq 广播，同一存活 manager 的 driver/observer/新
                    // attach 看到相同 recovery_id/reason/allowed_actions。读取
                    // 失败广播 typed ProtocolError，不发伪 SessionState。mapping
                    // 面保持 None（内部事件不是 wire delta）。
                    EngineEvent::LcResumeRecoveryChanged => {
                        manager.broadcast_current_session_state();
                    }
                    EngineEvent::HumanGateClosed { .. } => {
                        // C3/REQ-HTR-01：任何门关闭即摘门开标志（会话至多一个
                        // Active 门，见 enter_human_confirm 收口注释）。
                        manager.set_human_confirm_gate_open(false);
                        // 批 2 P4（REQ-WIGA-12）：门关闭 = plan 停点事实 resolve
                        //（通知消失；幂等）。
                        resolve_plan_stop_fact(
                            &manager,
                            &crate::web::plan_generation::plan_gate_fact_key(&manager.session_id),
                        );
                        if let Some(message) = map_engine_event(event) {
                            manager.broadcast(message);
                        }
                    }
                    EngineEvent::TimelineNodeUpdated { ref status, .. } => {
                        // 批 2 P4（REQ-WIGA-12）：durable Failed 收敛（provider
                        // 失败）→ automation_failed 事实（无活窗口仍可通知）。
                        if *status == crate::web::workspace_ws_types::TimelineNodeStatus::Failed {
                            record_plan_provider_failure_if_durable(&manager);
                        }
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
    ///
    /// A1b2（设计 §4.4）：authoritative session_state 的唯一取帧面——engine 锁
    /// 空闲取活引擎投影；忙时读最新 durable record 纯投影（不等锁、不出生
    /// record）。读取失败 typed 上抛，由广播面转 ProtocolError 或缺基线
    /// resync，绝不发伪 SessionState。
    fn current_session_state(&self) -> Result<WsOutMessage, DurableProjectionError> {
        if let Ok(engine) = self.engine.try_lock() {
            let mut session_state = engine.build_session_state();
            self.project_automation_ownership(&mut session_state);
            Ok(session_state)
        } else {
            self.durable_projection()
                .map(|projection| projection.session_state)
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
        // A1b2（设计 §4.4）：基线构建失败（durable 纯读失败）时置 None——
        // 不得用旧可启动 snapshot 补基线；关键帧走 resync_required 终态。
        let recovery_baseline = (keyframe || attachments.iter().any(|(_, degraded)| *degraded))
            .then(|| self.current_session_state().ok())
            .flatten()
            .and_then(|snapshot| serde_json::to_string(&snapshot).ok())
            .and_then(|baseline| inject_event_seq(baseline, seq));
        for (connection_id, degraded) in attachments {
            if degraded {
                if let Some(baseline) = recovery_baseline.as_deref() {
                    self.try_recover_degraded_attachment(&connection_id, baseline, keyframe, seq);
                } else if keyframe {
                    // A1b2：关键帧基线缺失（durable 纯读失败）——degraded
                    // 连接按有界投递失败终态发 resync_required。
                    self.send_resync_required(&connection_id, seq);
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
                if keyframe {
                    if let Some(baseline) = recovery_baseline.as_deref() {
                        spawn_keyframe_delivery(
                            Arc::clone(self),
                            connection_id.to_string(),
                            sender,
                            baseline.to_string(),
                            seq,
                        );
                    } else {
                        // A1b2（设计 §4.4）：关键帧被降级丢弃且无可投基线
                        //（durable 纯读失败）——必发 resync_required，不得
                        // 用旧可启动 snapshot 补基线。
                        self.send_resync_required(connection_id, seq);
                    }
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

    /// A1b2（设计 §4.4）：关键帧基线缺失（durable 纯读失败）时的 degraded
    /// 终态信号——非阻塞投递；队列仍满则维持 degraded，等待下一次广播/
    /// 重连恢复。
    fn send_resync_required(&self, connection_id: &str, seq: u64) {
        let sender = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state
                .attachments
                .get(connection_id)
                .map(|attachment| attachment.outbound_tx.clone())
        };
        if let Some(sender) = sender {
            let _ = sender.try_send(OutboundControl::Text(resync_required_json(seq)));
        }
    }

    /// F-23：恢复链整流后向全部连接广播当前快照（manager 创建时无订阅者，
    /// 广播仍入 journal 供后续 cursor 回放）。
    /// A1b2（设计 §4.4）：快照读取失败广播 typed ProtocolError
    ///（workspace_durable_projection_unavailable），不填 Open/null/ownership
    /// 假态；event_seq 沿 journal，客户端经 cursor/重连恢复。

    /// T22 r3 根因⑤:HumanGateOpened 的非阻塞广播面(与 F-27R2
    /// `broadcast_choice_pending_state` 同构)——try_lock 活引擎即时广播;
    /// 锁被 provider run 持有(run 任务全期持锁)时按 durable 投影即时广播,
    /// 并调度锁释放后的活引擎补帧收敛内存面。绝不在 router 事件臂内阻塞
    /// 等 engine 锁:run 内发出的门事件+通道容量上限=循环等待(WS 零帧)。
    fn broadcast_gate_opened_state(self: &Arc<Self>) {
        if let Ok(engine) = self.engine.try_lock() {
            let mut state = engine.build_session_state();
            drop(engine);
            self.project_automation_ownership(&mut state);
            self.broadcast(state);
            return;
        }
        match self.durable_projection() {
            Ok(projection) => self.broadcast(projection.session_state),
            // A1b2(设计 §4.4):durable 纯读失败发 typed ProtocolError,不发伪
            // session_state;锁释放后的活引擎补帧仍会调度。
            Err(error) => self.broadcast(error.unavailable_frame(&self.session_id)),
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
    pub(crate) fn broadcast_current_session_state(self: &Arc<Self>) {
        match self.current_session_state() {
            Ok(frame) => self.broadcast(frame),
            Err(error) => self.broadcast(error.unavailable_frame(&self.session_id)),
        }
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
            match self.durable_projection() {
                Ok(projection) => self.broadcast(projection.session_state),
                // A1b2（设计 §4.4）：durable 纯读失败发 typed ProtocolError，
                // 不以伪 session_state 补帧；锁释放后的活引擎补帧仍会调度。
                Err(error) => self.broadcast(error.unavailable_frame(&self.session_id)),
            }
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

/// resync_required 帧文本（序列化失败兜底保持 wire 形状）。
fn resync_required_json(seq: u64) -> String {
    serde_json::to_string(&WsOutMessage::ResyncRequired { event_seq: seq })
        .unwrap_or_else(|_| "{\"type\":\"resync_required\",\"event_seq\":0}".to_string())
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
    let resync = resync_required_json(seq);
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

// ---------------------------------------------------------------------------
// 批 2 N1（REQ-WIGA-12）：ChoicePendingChanged 答后清题（router 全链路）。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod n1_tests {
    use std::sync::Arc;

    use super::WorkspaceSessionManager;
    use crate::product::issue_automation_store::IssueAutomationStore;
    use crate::product::logical_codebase::EnrollmentTarget;
    use crate::product::models::automation::{
        AutomationAttentionFact, AutomationAttentionKind, AutomationAttentionStage,
        EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand,
    };
    use crate::product::models::{IssueWorkItemPlanOptions, ProviderName};

    const N1_PROJECT: &str = "project_test";
    const N1_ISSUE: &str = "issue_test";
    const N1_PLAN: &str = "plan_n1_router";

    async fn wait_for_facts(
        store: &IssueAutomationStore,
        predicate: impl Fn(&[AutomationAttentionFact]) -> bool,
        label: &str,
    ) -> Vec<AutomationAttentionFact> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let facts = store
                .get(N1_PROJECT, N1_ISSUE)
                .unwrap()
                .expect("enrollment present")
                .attention_facts;
            if predicate(&facts) {
                return facts;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {label}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    fn fact(
        session_id: &str,
        key: String,
        kind: AutomationAttentionKind,
        choice_id: Option<&str>,
    ) -> AutomationAttentionFact {
        AutomationAttentionFact {
            key,
            kind,
            stage: AutomationAttentionStage::Plan,
            plan_id: Some(N1_PLAN.to_string()),
            session_id: Some(session_id.to_string()),
            attempt_id: None,
            reason_code: if choice_id.is_some() {
                "plan_choice_pending".to_string()
            } else {
                "plan_provider_failed".to_string()
            },
            occurred_at: "2026-10-16T00:00:00Z".to_string(),
            choice_id: choice_id.map(str::to_string),
            gate_id: None,
        }
    }

    /// 红测③（router 全链路）：ChoicePendingChanged 后挂起集（引擎进程级
    /// 登记簿）外的 choice 事实清、durable Failed/review_limit 不误清。
    /// 测试现场无 provider 驱动 → 登记簿恒空 → 全部 choice 事实视为已答。
    #[tokio::test]
    async fn choice_pending_changed_clears_answered_choice_facts_full_path() {
        let manager: Arc<WorkspaceSessionManager> =
            WorkspaceSessionManager::test_fixture_with_event_router("workspace_session_n1_router");
        let store = IssueAutomationStore::new(manager.app_paths.clone());
        store
            .compare_and_set(
                N1_PROJECT,
                N1_ISSUE,
                None,
                EnrollmentWriteCommand::Enable {
                    selection_key: "n1-router".to_string(),
                    source: EnrollmentSource {
                        stories: Vec::new(),
                        designs: Vec::new(),
                    },
                    options: EnrollmentOptions {
                        author_provider: ProviderName::Fake,
                        reviewer_provider: ProviderName::Fake,
                        review_rounds: 0,
                        superpowers_enabled: false,
                        openspec_enabled: false,
                        plan_approval: crate::product::models::automation::PlanApprovalPolicy::Auto,
                        plan_options: IssueWorkItemPlanOptions {
                            include_integration_tests: false,
                            include_e2e_tests: false,
                            force_frontend_backend_split: false,
                            require_execution_plan_confirm: false,
                        },
                    },
                    target: EnrollmentTarget::SingleRepository {
                        repository_id: "repo_n1".to_string(),
                    },
                },
            )
            .expect("enable enrollment");
        store
            .bind_plan(N1_PROJECT, N1_ISSUE, 1, N1_PLAN, &manager.session_id)
            .expect("bind plan");
        let enrollment = store
            .get(N1_PROJECT, N1_ISSUE)
            .unwrap()
            .expect("enrollment present");
        let binding = crate::web::advance_plan::load_current_enrollment_binding(&enrollment)
            .expect("current binding");
        let choice_a = crate::web::plan_generation::plan_choice_fact_key(
            &manager.session_id,
            "choice_answered_a",
        );
        let choice_b = crate::web::plan_generation::plan_choice_fact_key(
            &manager.session_id,
            "choice_answered_b",
        );
        let gate_key = crate::web::plan_generation::plan_gate_fact_key(&manager.session_id);
        let failed_key =
            crate::web::plan_generation::plan_failed_node_fact_key(&manager.session_id);
        for seeded in [
            fact(
                &manager.session_id,
                choice_a,
                AutomationAttentionKind::PlanQuestion,
                Some("choice_answered_a"),
            ),
            fact(
                &manager.session_id,
                choice_b,
                AutomationAttentionKind::PlanQuestion,
                Some("choice_answered_b"),
            ),
            fact(
                &manager.session_id,
                gate_key.clone(),
                AutomationAttentionKind::ReviewLimit,
                None,
            ),
            fact(
                &manager.session_id,
                failed_key.clone(),
                AutomationAttentionKind::AutomationFailed,
                None,
            ),
        ] {
            store
                .upsert_automation_attention_fact(N1_PROJECT, N1_ISSUE, &binding, seeded)
                .expect("seed fact");
        }

        manager
            .engine_tx()
            .send(crate::product::workspace_engine::EngineEvent::ChoicePendingChanged)
            .await
            .expect("send choice pending changed");

        let remaining = wait_for_facts(
            &store,
            |facts| {
                facts
                    .iter()
                    .all(|fact| fact.kind != AutomationAttentionKind::PlanQuestion)
            },
            "answered choice facts cleared",
        )
        .await;
        let mut remaining_keys = remaining
            .into_iter()
            .map(|fact| fact.key)
            .collect::<Vec<_>>();
        remaining_keys.sort();
        assert_eq!(
            remaining_keys,
            vec![failed_key, gate_key],
            "答后清题；durable Failed/review_limit 不误清"
        );
    }
}
