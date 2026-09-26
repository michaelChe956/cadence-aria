//! connection attachment 生命周期：登记/激活/初帧补发/重订阅/摘除。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::mpsc;

use super::{
    Attachment, WorkspaceSessionManager,
    choices::{choice_frame_id, choice_ids_in_frames},
    inject_event_seq,
};
use crate::web::workspace_session::ConnectionRole;
use crate::web::workspace_ws_handler::OutboundControl;
use crate::web::workspace_ws_types::WsOutMessage;

impl WorkspaceSessionManager {
    /// registry 在 sessions 互斥中调用。新连接先进入 pending 表：在首条入站裁决
    /// 前绝不接收直播，避免直播帧越过初帧或 cursor 回放。
    pub(crate) fn register_attachment(
        &self,
        connection_id: &str,
        outbound_tx: mpsc::Sender<OutboundControl>,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(message) = state.recovery_error.clone() {
            let _ = outbound_tx.try_send(OutboundControl::Text(
                serde_json::to_string(&WsOutMessage::Error { message }).unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"serialization failed\"}".to_string()
                }),
            ));
        }
        // REQ-DLS-04：attach 对租约零效应——不获取、不快照、不回滚。role 在
        // Hello 前维持 legacy Driver 缺省；lease_epoch 仅记录 attach 时刻的当前
        // epoch，由 hello 显式获取或首写自愈（REQ-DLS-01）在同一状态锁内刷新。
        let lease_epoch = state.lease.epoch;
        state.pending_attachments.insert(
            connection_id.to_string(),
            Attachment {
                outbound_tx,
                degraded: false,
                role: ConnectionRole::Driver,
                after_event_seq: None,
                lease_epoch,
            },
        );
    }

    /// 仅在初帧已排队、或 cursor 分支即将冻结 journal 时转为直播 attachment。
    pub(crate) fn activate_attachment(&self, connection_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(attachment) = state.pending_attachments.remove(connection_id) {
            state
                .attachments
                .insert(connection_id.to_string(), attachment);
        }
    }

    /// REQ-WCR-03：连接关闭只摘除 attachment；若它持有 lease，仅撤销该授权。
    /// 连接关闭绝不取消 run、不写入 durable 终态也不改写 engine 状态。
    pub(crate) async fn handle_connection_closed(self: &Arc<Self>, connection_id: &str) {
        let released_epoch = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let released = state.lease.revoke_if_holder(connection_id);
            state.pending_attachments.remove(connection_id);
            state.attachments.remove(connection_id);
            released.then_some(state.lease.epoch)
        };
        if let Some(epoch) = released_epoch {
            // REQ-DLS-03：holder 释放打点（锁外、失败零影响）。
            self.lease_diagnostics.record_release(connection_id, epoch);
        }
        self.maybe_recycle().await;
    }

    /// 返回已登记 attachment 的 initial snapshot 与已恢复 choice。
    pub(crate) async fn attached_session_state(&self) -> (WsOutMessage, Option<WsOutMessage>) {
        for attempt in 0..2 {
            if let Ok(engine) = self.engine.try_lock() {
                let mut session_state = engine.build_session_state();
                self.project_automation_ownership(&mut session_state);
                return (
                    session_state,
                    engine.pending_author_choice_request_message(),
                );
            }
            if attempt == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }
        let (mut session_state, pending_frame) = self.durable_projection_offload().await;
        self.stamp_pending_choice_run_ids(&mut session_state);
        (session_state, pending_frame)
    }

    /// 初帧 attach 基线序号：snapshot 构建时刻的当前 event_seq。初帧投递时若存在
    /// 活跃 run 补发窗口，基线会进一步压到窗口首事件之前（见
    /// `activate_attachment_with_initial_frames`），保证补发帧不会被客户端按
    /// event_seq 去重吞掉。
    pub(crate) fn attach_baseline_seq(&self) -> u64 {
        self.next_event_seq
            .load(Ordering::Relaxed)
            .saturating_sub(1)
    }

    /// 初帧投递的帧序列：同一状态锁内完成 pending→attachments 激活与 journal 补发
    /// 窗口冻结，投递由调用方在锁外进行（与 `resubscribe` 的顺序契约同构）。
    ///
    /// 恢复触发的 run 在连接登记 attachment 之前就已开始——其事件（如恢复出的
    /// Provider Prompt）先于任何连接侧标记进入 journal。无 cursor 的旧客户端在
    /// grace/首条入站消息激活时必须经活跃 run 窗口补发才能取回这些事件；快照
    /// 基线压到首个补发事件之前，重叠帧由客户端按 event_seq 去重。
    pub(crate) fn activate_attachment_with_initial_frames(
        &self,
        connection_id: &str,
        session_state: WsOutMessage,
        choice: Option<WsOutMessage>,
        attach_seq: u64,
    ) -> Vec<String> {
        let active_window = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(attachment) = state.pending_attachments.remove(connection_id) {
                state
                    .attachments
                    .insert(connection_id.to_string(), attachment);
            }
            state.journal.active_run_window()
        };
        let baseline_seq = match &active_window {
            Some((first_seq, _)) => (*first_seq).saturating_sub(1),
            None => attach_seq,
        };
        let mut frames = Vec::new();
        if let Some(json) = serde_json::to_string(&session_state)
            .ok()
            .and_then(|json| inject_event_seq(json, baseline_seq))
        {
            frames.push(json);
        }
        if let Some(choice) = choice
            && let Ok(json) = serde_json::to_string(&choice)
        {
            frames.push(json);
        }
        // F-24：补发活跃 run 的挂起 provider choice。journal 窗口缺失/截断或
        // 初帧路径无窗口时（`pending_author_choice_request_message` 只覆盖
        // TextFallback 面），挂起帧是用户可见性的唯一来源。
        let replayed = active_window
            .as_ref()
            .map(|(_, events)| choice_ids_in_frames(events))
            .unwrap_or_default();
        for pending_choice in self.pending_choice_frames() {
            if choice_frame_id(&pending_choice).is_some_and(|id| replayed.contains(id)) {
                continue;
            }
            if let Ok(json) = serde_json::to_string(&pending_choice) {
                frames.push(json);
            }
        }
        if let Some((_, events)) = active_window {
            frames.extend(events);
        }
        frames
    }

    /// 完成 cursor 重订阅：先在同一状态锁内确认 attachment 在直播表中，再冻结
    /// journal 窗口；锁外投递使 broadcaster 可以继续推进。重叠帧由客户端按
    /// `event_seq` 去重，因此该顺序没有遗漏窗口。
    pub(crate) async fn resubscribe(
        &self,
        outbound_tx: &mpsc::Sender<OutboundControl>,
        connection_id: &str,
        after_event_seq: u64,
    ) {
        enum Resubscription {
            Replay(Vec<String>),
            ActiveRunWindow { baseline: u64, events: Vec<String> },
            Snapshot { baseline: u64 },
        }

        let subscription = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let connection_id =
                state
                    .pending_attachments
                    .iter()
                    .find_map(|(connection_id, attachment)| {
                        attachment
                            .outbound_tx
                            .same_channel(outbound_tx)
                            .then(|| connection_id.clone())
                    });
            if let Some(connection_id) = connection_id
                && let Some(attachment) = state.pending_attachments.remove(&connection_id)
            {
                state.attachments.insert(connection_id, attachment);
            }
            match state.journal.replay_after(after_event_seq) {
                Some(events) => Resubscription::Replay(events),
                None => match state.journal.active_run_window() {
                    Some((first_seq, events)) => Resubscription::ActiveRunWindow {
                        baseline: first_seq.saturating_sub(1),
                        events,
                    },
                    None => Resubscription::Snapshot {
                        baseline: self
                            .next_event_seq
                            .load(Ordering::Relaxed)
                            .saturating_sub(1),
                    },
                },
            }
        };

        match subscription {
            Resubscription::Replay(events) => {
                for event in &events {
                    if outbound_tx
                        .send(OutboundControl::Text(event.clone()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                let (_, choice) = self.attached_session_state().await;
                if !send_optional_message(outbound_tx, choice).await {
                    return;
                }
                self.send_pending_choices_for_resubscription(outbound_tx, &events)
                    .await;
            }
            Resubscription::ActiveRunWindow { baseline, events } => {
                let (snapshot, choice) = self.attached_session_state().await;
                if !self
                    .send_snapshot_with_baseline(
                        outbound_tx,
                        snapshot.with_connection_id(connection_id),
                        baseline,
                    )
                    .await
                {
                    return;
                }
                if !send_optional_message(outbound_tx, choice).await {
                    return;
                }
                for event in &events {
                    if outbound_tx
                        .send(OutboundControl::Text(event.clone()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                self.send_pending_choices_for_resubscription(outbound_tx, &events)
                    .await;
            }
            Resubscription::Snapshot { baseline } => {
                let (snapshot, choice) = self.attached_session_state().await;
                if !self
                    .send_snapshot_with_baseline(
                        outbound_tx,
                        snapshot.with_connection_id(connection_id),
                        baseline,
                    )
                    .await
                {
                    return;
                }
                let _ = send_optional_message(outbound_tx, choice).await;
                self.send_pending_choices_for_resubscription(outbound_tx, &[])
                    .await;
            }
        }
    }

    /// F-24：重订阅收尾补发活跃 run 的挂起 provider choice。`replayed` 是本次
    /// 已经回放/补发过的帧集合——已含同 id 的挂起卡不重复投递。
    async fn send_pending_choices_for_resubscription(
        &self,
        outbound_tx: &mpsc::Sender<OutboundControl>,
        replayed: &[String],
    ) {
        let replayed_ids = choice_ids_in_frames(replayed);
        for pending_choice in self.pending_choice_frames() {
            if choice_frame_id(&pending_choice).is_some_and(|id| replayed_ids.contains(id)) {
                continue;
            }
            let Ok(json) = serde_json::to_string(&pending_choice) else {
                continue;
            };
            if outbound_tx.send(OutboundControl::Text(json)).await.is_err() {
                return;
            }
        }
    }

    async fn send_snapshot_with_baseline(
        &self,
        outbound_tx: &mpsc::Sender<OutboundControl>,
        snapshot: WsOutMessage,
        baseline: u64,
    ) -> bool {
        let Some(json) = inject_event_seq(
            match serde_json::to_string(&snapshot) {
                Ok(json) => json,
                Err(_) => return false,
            },
            baseline,
        ) else {
            return false;
        };
        outbound_tx.send(OutboundControl::Text(json)).await.is_ok()
    }

    pub async fn detach(self: &Arc<Self>, connection_id: &str) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.attachments.remove(connection_id);
            state.pending_attachments.remove(connection_id);
        }
        self.maybe_recycle().await;
    }

    /// C3/REQ-HTR-03：关键帧有界等待投递任务复查 degraded 态（已恢复直播的
    /// 连接不再投递旧基线，防 event_seq 回退覆盖新状态）。
    pub(crate) fn attachment_is_degraded(&self, connection_id: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .attachments
            .get(connection_id)
            .is_some_and(|attachment| attachment.degraded)
    }

    #[cfg(test)]
    pub(crate) async fn attach(
        &self,
        connection_id: &str,
        outbound_tx: mpsc::Sender<OutboundControl>,
    ) {
        self.register_attachment(connection_id, outbound_tx);
        self.activate_attachment(connection_id);
    }
}

async fn send_optional_message(
    outbound_tx: &mpsc::Sender<OutboundControl>,
    message: Option<WsOutMessage>,
) -> bool {
    let Some(message) = message else {
        return true;
    };
    let Ok(json) = serde_json::to_string(&message) else {
        return false;
    };
    outbound_tx.send(OutboundControl::Text(json)).await.is_ok()
}
