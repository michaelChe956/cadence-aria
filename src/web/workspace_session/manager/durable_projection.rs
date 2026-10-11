//! durable 投影与创建期恢复（human gate / outline resume / 僵尸 run 终态落盘）。

use std::sync::Arc;

use tokio::sync::mpsc;

use super::WorkspaceSessionManager;
use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{WorkspaceSessionRecord, WorkspaceType};
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use crate::product::workspace_repository::workspace_repository_for_session;
use crate::web::workspace_ws_handler::{
    ProviderRunContext, ProviderRunKind, planning_resume_decision_with_fresh_index,
    planning_resume_run_kind, spawn_provider_run_from_handler,
};
use crate::web::workspace_ws_types::WsOutMessage;

/// A1b1（设计 §4.4）：durable 投影读取失败的稳定 ProtocolError code——
/// 读取失败不得发布可启动假态。
pub(crate) const DURABLE_PROJECTION_UNAVAILABLE_CODE: &str =
    "workspace_durable_projection_unavailable";

/// A1b1（设计 §4.4 / Plan 接口字典）：manager 绑定身份——出生 record 冻结的
/// session/project/issue/entity/type 五元组。纯投影每次以 session_id 定位
/// 最新 durable record 并以此核对身份；出生 record 仅作 identity 对照，
/// 不再是投影正文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceSessionIdentity {
    pub(crate) session_id: String,
    pub(crate) project_id: String,
    pub(crate) issue_id: String,
    pub(crate) entity_id: String,
    pub(crate) workspace_type: WorkspaceType,
}

impl WorkspaceSessionIdentity {
    /// 从 manager 绑定（出生）record 冻结身份。
    pub(crate) fn from_bound_record(record: &WorkspaceSessionRecord) -> Self {
        Self {
            session_id: record.id.clone(),
            project_id: record.project_id.clone(),
            issue_id: record.issue_id.clone(),
            entity_id: record.entity_id.clone(),
            workspace_type: record.workspace_type.clone(),
        }
    }
}

/// 纯投影产物：durable session_state 帧＋挂起 author choice 帧。
#[derive(Debug, Clone)]
pub(crate) struct DurableWorkspaceProjection {
    pub(crate) session_state: WsOutMessage,
    pub(crate) pending_choice: Option<WsOutMessage>,
}

impl DurableWorkspaceProjection {
    /// 最新 durable record 的当前 LC 恢复 ID（无恢复为 None）——核对
    /// 「最新 record 而非出生 record」是投影正文。
    pub(crate) fn recovery_id(&self) -> Option<&str> {
        match &self.session_state {
            WsOutMessage::SessionState {
                lc_resume_recovery, ..
            } => lc_resume_recovery
                .as_ref()
                .map(|recovery| recovery.recovery_id.as_str()),
            _ => None,
        }
    }
}

/// 纯投影读取的 typed 错误：store 读失败（NotFound/IO/JSON）或最新 record
/// 与 manager 绑定身份不一致。两者都不得把出生 record、空 recovery 或
/// Open 当当前事实（设计 §4.4）。
#[derive(Debug)]
pub(crate) enum DurableProjectionError {
    Store(ProductStoreError),
    IdentityMismatch { session_id: String },
}

impl std::fmt::Display for DurableProjectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "durable store read failed: {error}"),
            Self::IdentityMismatch { session_id } => write!(
                formatter,
                "latest workspace session record identity mismatch for {session_id}"
            ),
        }
    }
}
impl DurableProjectionError {
    /// A1b2（设计 §4.4）：读失败的稳定 ProtocolError 帧（code 单点常量
    /// `workspace_durable_projection_unavailable`）——router/attachment/
    /// socket 的 caller 消费面统一经此转帧，不各自拼字面量、不发伪
    /// SessionState。
    pub(crate) fn unavailable_frame(&self, session_id: &str) -> WsOutMessage {
        eprintln!(
            "[aria-durable-projection] latest record load failed session={session_id}: {self}"
        );
        WsOutMessage::ProtocolError {
            code: DURABLE_PROJECTION_UNAVAILABLE_CODE.to_string(),
            message: format!("durable projection unavailable for session {session_id}: {self}"),
            context: None,
        }
    }
}

/// A1b1 private 纯加载核心（设计 §4.4）：每次以 session_id 从 LifecycleStore
/// 读取**最新** WorkspaceSessionRecord，核对 manager 绑定身份后经
/// `WorkspaceEngine::new_persistent_for_projection`（ProjectionOnly，零恢复
/// 写）构造纯投影。NotFound/IO/JSON/IdentityMismatch 一律 typed 错误上抛，
/// 不发伪 SessionState、不回出生 record、不 default 假 state。LC 恢复等待
/// 的 reconcile 只属于真实恢复入口（A2b），本核心零写。
pub(crate) fn load_durable_workspace_projection(
    app_paths: &ProductAppPaths,
    expected_identity: &WorkspaceSessionIdentity,
) -> Result<DurableWorkspaceProjection, DurableProjectionError> {
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let latest_record = lifecycle
        .get_workspace_session(&expected_identity.session_id)
        .map_err(DurableProjectionError::Store)?;
    if latest_record.project_id != expected_identity.project_id
        || latest_record.issue_id != expected_identity.issue_id
        || latest_record.entity_id != expected_identity.entity_id
        || latest_record.workspace_type != expected_identity.workspace_type
    {
        return Err(DurableProjectionError::IdentityMismatch {
            session_id: expected_identity.session_id.clone(),
        });
    }
    let checkpoint_store = Arc::new(CheckpointStore::new(
        app_paths.issue_lifecycle_root(&latest_record.project_id, &latest_record.issue_id),
    ));
    let (projection_tx, _projection_rx) = mpsc::channel::<EngineEvent>(1);
    let mut session = WorkspaceSession::from_record(latest_record.clone());
    if let Ok(repository) = workspace_repository_for_session(&app_paths, &lifecycle, &latest_record)
    {
        session.repository_path = Some(repository.path);
    }
    if let Ok(checkpoints) = checkpoint_store.list_checkpoints(&session.session_id) {
        session.restore_checkpoint_ids(&checkpoints);
    }
    let engine = WorkspaceEngine::new_persistent_for_projection(
        checkpoint_store,
        lifecycle,
        projection_tx,
        session,
    )
    .map_err(DurableProjectionError::Store)?;
    let mut session_state = engine.build_session_state();
    // P0 1.2：durable 投影同样以 durable enrollment 覆盖归属；读取失败置
    // None（未知），绝不伪 client。
    let store =
        crate::product::issue_automation_store::IssueAutomationStore::new(app_paths.clone());
    if let Err(error) = crate::product::issue_automation_store::project_session_automation(
        &mut session_state,
        &latest_record,
        &store,
    ) {
        eprintln!(
            "[aria-automation-ownership] durable projection failed session={}: {error}",
            latest_record.id
        );
        if let WsOutMessage::SessionState { automation, .. } = &mut session_state {
            *automation = None;
        }
    }
    Ok(DurableWorkspaceProjection {
        session_state,
        pending_choice: engine.pending_author_choice_request_message(),
    })
}

impl WorkspaceSessionManager {
    /// F-06/F-09：durable 投影的同步读盘挪入阻塞线程池——观察 attach 频发且 run 任务
    /// 长期持有 engine 锁时，fallback 不得阻塞 async worker。JoinHandle 异常（panic/
    /// 取消）时退回原同步路径：投影只读，最坏也只是回到旧行为。
    pub(super) async fn durable_projection_offload(
        &self,
    ) -> Result<DurableWorkspaceProjection, DurableProjectionError> {
        let app_paths = self.app_paths.clone();
        let session_record = self.session_record.clone();
        let loaded = match tokio::task::spawn_blocking(move || {
            Self::durable_projection_with(app_paths, session_record)
        })
        .await
        {
            Ok(result) => result,
            Err(join_error) => {
                eprintln!(
                    "[aria-durable-projection] blocking offload join failed session={}: {join_error}",
                    self.session_id
                );
                Self::durable_projection_with(self.app_paths.clone(), self.session_record.clone())
            }
        };
        loaded.map(|mut projection| {
            self.stamp_pending_choice_run_ids(&mut projection.session_state);
            projection
        })
    }

    /// 一次性只读 durable 投影器（A1b2 Result cutover，设计 §4.4）：正文每次
    /// 经 `load_durable_workspace_projection` 取**最新** durable record（出生
    /// record 仅身份对照）；读取失败 typed `DurableProjectionError` 上抛，由
    /// router/attachment/socket caller 转 ProtocolError（不回出生 record、不
    /// default 假 state、不发伪 SessionState）。禁止将它扩展为第二 engine
    /// 状态源：没有 spawn、没有 provider/run 注册、没有 event receiver。
    /// 读盘重（Lifecycle/Checkpoint 仓库 + JSON 反序列化），async 上下文必须
    /// 经 `durable_projection_offload` 调用；`current_session_state` 的同步
    /// 降级路径是唯一直接调用方。
    pub(crate) fn durable_projection(
        &self,
    ) -> Result<DurableWorkspaceProjection, DurableProjectionError> {
        Self::durable_projection_with(self.app_paths.clone(), self.session_record.clone()).map(
            |mut projection| {
                self.stamp_pending_choice_run_ids(&mut projection.session_state);
                projection
            },
        )
    }

    /// A1b2（设计 §4.4）：出生 record 只构造绑定身份；投影正文每次经
    /// `load_durable_workspace_projection` 取最新 durable record，失败原样
    /// typed 上抛（打点在 caller 转帧面 `unavailable_frame`）。
    fn durable_projection_with(
        app_paths: ProductAppPaths,
        session_record: WorkspaceSessionRecord,
    ) -> Result<DurableWorkspaceProjection, DurableProjectionError> {
        let identity = WorkspaceSessionIdentity::from_bound_record(&session_record);
        load_durable_workspace_projection(&app_paths, &identity)
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
        // F2（0018 现场）：F-23 僵尸恢复的 stage 集合（Running/CrossReview/Revision）
        // 漏掉「stage=HumanConfirm ∧ durable status=running」的不自洽卡死形态——
        // 重建时引擎 stage 取自最后 Active 门节点（human_confirm），既不在集合内
        // 也被误当正常门态。该形态只可能是 SC 委托返修接力孤儿（phase=Generate、
        // 无在途 run），走同源引擎守卫回落人工门（durable WaitingForHuman + 门节点
        // 摘要携带原因），令会话恢复可操作（人工可再反馈或放弃）——优先于
        // AbortedByDisconnect 终态化。
        self.recover_delegated_rerun_orphan().await;
        // F-23（0482 跨重启僵尸）：恢复臂（human gate resume / outline resume）之后
        // 仍停留在 Running/CrossReview/Revision 且无活跃 run 的 durable 会话，只可能是
        // 进程重启孤儿——registry 单例契约保证 manager 创建时不存在幸存 run。落
        // AbortedByDisconnect 终态并整流回 prepare_context，否则会话永久楔死在
        // running、abort 无处落地（旧架构连接关闭曾走的正是同两条引擎 API）。
        self.recover_stale_run_if_zombie().await;
    }

    async fn recover_delegated_rerun_orphan(self: &Arc<Self>) {
        let mutated = {
            let mut engine = self.engine.lock().await;
            let before = engine.session().session_status.clone();
            engine
                .recover_delegated_author_rerun_failure("进程重启：委托返修接力未启动")
                .await;
            engine.session().session_status != before
        };
        if mutated {
            eprintln!(
                "[aria-recovery] delegated rerun orphan recovered to human gate session={}",
                self.session_id
            );
            self.broadcast_current_session_state();
        }
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
