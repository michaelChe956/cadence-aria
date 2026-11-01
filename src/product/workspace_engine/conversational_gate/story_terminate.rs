//! story/design 会话 author_confirm 门的 typed terminate 关门（F-18）：
//! `terminate_story_author_gate` durable CAS 关门与失配重读翻译
//! `translate_lost_story_terminate_race`（k3 审 P2）。
//! 自 `conversational_gate.rs` 拆出（1200 行守护，Wave 3.1）：纯移动零语义变化。

use super::super::{
    EngineEvent, TimelineNodeDraft, TimelineNodeStatus, TimelineNodeType, WorkspaceEngine,
    WorkspaceStage,
};
use super::*;
/// k3 审 P2 测试注入口：story 门 terminate 的「durable 读 → CAS 写」窗口内
/// 注入一次外部写入（模拟 HTTP confirm/另一 terminate 先行落盘），仅测试构建存在。
#[cfg(test)]
type StoryTerminateDriftHook = Box<dyn Fn() + Send>;

#[cfg(test)]
fn story_terminate_drift_hooks() -> &'static std::sync::Mutex<Vec<(String, StoryTerminateDriftHook)>>
{
    static HOOKS: std::sync::OnceLock<std::sync::Mutex<Vec<(String, StoryTerminateDriftHook)>>> =
        std::sync::OnceLock::new();
    HOOKS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

#[cfg(test)]
pub(crate) fn register_story_terminate_drift_hook(session_id: &str, hook: StoryTerminateDriftHook) {
    story_terminate_drift_hooks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push((session_id.to_string(), hook));
}

#[cfg(test)]
fn run_story_terminate_drift_hooks(session_id: &str) {
    let mut hooks = story_terminate_drift_hooks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    hooks.retain(|(id, hook)| {
        if id == session_id {
            hook();
            false
        } else {
            true
        }
    });
}

#[cfg(not(test))]
#[inline]
fn run_story_terminate_drift_hooks(_session_id: &str) {}

impl WorkspaceEngine {
    /// story/design 会话 author_confirm 门的 typed terminate 关门（F-18）：
    /// durable WaitingForHuman（author_confirm 的 status 映射）+ 内存
    /// author_confirm 门开 → Terminated 终态 + Completed 阶段 + 「流程终止」
    /// 节点 + 恰一条 terminate close 事件（终态形状承接 C3 删除的 legacy
    /// Terminate 分支）。幂等/开态判据读 durable：重复 terminate 幂等 no-op
    ///（AlreadyClosed，不依赖内存 stage——迟到 worker 内存可能已 Completed）；
    /// 已 Confirmed（HTTP approve 先行）或非门开态 fail-closed 不改写。终态写入走
    /// `compare_and_update_workspace_session_status` CAS（amendment.rs 同款先例：
    /// 独占锁内比对 expected 快照后置终态——k3 审 P2：HTTP confirm 不持锁无条件写
    /// Confirmed，落在读-写窗口时非原子写会把 Confirmed 覆盖成 Terminated；CAS
    /// 失配经 `translate_lost_story_terminate_race` 重读翻译，绝不覆盖先到者），
    /// 不占用 SC 门 close CAS。
    pub(super) async fn terminate_story_author_gate(
        &mut self,
    ) -> Result<HumanGateCloseOutcome, String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let durable = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        match durable.status {
            WorkspaceSessionStatus::Terminated => {
                // 先到者已关门：迟到 terminate 幂等 no-op（与 SC 迟到翻译同族）。
                self.session.session_status = durable.status.clone();
                return Ok(HumanGateCloseOutcome::AlreadyClosed {
                    status: durable.status,
                });
            }
            WorkspaceSessionStatus::WaitingForHuman => {
                if self.session.stage != WorkspaceStage::AuthorConfirm {
                    return Err(format!(
                        "story author gate terminate requires the author_confirm stage (current stage {}, session {})",
                        self.session.stage.as_str(),
                        self.session.session_id
                    ));
                }
            }
            status => {
                return Err(format!(
                    "story author gate terminate requires a waiting_for_human session (durable status {status:?}, session {})",
                    self.session.session_id
                ));
            }
        }
        run_story_terminate_drift_hooks(&self.session.session_id);
        let saved = match lifecycle.compare_and_update_workspace_session_status(
            &durable,
            WorkspaceSessionStatus::Terminated,
        ) {
            Ok(saved) => saved,
            Err(ProductStoreError::IdentityMismatch { .. }) => {
                // 读-CAS 窗口内 durable 已漂移（HTTP confirm 先行等）：重读翻译
                //（幂等/明确错误），绝不以非原子写覆盖先到者（k3 审 P2）。
                return self
                    .translate_lost_story_terminate_race(&lifecycle, "terminate".to_string())
                    .await;
            }
            Err(error) => return Err(error.to_string()),
        };
        self.session.session_status = saved.status;
        let terminal_stage = WorkspaceStage::Completed;
        self.session.stage = terminal_stage.clone();
        let _ = self
            .event_tx
            .send(EngineEvent::HumanGateClosed {
                decision: "terminate".to_string(),
                stage: terminal_stage.as_str().to_string(),
            })
            .await;
        self.complete_active_node(Some("已终止".to_string())).await;
        self.transition_stage(terminal_stage).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::Completed,
                agent: None,
                stage: WorkspaceStage::Completed,
                round: None,
                title: "流程终止".to_string(),
                summary: Some("已终止".to_string()),
                status: TimelineNodeStatus::Completed,
            })
            .await;
        Ok(HumanGateCloseOutcome::Abandoned)
    }

    /// story 门 terminate CAS 失配后的 durable 重读翻译（k3 审 P2，与 SC
    /// `translate_lost_human_gate_close_race` 同族）：先到 terminate 已关 → 幂等
    /// AlreadyClosed（同步 in-memory 状态、不重复 close 语义）；其余漂移（含
    /// HTTP confirm 先行的 Confirmed）→ 明确错误上抛，绝不覆盖先到者。
    async fn translate_lost_story_terminate_race(
        &mut self,
        lifecycle: &LifecycleStore,
        direction: String,
    ) -> Result<HumanGateCloseOutcome, String> {
        let durable = match lifecycle.get_workspace_session(&self.session.session_id) {
            Ok(durable) => durable,
            Err(_) => {
                return Err(format!(
                    "story author gate {direction} lost the single-flight race; durable record drifted (session {})",
                    self.session.session_id
                ));
            }
        };
        match durable.status {
            WorkspaceSessionStatus::Terminated => {
                self.session.session_status = durable.status.clone();
                Ok(HumanGateCloseOutcome::AlreadyClosed {
                    status: durable.status,
                })
            }
            status => Err(format!(
                "story author gate {direction} lost the single-flight race (durable status now {status:?}, session {}); late {direction} is rejected without overwriting",
                self.session.session_id
            )),
        }
    }
}
