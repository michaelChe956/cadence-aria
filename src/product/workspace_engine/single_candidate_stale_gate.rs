//! C（陈旧人工门）：SC 委托返修接力「瞬时启动失败 ⇒ 回落人工门」与「接力随后
//! 真的接管接续路径」的判定与收口。
//!
//! 现场（陈旧人工门）：委托返修接力一次瞬时启动失败后**真的跑成功了**（author
//! 节点 completed、评审继续），但人工门已经开着且没人关——臂终态
//! `stopped_needs_human`，门节点仍 active，会话永远停在人工门上。
//!
//! 本模块只做一件事：**接力接管时关闭由恢复路径开出的那一扇门**。复用既有门
//! 机制（门节点时间线 + 会话 `WaitingForHuman` 门开态证据），不新造第二套门状态
//! 机，也不关闭**用户**开出的门。

use super::*;
use crate::product::work_item_plan_policy::RepairReservationState;

impl WorkspaceEngine {
    /// C（陈旧人工门）：SC 委托返修接力启动失败回落人工门的**门来源标记**。
    ///
    /// 该前缀既是既有对外文案契约（逐字保留，见
    /// `single_candidate.rs::recover_delegated_author_rerun_failure`），也是
    /// 「本恢复路径开出的门」在 durable 时间线上的唯一可判定标记——接力随后
    /// 真正接管接续路径时据此关闭该门，绝不误关**用户**开出的门（用户门的
    /// 摘要无此前缀）。
    pub(crate) const DELEGATED_RERUN_GATE_SUMMARY: &str = "SC 返修接力启动失败，已回落人工门";

    /// C（陈旧人工门）：SC 委托返修接力**真正接管接续路径**后，关闭由
    /// `recover_delegated_author_rerun_failure` 回落开出的陈旧人工门。
    ///
    /// 判据（全部读 durable 事实，不依赖内存副本）：
    /// - durable 会话 `status == WaitingForHuman`——该门不写门快照
    ///   （`candidate_recovery_gate_applies` 只对 Approval 相位落快照），故
    ///   `WaitingForHuman` 本身即门开态证据（lifecycle_store F-21 裁定同口径）；
    /// - durable `repair_reservation` 已被接力消费（`state != Reserved`）——接力
    ///   真正领走了 provider start，接续路径已回到接力手上；预领仍 `Reserved`
    ///   ⇒ 接力尚未接管（人工出口必须保留），本方法零副作用；
    /// - durable 时间线存在 **Active 的 HumanConfirm 门节点**，且其摘要带
    ///   `DELEGATED_RERUN_GATE_SUMMARY` 前缀——该门由本恢复路径开出；**用户**
    ///   开出的门摘要无此前缀，绝不被本方法关闭。
    ///
    /// 关闭动作（复用既有门机制）：门节点经既有 `update_timeline_node` 链路收口
    /// 为 Completed ＋ 会话门开态证据经 **CAS** 置回 `Running`（并发写漂移即
    /// Conflict 上抛——不覆盖并发写、不吞错；冲突只发可见错误事件，不阻断已领到
    /// provider start 的接力）。
    ///
    /// 返回 `true` 表示本次确实收口了一扇陈旧门。
    pub(crate) async fn close_stale_delegated_rerun_gate(&mut self) -> Result<bool, String> {
        let Some(store) = self.lifecycle_store.clone() else {
            return Ok(false);
        };
        let Ok(expected) = store.get_workspace_session(&self.session.session_id) else {
            return Ok(false);
        };
        if expected.status != crate::product::models::WorkspaceSessionStatus::WaitingForHuman {
            return Ok(false);
        }
        // 接力是否真的接管接续路径：durable repair 预领已被消费（不再是 Reserved）。
        // 预领仍 Reserved ⇒ 接力尚未接管，人工出口必须保留（本方法零副作用）。
        if !expected
            .repair_reservation
            .as_ref()
            .is_some_and(|reservation| reservation.state != RepairReservationState::Reserved)
        {
            return Ok(false);
        }
        let Ok(nodes) = store.load_timeline_nodes(&self.session.session_id) else {
            return Ok(false);
        };
        let Some(gate_node_id) = nodes
            .iter()
            .find(|node| {
                node.node_type == crate::web::workspace_ws_types::TimelineNodeType::HumanConfirm
                    && node.status == crate::web::workspace_ws_types::TimelineNodeStatus::Active
                    && node.summary.as_deref().is_some_and(|summary| {
                        summary.starts_with(Self::DELEGATED_RERUN_GATE_SUMMARY)
                    })
            })
            .map(|node| node.node_id.clone())
        else {
            return Ok(false);
        };
        // durable 门节点必须在内存时间线中可见——`update_timeline_node` 落盘的是
        // 整条内存时间线，内存缺该节点时落盘会丢掉 durable 上的门节点（宁可不动）。
        if !self
            .timeline_nodes
            .iter()
            .any(|node| node.node_id == gate_node_id)
        {
            return Ok(false);
        }
        // 门开态证据先收（CAS）：并发写漂移即 Conflict 上抛，绝不覆盖他人的写。
        let saved = match store.compare_and_save_delegated_rerun_gate_close(&expected) {
            Ok(saved) => saved,
            Err(error) => {
                let message = format!(
                    "close stale delegated rerun gate failed ({}): {error}",
                    self.session.session_id
                );
                tracing::error!(%message, "stale delegated rerun human gate was not closed");
                // 不吞错：可见错误事件上抛（不阻断已领到 provider start 的接力）。
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                return Err(format!(
                    "close stale delegated rerun gate failed ({}): {error}",
                    self.session.session_id
                ));
            }
        };
        self.reload_session_from_record(saved);
        // 门节点收口（既有时间线链路：内存 + durable 时间线 + 事件广播）。
        self.update_timeline_node(
            &gate_node_id,
            crate::web::workspace_ws_types::TimelineNodeStatus::Completed,
            Some("SC 返修接力已接管接续路径，人工门自动关闭".to_string()),
        )
        .await;
        self.transition_stage(crate::product::workspace_engine::WorkspaceStage::Running)
            .await;
        let _ = self
            .event_tx
            .send(EngineEvent::HumanGateClosed {
                decision: "delegated_rerun_relay".to_string(),
                stage: self.session.stage.as_str().to_string(),
            })
            .await;
        Ok(true)
    }
}
