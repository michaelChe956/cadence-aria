//! F-24 挂起 provider choice 帧的登记、移除与补发辅助。

use std::collections::HashSet;

use super::{ActiveRun, WorkspaceSessionManager};
use crate::web::workspace_ws_types::WsOutMessage;

impl WorkspaceSessionManager {
    /// F-24：注册挂起 provider choice 帧（按 id 幂等）。无活跃 run 时丢弃——
    /// 该 choice 无应答通道，重投只会制造 stale 卡。
    pub fn register_pending_choice_frame(&self, frame: WsOutMessage) {
        let Some(run) = self.active_run_ref() else {
            return;
        };
        let mut pending = run
            .pending_choices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !pending
            .iter()
            .any(|existing| choice_frame_id(existing) == choice_frame_id(&frame))
        {
            pending.push(frame);
        }
    }

    /// F-24：应答到达时移除挂起帧。返回 false 表示该 id 不在挂起集（stale 应答）。
    pub fn remove_pending_choice_frame(&self, id: &str) -> bool {
        let Some(run) = self.active_run_ref() else {
            return false;
        };
        let mut pending = run
            .pending_choices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = pending.len();
        pending.retain(|frame| choice_frame_id(frame) != Some(id));
        before != pending.len()
    }

    /// F-24：当前活跃 run 的挂起 choice 帧（供 attach/重订阅/degraded 恢复补发）。
    pub(crate) fn pending_choice_frames(&self) -> Vec<WsOutMessage> {
        self.active_run_ref()
            .map(|run| {
                run.pending_choices
                    .lock()
                    .map(|pending| pending.clone())
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    fn active_run_ref(&self) -> Option<ActiveRun> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_run
            .clone()
    }
}

/// F-24：从 wire 帧提取 choice id（仅 choice_request 帧有）。
pub(super) fn choice_frame_id(frame: &WsOutMessage) -> Option<&str> {
    match frame {
        WsOutMessage::ChoiceRequest { id, .. } => Some(id),
        _ => None,
    }
}

pub(super) fn choice_ids_in_frames(frames: &[String]) -> HashSet<String> {
    frames
        .iter()
        .filter_map(|json| {
            let value: serde_json::Value = serde_json::from_str(json).ok()?;
            let id = value.get("id")?.as_str()?;
            (value.get("type")?.as_str()? == "choice_request").then(|| id.to_string())
        })
        .collect()
}
