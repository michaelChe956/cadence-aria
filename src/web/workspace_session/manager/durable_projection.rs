//! durable 投影与创建期恢复（human gate / outline resume / 僵尸 run 终态落盘）。

use std::sync::Arc;

use tokio::sync::mpsc;

use super::WorkspaceSessionManager;
use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::WorkspaceSessionRecord;
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use crate::product::workspace_repository::workspace_repository_for_session;
use crate::web::workspace_ws_handler::{
    ProviderRunContext, ProviderRunKind, planning_resume_decision_with_fresh_index,
    planning_resume_run_kind, spawn_provider_run_from_handler,
};
use crate::web::workspace_ws_types::WsOutMessage;

impl WorkspaceSessionManager {
    /// F-06/F-09：durable 投影的同步读盘挪入阻塞线程池——观察 attach 频发且 run 任务
    /// 长期持有 engine 锁时，fallback 不得阻塞 async worker。JoinHandle 异常（panic/
    /// 取消）时退回原同步路径：投影只读，最坏也只是回到旧行为。
    pub(super) async fn durable_projection_offload(&self) -> (WsOutMessage, Option<WsOutMessage>) {
        let app_paths = self.app_paths.clone();
        let session_record = self.session_record.clone();
        match tokio::task::spawn_blocking(move || {
            Self::durable_projection_with(app_paths, session_record)
        })
        .await
        {
            Ok(projection) => projection,
            Err(join_error) => {
                eprintln!(
                    "[aria-durable-projection] blocking offload join failed session={}: {join_error}",
                    self.session_id
                );
                self.durable_projection()
            }
        }
    }

    /// 一次性只读 durable 投影器。禁止将它扩展为第二 engine 状态源：没有 spawn、没有
    /// provider/run 注册、没有 event receiver，并且使用有接收端但永不消费的 channel。
    /// 读盘重（Lifecycle/Checkpoint 仓库 + JSON 反序列化），async 上下文必须经
    /// `durable_projection_offload` 调用；`current_session_state` 的同步降级路径是
    /// 唯一的直接调用方。
    pub(crate) fn durable_projection(&self) -> (WsOutMessage, Option<WsOutMessage>) {
        Self::durable_projection_with(self.app_paths.clone(), self.session_record.clone())
    }

    fn durable_projection_with(
        app_paths: ProductAppPaths,
        session_record: WorkspaceSessionRecord,
    ) -> (WsOutMessage, Option<WsOutMessage>) {
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let checkpoint_store = Arc::new(CheckpointStore::new(
            app_paths.issue_lifecycle_root(&session_record.project_id, &session_record.issue_id),
        ));
        let (projection_tx, _projection_rx) = mpsc::channel::<EngineEvent>(1);
        let mut session = WorkspaceSession::from_record(session_record.clone());
        if let Ok(repository) =
            workspace_repository_for_session(&app_paths, &lifecycle, &session_record)
        {
            session.repository_path = Some(repository.path);
        }
        if let Ok(checkpoints) = checkpoint_store.list_checkpoints(&session.session_id) {
            session.restore_checkpoint_ids(&checkpoints);
        }
        let engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle, projection_tx, session);
        (
            engine.build_session_state(),
            engine.pending_author_choice_request_message(),
        )
    }

    pub(super) async fn recover_on_creation(self: &Arc<Self>) {
        let run_context = self.provider_run_context(self.workspace_runs.clone());
        let recovery_error = match self.recover_human_gate_turns(&run_context).await {
            Ok(()) => self.recover_outline_run(&run_context).await,
            Err(error) => Err(error),
        };
        if let Err(message) = recovery_error {
            eprintln!(
                "[aria-recovery] workspace session recovery skipped session={}: {message}",
                self.session_id
            );
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recovery_error = Some(message);
        }
        // F-23（0482 跨重启僵尸）：恢复臂（human gate resume / outline resume）之后
        // 仍停留在 Running/CrossReview/Revision 且无活跃 run 的 durable 会话，只可能是
        // 进程重启孤儿——registry 单例契约保证 manager 创建时不存在幸存 run。落
        // AbortedByDisconnect 终态并整流回 prepare_context，否则会话永久楔死在
        // running、abort 无处落地（旧架构连接关闭曾走的正是同两条引擎 API）。
        self.recover_stale_run_if_zombie().await;
    }

    async fn recover_stale_run_if_zombie(self: &Arc<Self>) {
        if self.active_run().await.is_some() {
            return;
        }
        let recovered = {
            let mut engine = self.engine.lock().await;
            if !matches!(
                engine.current_stage(),
                crate::product::workspace_engine::WorkspaceStage::Running
                    | crate::product::workspace_engine::WorkspaceStage::CrossReview
                    | crate::product::workspace_engine::WorkspaceStage::Revision
            ) {
                return;
            }
            engine.recover_stale_active_run_after_disconnect().await;
            true
        };
        if recovered {
            eprintln!(
                "[aria-recovery] stale zombie run recovered to prepare_context session={}",
                self.session_id
            );
            self.broadcast_current_session_state();
        }
    }

    async fn recover_human_gate_turns(
        self: &Arc<Self>,
        run_context: &ProviderRunContext,
    ) -> Result<(), String> {
        let human_gate_recovery = {
            let mut engine = self.engine.lock().await;
            engine.recover_human_gate_turns(false)
        }
        .map_err(|error| format!("human gate recovery failed: {error}"))?;
        for (turn_id, action) in human_gate_recovery {
            if !matches!(
                action,
                crate::product::workspace_engine::HumanGateRecoveryAction::ResumeSameTurn { .. }
            ) {
                continue;
            }
            let run_kind = crate::product::workspace_engine::provider_run_kind_for_human_gate(
                self.session_record.flow_kind,
                &turn_id,
            )?;
            let (outbound_tx, _outbound_rx) = mpsc::channel(1);
            spawn_provider_run_from_handler(run_context.clone(), run_kind, outbound_tx).await?;
        }
        Ok(())
    }

    async fn recover_outline_run(
        self: &Arc<Self>,
        run_context: &ProviderRunContext,
    ) -> Result<(), String> {
        let outline_resume_kind = {
            let engine = self.engine.lock().await;
            let durable_flow_kind = self.session_record.flow_kind;
            if let Some(error) = engine.outline_revision_recovery_error() {
                Err(format!("outline revision recovery failed: {error}"))
            } else {
                let should_resume = engine.session().workspace_type
                    == crate::product::models::WorkspaceType::WorkItemPlan
                    && engine.session().stage == crate::product::workspace_engine::WorkspaceStage::Running
                    && engine.active_node_type()
                        == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemPlanOutlineRun)
                    && engine.active_run_id().is_none();
                if !should_resume {
                    Ok(None)
                } else if let Some(node_id) = engine.active_timeline_node_id() {
                    match LifecycleStore::new(self.app_paths.clone())
                        .load_node_detail(&self.session_id, &node_id)
                    {
                        Ok(detail) => Ok(Some(if durable_flow_kind
                            == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
                        {
                            ProviderRunKind::work_item_plan_author_for_durable_flow(durable_flow_kind)
                        } else if detail.is_revision {
                            ProviderRunKind::WorkItemPlanOutlineRevision {
                                feedback: detail.revision_feedback,
                            }
                        } else {
                            ProviderRunKind::work_item_plan_author_for_durable_flow(durable_flow_kind)
                        })),
                        Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => {
                            Ok(Some(ProviderRunKind::work_item_plan_author_for_durable_flow(
                                durable_flow_kind,
                            )))
                        }
                        Err(error) => Err(format!(
                            "resume outline run detail failed for {node_id}: {error}"
                        )),
                    }
                } else {
                    Err("resume outline run detail failed: active node id unavailable".to_string())
                }
            }
        };
        if let Some(run_kind) = outline_resume_kind?
            && self.active_run().await.is_none()
        {
            let decision = planning_resume_decision_with_fresh_index(
                &self.app_paths,
                &self.session_record.project_id,
                &self.session_record.issue_id,
            )
            .await?;
            let (outbound_tx, _outbound_rx) = mpsc::channel(1);
            spawn_provider_run_from_handler(
                run_context.clone(),
                planning_resume_run_kind(&decision, run_kind),
                outbound_tx,
            )
            .await?;
        }
        Ok(())
    }
}
