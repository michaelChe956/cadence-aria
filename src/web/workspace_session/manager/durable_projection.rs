//! durable 投影与创建期恢复（human gate / outline resume / 僵尸 run 终态落盘）。

use std::sync::Arc;

use tokio::sync::mpsc;

use super::WorkspaceSessionManager;
use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    LcRecoveryArtifactRef, LcRecoveryBusinessContext, LcRecoveryRunContext, WorkspaceSessionRecord,
    WorkspaceType,
};
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use crate::product::workspace_repository::workspace_repository_for_session;
use crate::web::workspace_ws_handler::{
    PlanningResumeLaunchDecision, ProviderRunContext, ProviderRunKind,
    planning_resume_decision_with_fresh_index, planning_resume_launch_decision,
    spawn_provider_run_from_handler,
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
    #[cfg(test)] // part_07 typed 断言专用读取面。
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
    if let Ok(repository) = workspace_repository_for_session(app_paths, &lifecycle, &latest_record)
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

    pub(crate) async fn recover_on_creation(self: &Arc<Self>) {
        // A2b（Task 2.6，设计 §4.3/§4.4）：真实创建恢复入口**先**执行 LC
        // 等待收口（唯一写方：幂等终态化旧 provider 节点、清 active run、
        // run 阶段回 PrepareContext——在任何自动 recover_outline/human_turn
        // 之前）；等待 active 时阻断其后的一切自动接力：旧 outline resume、
        // human gate turn 重驱、委托返修孤儿回落与僵尸 run 终态化都不得在
        // 等待期间再动业务事实（SC gate/phase/turn/预算保留、旁路 spawn
        // 恒 0）。纯投影 fallback 从不写恢复事实。
        let lc_waiting_active = {
            let mut engine = self.engine.lock().await;
            if engine.lc_resume_recovery_active() {
                engine.reconcile_lc_resume_waiting().await;
                true
            } else {
                false
            }
        };
        if lc_waiting_active {
            eprintln!(
                "[aria-recovery] lc resume recovery waiting active; auto recovery chains blocked session={}",
                self.session_id
            );
            self.broadcast_current_session_state();
            return;
        }

        // B2a（Task 5.1，设计 §4.3/§5.3）：Plan stale 等待先于一切自动接力
        //（human gate turn resume / outline resume / 委托孤儿回落 / 僵尸终
        // 态化）。legacy 规划会话存在可恢复中断 outline run 时先做 planning
        // 指纹决策：StaleContext 不再自动 spawn OutlineRebuild（B3 旧行为
        // 删除）——捕获 run_context（revision feedback 在任何 take 前固化）
        // 落 `planning_context_stale` 等待并收口；此后由上方 lc_waiting_active
        // 同语义阻断全部自动链（manual turn 不自动 resume/increment、旁路
        // spawn 恒 0），重建仅显式 fresh 授权后（dispatch 面）。捕获写失败
        // fail-closed：阻断自动链但不 spawn。SameContext/None/SC 交原恢复
        // 链（SC stale 等待语义归 B2c/5.5）。
        if self.persist_planning_stale_wait_if_drifted().await {
            self.broadcast_current_session_state();
            return;
        }
        let run_context = self.provider_run_context(self.workspace_runs.clone());
        // 重启期在飞运行收口（B）：`Running` 的 turn 意味着上一进程的 provider
        // **已启动**且必已随进程消失——先按既有终态 `Failed(provider_err)` 收口，
        // 既有恢复链才不会为本轮安排 `ResumeSameTurn`（那会重复执行已开始过的
        // provider 工作）。`Reserved` turn（provider 可证明未启动）保持既有安全
        // 续跑语义，随后由 `recover_human_gate_turns` 正常重启 attempt 1。
        self.close_lost_provider_human_gate_turns().await;
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
        // 重启期在飞运行收口（B）：durable `status=running` ∧ stage `human_confirm`
        // ∧ 无活动 run 的会话，F-23 僵尸臂（stage 集合不含 HumanConfirm）与委托
        // 孤儿臂（只认 phase=Generate 委托接力）都不命中时会永久停在 running——
        // `confirm`/`human_gate_feedback` 都要求 waiting_for_human，人工无从处置。
        // 回落既有可恢复终态 waiting_for_human（门重开、不启动 provider、不双跑
        // 已开始过的 provider 工作）；优先于下方 AbortedByDisconnect 终态化。
        self.recover_human_confirm_lost_provider().await;
        // F-23（0482 跨重启僵尸）：恢复臂（human gate resume / outline resume）之后
        // 仍停留在 Running/CrossReview/Revision 且无活跃 run 的 durable 会话，只可能是
        // 进程重启孤儿——registry 单例契约保证 manager 创建时不存在幸存 run。落
        // AbortedByDisconnect 终态并整流回 prepare_context，否则会话永久楔死在
        // running、abort 无处落地（旧架构连接关闭曾走的正是同两条引擎 API）。
        self.recover_stale_run_if_zombie().await;
    }

    /// B2a（Task 5.1）：恢复入口的 planning 指纹预检（读侧判据与
    /// `recover_outline_run` 的 should_resume 同源：WorkItemPlan/Running/
    /// 活动 OutlineRun 节点/无活 run，且非 SC 流）。Stale 时捕获 run_context
    /// 落 `planning_context_stale` 等待（0 spawn）；决策面错误交
    /// `recover_outline_run` 原路径报错（可观测、行为不变）。
    async fn persist_planning_stale_wait_if_drifted(self: &Arc<Self>) -> bool {
        let eligible_node = {
            let engine = self.engine.lock().await;
            let should_check = engine.session().workspace_type == WorkspaceType::WorkItemPlan
                && self.session_record.flow_kind
                    != crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
                && engine.session().stage
                    == crate::product::workspace_engine::WorkspaceStage::Running
                && engine.active_node_type()
                    == Some(
                        crate::web::workspace_ws_types::TimelineNodeType::WorkItemPlanOutlineRun,
                    )
                && engine.active_run_id().is_none();
            if should_check {
                engine.active_timeline_node_id()
            } else {
                None
            }
        };
        let Some(node_id) = eligible_node else {
            return false;
        };
        let decision = planning_resume_decision_with_fresh_index(
            &self.app_paths,
            &self.session_record.project_id,
            &self.session_record.issue_id,
        )
        .await;
        let Ok(decision) = decision else {
            // 决策面错误交原恢复链报错路径（recover_outline_run 同判据重试）。
            return false;
        };
        match planning_resume_launch_decision(
            &decision,
            ProviderRunKind::WorkItemPlanOutlineRevision { feedback: None },
        ) {
            PlanningResumeLaunchDecision::ResumeExisting(_) => false,
            PlanningResumeLaunchDecision::SupersededWaiting => {
                match self.persist_planning_stale_wait_for_node(&node_id).await {
                    Ok(()) => {
                        eprintln!(
                            "[aria-recovery] planning context stale; waiting for explicit fresh session={} node={}",
                            self.session_id, node_id
                        );
                        true
                    }
                    Err(message) => {
                        // fail-closed（§4.3：先保存阻止旧 resume 的 recovery
                        // record）：捕获写失败时阻断全部自动链（不 spawn 漂移
                        // run），错误进 manager recovery_error 可观测。
                        eprintln!(
                            "[aria-recovery] planning stale wait persist failed session={}: {message}",
                            self.session_id
                        );
                        self.state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .recovery_error = Some(message);
                        true
                    }
                }
            }
        }
    }

    /// B2a（Task 5.1，设计 §5.3）：捕获中断 outline run 的 tagged
    /// run_context 并经 engine 2.6 安全线
    ///（`persist_lc_superseded_wait` + `reconcile_lc_resume_waiting`）落
    /// `planning_context_stale` 等待。feedback 取 durable node detail 冻结值
    ///（任何 take 前）；未知身份留 None 不倒算（纪元前态：map/run 绑定归
    /// 5.2 B2b1）。
    async fn persist_planning_stale_wait_for_node(
        self: &Arc<Self>,
        node_id: &str,
    ) -> Result<(), String> {
        let store = LifecycleStore::new(self.app_paths.clone());
        let detail = match store.load_node_detail(&self.session_id, node_id) {
            Ok(detail) => Some(detail),
            Err(ProductStoreError::NotFound { .. }) => None,
            Err(error) => {
                return Err(format!("load node detail failed for {node_id}: {error}"));
            }
        };
        let (project_id, issue_id, entity_id, workspace_type, author_provider, reviewer_provider) = {
            let engine = self.engine.lock().await;
            let session = engine.session();
            (
                session.project_id.clone(),
                session.issue_id.clone(),
                session.entity_id.clone(),
                session.workspace_type.clone(),
                session.author_provider.clone(),
                session.reviewer_provider.clone(),
            )
        };
        let repository =
            workspace_repository_for_session(&self.app_paths, &store, &self.session_record).ok();
        let plan = store
            .get_issue_work_item_plan(&project_id, &issue_id, &entity_id)
            .ok();
        let business = LcRecoveryBusinessContext {
            entity_id: entity_id.clone(),
            plan_id: Some(entity_id),
            workspace_type,
            business_repository_physical_id: repository
                .as_ref()
                .map(|repository| repository.id.clone()),
            business_repository_logical_id: repository
                .as_ref()
                .and_then(|repository| repository.logical_repository_id.as_ref())
                .map(|id| id.0.to_string()),
            business_checkout_id: repository
                .as_ref()
                .and_then(|repository| repository.primary_checkout_id.as_ref())
                .map(|id| id.0.to_string()),
            business_repository_path: repository
                .as_ref()
                .map(|repository| repository.path.to_string_lossy().to_string()),
            source_story_ids: plan
                .as_ref()
                .map(|plan| plan.source_story_spec_ids.clone())
                .unwrap_or_default(),
            source_design_ids: plan
                .as_ref()
                .map(|plan| plan.source_design_spec_ids.clone())
                .unwrap_or_default(),
            plan_options_ref: None,
            artifact_refs: detail
                .as_ref()
                .and_then(|detail| detail.artifact_ref.as_ref())
                .map(|artifact_ref| {
                    vec![LcRecoveryArtifactRef {
                        ref_id: artifact_ref.artifact_id.clone(),
                        version: Some(artifact_ref.version),
                        hash: None,
                    }]
                })
                .unwrap_or_default(),
            review_refs: Vec::new(),
            author_provider,
            effective_reviewer_provider: reviewer_provider,
            original_node_id: Some(node_id.to_string()),
            original_role_run_seq: None,
        };
        // 捕获 tag（§5.3）：revision 节点 → LegacyOutlineRevision（feedback
        // 冻结值）；首发/重建中断节点 → LegacyOutlineAuthor（fresh=按当前
        // inventory 重新 full outline，full fresh 臂归 5.3 B2b2 接管）。
        let outline_ref = || {
            detail
                .as_ref()
                .and_then(|detail| detail.artifact_ref.as_ref())
                .map(|artifact_ref| artifact_ref.artifact_id.clone())
        };
        let outline_version = || {
            detail
                .as_ref()
                .and_then(|detail| detail.artifact_ref.as_ref())
                .map(|artifact_ref| artifact_ref.version)
        };
        let run_context = if detail.as_ref().is_some_and(|detail| detail.is_revision) {
            LcRecoveryRunContext::LegacyOutlineRevision {
                business,
                normalized_feedback: detail
                    .as_ref()
                    .and_then(|detail| detail.revision_feedback.clone())
                    .unwrap_or_default(),
                outline_ref: outline_ref(),
                outline_version: outline_version(),
                outline_hash: None,
            }
        } else {
            LcRecoveryRunContext::LegacyOutlineAuthor {
                business,
                flow: crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy,
                original_provider_node_id: Some(node_id.to_string()),
                outline_ref: outline_ref(),
            }
        };
        let mut engine = self.engine.lock().await;
        engine
            .persist_lc_superseded_wait(run_context, "planning_context_stale")
            .await
            .map(|_| ())?;
        engine.reconcile_lc_resume_waiting().await;
        Ok(())
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

    /// 重启期在飞运行收口（B）：把 `Running` 的 turn 收口为既有终态
    /// `Failed(provider_err)`。
    ///
    /// 判据：本 manager 无活动 run（manager 单例契约 ⇒ 上一进程的在飞 provider
    /// 必已不存在）。`Reserved` turn 不在此列——durable 证明 provider 未启动，
    /// 既有恢复链可安全续跑 attempt 1。收口的 turn 不再被
    /// `recover_human_gate_turns` 安排 `ResumeSameTurn`（避免双跑副作用），
    /// 人工经既有 `human_gate_feedback` 提交新一轮继续。
    async fn close_lost_provider_human_gate_turns(self: &Arc<Self>) {
        if self.active_run().await.is_some() {
            return;
        }
        let closed = {
            let mut engine = self.engine.lock().await;
            engine
                .fail_human_gate_turns_lost_provider_at_restart()
                .await
        };
        if closed > 0 {
            eprintln!(
                "[aria-recovery] lost-provider human gate turns closed at restart session={} count={closed}",
                self.session_id
            );
            self.broadcast_current_session_state();
        }
    }

    /// 重启期在飞运行收口（B）：durable `status=running` ∧ stage `human_confirm`
    /// ∧ 无活动 run 的会话回落**既有**可恢复终态 `waiting_for_human`。
    ///
    /// 不启动 provider（已开始过的 provider 工作不自动重跑）；收口后人工可经
    /// 既有 wire 动作继续（`human_gate_feedback` / `confirm` / `abandon`）。
    async fn recover_human_confirm_lost_provider(self: &Arc<Self>) {
        if self.active_run().await.is_some() {
            return;
        }
        let mutated = {
            let mut engine = self.engine.lock().await;
            engine
                .recover_human_confirm_lost_provider_at_restart()
                .await
        };
        if mutated {
            eprintln!(
                "[aria-recovery] human_confirm lost-provider run recovered to waiting_for_human session={}",
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
            match planning_resume_launch_decision(&decision, run_kind) {
                PlanningResumeLaunchDecision::ResumeExisting(resume_kind) => {
                    let (outbound_tx, _outbound_rx) = mpsc::channel(1);
                    spawn_provider_run_from_handler(run_context.clone(), resume_kind, outbound_tx)
                        .await?;
                }
                // B2a（Task 5.1）：漂移 0 自动 spawn——捕获 run_context 落
                // planning_context_stale 等待（TOCTOU 安全臂：创建期预检
                // `persist_planning_stale_wait_if_drifted` 已先行，这里只兜
                // 同创建窗口内决策变化的残余）。
                PlanningResumeLaunchDecision::SupersededWaiting => {
                    let node_id = {
                        let engine = self.engine.lock().await;
                        engine.active_timeline_node_id()
                    };
                    if let Some(node_id) = node_id {
                        self.persist_planning_stale_wait_for_node(&node_id).await?;
                        self.broadcast_current_session_state();
                    }
                }
            }
        }
        Ok(())
    }
}

/// SG7a（plan v1.1 Task SG7a Step 1）：live CAS→镜像 sync→reload 与
/// production `load_durable_workspace_projection`（最新 durable record 纯
/// 投影）逐字段同值——真实 store CAS 链（reserve→claim→settle），不手工
/// 填 wire；纯投影零写（前后 durable 字节不变）；CAS 失败/错 scope 镜像
/// 保持。
#[cfg(test)]
mod sg_07a_tests {
    use super::*;
    use crate::product::json_store::{ProductStoreError, write_json};
    use crate::product::lifecycle_store::inputs::CreateWorkspaceSessionInput;
    use crate::product::models::{
        HumanGateReservation, HumanGateTurn, HumanGateTurnStatus, ProviderName,
        StructuralGovernanceState, StructuralGrantPhase, StructuralRevisionGrant,
        UserBudgetReceipt, UserBudgetReceiptKind, WorkspaceSessionStatus,
    };
    use crate::product::work_item_plan_policy::{
        HumanGateSnapshot, HumanReason, ProviderStartLedgerEntry, WorkItemPlanFlowKind,
    };
    use crate::product::workspace_engine::contract_prerevision::{
        ProtectedBusinessFacts, StructuralRepairInstruction, StructuralRepairSolution,
    };
    use crate::web::workspace_ws_types::{
        StructuralGovernanceSnapshot, WsOutMessage, project_structural_governance_snapshot,
    };

    struct Sg07aStore {
        _tmp: tempfile::TempDir,
        paths: ProductAppPaths,
        lifecycle: LifecycleStore,
        birth: WorkspaceSessionRecord,
    }

    fn sg_07a_record_path(
        paths: &ProductAppPaths,
        record: &WorkspaceSessionRecord,
    ) -> std::path::PathBuf {
        paths
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id))
    }

    fn sg_07a_store(project_id: &str) -> Sg07aStore {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        let mut record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: project_id.to_string(),
                issue_id: "issue_sg07a_proj".to_string(),
                entity_id: "plan_sg07a_proj".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: None,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("durable session record");
        record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
        record.status = WorkspaceSessionStatus::WaitingForHuman;
        record.human_gate_snapshot = Some(HumanGateSnapshot {
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: 0,
            manual_repairs_remaining: 3,
            trigger: HumanReason::NativeHumanRequired,
            resumable: false,
            accepted_feedback_turns: Some(0),
            candidate_recovery: None,
        });
        record.work_item_plan_source_revision_ref = Some("sg07a-source-ref".to_string());
        record.plan_candidate_ir_ref = Some("sg07a-ir-ref".to_string());
        record.mechanical_report_ref = Some("sg07a-report-ref".to_string());
        write_json(&sg_07a_record_path(&paths, &record), &record).expect("persist birth");
        Sg07aStore {
            _tmp: tmp,
            paths,
            lifecycle,
            birth: record,
        }
    }

    fn sg_07a_turn(session_id: &str, turn_id: &str, command_id: &str) -> HumanGateTurn {
        HumanGateTurn {
            turn_id: turn_id.to_string(),
            session_id: session_id.to_string(),
            command_id: command_id.to_string(),
            feedback_text: "修复互依赖环".to_string(),
            status: HumanGateTurnStatus::Reserved,
            attempt_no: 1,
            budget_reserved: 1,
            source_hash: "a".repeat(64),
            result_artifact_ref: None,
            failure_class: None,
            created_at: "2026-10-14T00:00:00Z".to_string(),
            updated_at: "2026-10-14T00:00:00Z".to_string(),
        }
    }

    /// 真实 store CAS：接受反馈 → 物化聚合状态 + 活动 hold（SG4a 权威链）。
    fn sg_07a_reserve(
        store: &Sg07aStore,
        turn_id: &str,
        command_id: &str,
    ) -> WorkspaceSessionRecord {
        let turn = sg_07a_turn(&store.birth.id, turn_id, command_id);
        let reservation = HumanGateReservation {
            command_id: command_id.to_string(),
            turn_id: turn_id.to_string(),
            provider_start_idempotency_key: format!("sg07a:start:{turn_id}"),
            reserved_at: "2026-10-14T00:00:00Z".to_string(),
        };
        store
            .lifecycle
            .compare_and_reserve_human_gate_turn(&store.birth, turn, reservation)
            .expect("reserve human gate turn")
            .0
    }

    fn sg_07a_receipt(
        kind: UserBudgetReceiptKind,
        turn_id: &str,
        command_id: &str,
    ) -> UserBudgetReceipt {
        UserBudgetReceipt {
            kind,
            turn_id: turn_id.to_string(),
            command_id: command_id.to_string(),
            attempt_reason: Some(crate::product::models::StructuralAttemptReason::HumanRevision),
            source_revision_ref: Some("sg07a-source-ref".to_string()),
            plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
            structure_report_ref: Some("sg07a-report-ref".to_string()),
            context_digest: Some("sha256:sg07a-context".to_string()),
            reason: Some("sg07a five-state fixture".to_string()),
            settled_at: "2026-10-14T00:00:00Z".to_string(),
        }
    }

    /// SG5.3/SG6a.1 同款 read-modify-write 注入：保留既有 CAS 事实
    ///（hold/ledger/预算），只补无生产 producer 的耐久形状（producer 归
    /// SG6a.3）。
    fn sg_07a_mutate<F>(store: &Sg07aStore, mutate: F) -> WorkspaceSessionRecord
    where
        F: FnOnce(&mut WorkspaceSessionRecord),
    {
        let mut record = store
            .lifecycle
            .get_workspace_session(&store.birth.id)
            .expect("latest record");
        mutate(&mut record);
        write_json(&sg_07a_record_path(&store.paths, &record), &record)
            .expect("persist fixture mutation");
        record
    }

    fn sg_07a_instruction() -> StructuralRepairInstruction {
        StructuralRepairInstruction {
            repair_id: "repair_sg07a_0001".to_string(),
            failure_id: "failure_sg07a_0001".to_string(),
            rule_version: "structure_rules/v1".to_string(),
            source_hash: "sha256:sg07a-failing-source".to_string(),
            context_digest: "sha256:sg07a-context".to_string(),
            affected_work_item_ids: vec!["work_item_0001".to_string()],
            affected_spans: Vec::new(),
            conflict_edges: Vec::new(),
            allowed_change_closure: vec!["heading".to_string()],
            forbidden_operations: vec!["delete_required_inputs".to_string()],
            protected_facts: ProtectedBusinessFacts {
                business_request_digest: "d".repeat(64),
                confirmed_design_digest: None,
                protected_field_refs: Vec::new(),
                protected_contract_refs: Vec::new(),
                authorized_member_ids: Vec::new(),
            },
            solution: StructuralRepairSolution::Candidates {
                candidates: Vec::new(),
            },
            required_action: "修复非法 heading".to_string(),
            expected_predicates: Vec::new(),
            previous_valid_refs: None,
            instruction_digest: "e".repeat(64),
        }
    }

    fn sg_07a_snapshot_of(frame: &WsOutMessage) -> StructuralGovernanceSnapshot {
        match frame {
            WsOutMessage::SessionState {
                structural_governance: Some(snapshot),
                ..
            } => snapshot.clone(),
            WsOutMessage::SessionState {
                structural_governance: None,
                ..
            } => panic!("structural governance snapshot missing"),
            other => panic!("expected SessionState, got {other:?}"),
        }
    }

    /// 每态同值链：manager production `load_durable_workspace_projection`
    ///（最新 record 纯投影，出生 record 仅身份对照）＝ 引擎纯构造
    /// build_session_state ＝ live sync helper 吸收后的镜像 ＝
    /// reload_session_from_record 重建——逐字段相等＋快照事实与 durable
    /// 单源对账。
    fn sg_07a_assert_chain(
        store: &Sg07aStore,
        latest: &WorkspaceSessionRecord,
    ) -> StructuralGovernanceSnapshot {
        // ① manager production 纯投影。
        let identity = WorkspaceSessionIdentity::from_bound_record(&store.birth);
        let projection = load_durable_workspace_projection(&store.paths, &identity)
            .expect("pure projection of latest record");
        let projected = sg_07a_snapshot_of(&projection.session_state);

        // ② 引擎纯构造（ProjectionOnly）build_session_state。
        let engine = {
            let lifecycle = LifecycleStore::new(store.paths.clone());
            let (event_tx, _event_rx) = mpsc::channel(1);
            let checkpoint_store = Arc::new(CheckpointStore::new(
                store
                    .paths
                    .issue_lifecycle_root(&latest.project_id, &latest.issue_id),
            ));
            WorkspaceEngine::new_persistent_for_projection(
                checkpoint_store,
                lifecycle,
                event_tx,
                WorkspaceSession::from_record(latest.clone()),
            )
            .expect("projection engine")
        };
        let built = sg_07a_snapshot_of(&engine.build_session_state());
        assert_eq!(projected, built, "manager 纯投影与引擎构造必须同值");

        // ③ live CAS→sync helper：旧镜像吸收同 scope 最新 record。
        let mut synced = WorkspaceSession::from_record(store.birth.clone());
        synced
            .sync_structural_governance_from_record(latest)
            .expect("same-scope sync");
        assert_eq!(
            synced.structural_governance_state, latest.structural_governance_state,
            "sync 后镜像必须等于最新 record 聚合状态"
        );
        assert_eq!(synced.human_gate_snapshot, latest.human_gate_snapshot);
        assert_eq!(synced.provider_start_ledger, latest.provider_start_ledger);
        assert_eq!(
            synced.work_item_plan_source_revision_ref,
            latest.work_item_plan_source_revision_ref
        );
        assert_eq!(synced.plan_candidate_ir_ref, latest.plan_candidate_ir_ref);
        assert_eq!(synced.mechanical_report_ref, latest.mechanical_report_ref);

        // ④ reload_session_from_record 重建后 build 同值。
        let mut live_engine = {
            let lifecycle = LifecycleStore::new(store.paths.clone());
            let (event_tx, _event_rx) = mpsc::channel(1);
            let checkpoint_store = Arc::new(CheckpointStore::new(
                store
                    .paths
                    .issue_lifecycle_root(&store.birth.project_id, &store.birth.issue_id),
            ));
            WorkspaceEngine::new_persistent_for_projection(
                checkpoint_store,
                lifecycle,
                event_tx,
                WorkspaceSession::from_record(store.birth.clone()),
            )
            .expect("reload engine")
        };
        live_engine.reload_session_from_record(latest.clone());
        let reloaded = sg_07a_snapshot_of(&live_engine.build_session_state());
        assert_eq!(projected, reloaded, "reload 重建必须与纯投影同值");

        // ⑤ 快照事实与 durable 单源对账。
        let state = latest.structural_governance_state.as_ref().unwrap();
        assert_eq!(projected.user_receipts, state.user_receipts);
        assert_eq!(projected.user_holds, state.user_holds);
        assert_eq!(
            projected.available_user_budget,
            state.available_user_budget()
        );
        assert_eq!(
            projected.accepted_feedback_turns,
            latest
                .human_gate_snapshot
                .as_ref()
                .and_then(|gate| gate.accepted_feedback_turns)
        );
        assert_eq!(
            projected.actual_provider_starts,
            latest
                .provider_start_ledger
                .iter()
                .filter(|entry| entry.is_actual_launch_observation())
                .count() as u32
        );
        projected
    }

    fn sg_07a_durable_bytes(paths: &ProductAppPaths) -> Vec<(String, Vec<u8>)> {
        fn walk(dir: &std::path::Path, prefix: String, out: &mut Vec<(String, Vec<u8>)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
                if path.is_dir() {
                    walk(&path, name, out);
                } else {
                    out.push((name, std::fs::read(&path).expect("read durable file")));
                }
            }
        }
        let mut snapshot = Vec::new();
        walk(&paths.projects_root(), String::new(), &mut snapshot);
        snapshot
    }

    #[test]
    fn sg_07a_live_cas_and_reload_match_latest_durable_projection() {
        let store = sg_07a_store("project_sg07a_live_cas");

        // 1) live CAS：reserve（SG4a 权威链）→ 最新 record 纯投影同值。
        let turn = sg_07a_turn(&store.birth.id, "turn_sg07a_live", "cmd_sg07a_live");
        let reservation = HumanGateReservation {
            command_id: "cmd_sg07a_live".to_string(),
            turn_id: "turn_sg07a_live".to_string(),
            provider_start_idempotency_key: "sg07a:start:live".to_string(),
            reserved_at: "2026-10-14T00:00:00Z".to_string(),
        };
        let reserved = store
            .lifecycle
            .compare_and_reserve_human_gate_turn(&store.birth, turn, reservation)
            .expect("reserve human gate turn")
            .0;
        sg_07a_assert_chain(&store, &reserved);

        // 2) live CAS：注入 grant（producer 归 SG6a.3，fixture 注入沿
        //    SG5.3/SG6a.1 同款）后 claim_structural_attempt 真实推进
        //    attempt/phase/prompt 绑定。
        let mut granted = store
            .lifecycle
            .get_workspace_session(&store.birth.id)
            .expect("latest before grant injection");
        granted
            .structural_governance_state
            .get_or_insert_with(StructuralGovernanceState::default)
            .current_grant = Some(StructuralRevisionGrant {
            grant_id: "structural_grant_turn_sg07a_live".to_string(),
            authorization_hash: "sha256:sg07a-authorization".to_string(),
            turn_id: Some("turn_sg07a_live".to_string()),
            command_id: Some("cmd_sg07a_live".to_string()),
            candidate_source_revision_ref: Some("sg07a-source-ref".to_string()),
            plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
            mechanical_report_ref: Some("sg07a-report-ref".to_string()),
            phase: Some(StructuralGrantPhase::AuthorRunning),
            attempt_no: 1,
            prompt_ref: None,
            prompt_digest: None,
            instruction_digest: None,
            failing_source_hash: None,
            review_identity: None,
            granted_at: "2026-10-14T00:00:00Z".to_string(),
        });
        write_json(&sg_07a_record_path(&store.paths, &granted), &granted)
            .expect("persist grant injection");
        let mut running_turn = sg_07a_turn(&store.birth.id, "turn_sg07a_live", "cmd_sg07a_live");
        running_turn.status = HumanGateTurnStatus::Running;
        let granted = store
            .lifecycle
            .update_human_gate_turn(&granted, running_turn)
            .expect("mark turn running");
        sg_07a_assert_chain(&store, &granted);

        let claim = store
            .lifecycle
            .claim_structural_attempt(
                &granted,
                "turn_sg07a_live",
                &sg_07a_instruction(),
                "sg07a-frozen-prompt-ref",
                "sha256:sg07a-prompt",
            )
            .expect("claim structural attempt");
        assert!(
            matches!(
                claim,
                crate::product::models::StructuralClaimOutcome::Claimed { attempt_no: 2, .. }
            ),
            "claim 必须推进到 attempt2：{claim:?}"
        );
        let claimed = store
            .lifecycle
            .get_workspace_session(&store.birth.id)
            .expect("latest after claim");
        sg_07a_assert_chain(&store, &claimed);

        // 态②：注入 grant phase Reviewing（turn 仍 Running）→ settle
        // delivered_charge（真实 CAS）——reload（from_record 镜像）、sync
        // 与 manager 纯投影逐字段同值；纯投影前后 durable 字节不变。
        let reviewing = sg_07a_mutate(&store, |record| {
            let state = record
                .structural_governance_state
                .as_mut()
                .expect("state after claim");
            state
                .current_grant
                .as_mut()
                .expect("grant after claim")
                .phase = Some(StructuralGrantPhase::Reviewing);
        });
        // P3 修复（审查）：零写断言收缩到纯读段——基线在 settle 写完成
        // 后、assert_chain（manager 纯投影+引擎构造+sync+reload 全链）
        // 前，窗口比较完整 (name, bytes) 向量相等。
        let settled = store
            .lifecycle
            .settle_structural_user_budget(
                &reviewing,
                "turn_sg07a_live",
                UserBudgetReceipt {
                    kind: UserBudgetReceiptKind::DeliveredCharge,
                    turn_id: "turn_sg07a_live".to_string(),
                    command_id: "cmd_sg07a_live".to_string(),
                    attempt_reason: Some(
                        crate::product::models::StructuralAttemptReason::HumanRevision,
                    ),
                    source_revision_ref: Some("sg07a-source-ref".to_string()),
                    plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
                    structure_report_ref: Some("sg07a-report-ref".to_string()),
                    context_digest: Some("sha256:sg07a-context".to_string()),
                    reason: Some("sg07a live-cas fixture".to_string()),
                    settled_at: "2026-10-14T00:00:00Z".to_string(),
                },
            )
            .expect("settle delivered charge");
        let bytes_baseline = sg_07a_durable_bytes(&store.paths);
        let delivered_snapshot = sg_07a_assert_chain(&store, &settled);
        assert_eq!(
            delivered_snapshot.available_user_budget, 2,
            "3 − 1 delivered"
        );
        assert_eq!(delivered_snapshot.active_hold_count, 0);
        assert!(
            matches!(
                delivered_snapshot
                    .current_grant
                    .as_ref()
                    .map(|grant| grant.phase),
                Some(Some(StructuralGrantPhase::Reviewing))
            ),
            "grant phase 沿注入的 Reviewing 进度"
        );
        let bytes_after = sg_07a_durable_bytes(&store.paths);
        assert_eq!(
            bytes_baseline, bytes_after,
            "纯读窗口（assert_chain 全链）durable 字节必须零写"
        );

        // 态③：结构耗尽 released_structural——预算全额恢复（不实扣）。
        let store3 = sg_07a_store("project_sg07a_release");
        let reserved3 = sg_07a_reserve(&store3, "turn_sg07a_rel", "cmd_sg07a_rel");
        let released = store3
            .lifecycle
            .settle_structural_user_budget(
                &reserved3,
                "turn_sg07a_rel",
                sg_07a_receipt(
                    UserBudgetReceiptKind::ReleasedStructural,
                    "turn_sg07a_rel",
                    "cmd_sg07a_rel",
                ),
            )
            .expect("settle released structural");
        let released_snapshot = sg_07a_assert_chain(&store3, &released);
        assert_eq!(
            released_snapshot.available_user_budget, 3,
            "released 不实扣"
        );
        assert_eq!(released_snapshot.active_hold_count, 0);
        assert_eq!(
            released_snapshot.user_receipts.len(),
            1,
            "released receipt 历史保留"
        );

        // 态④：provider/取消非结构失败扣次——available 2、delivered 0。
        let store4 = sg_07a_store("project_sg07a_failure");
        let reserved4 = sg_07a_reserve(&store4, "turn_sg07a_fail", "cmd_sg07a_fail");
        let failed = store4
            .lifecycle
            .settle_structural_user_budget(
                &reserved4,
                "turn_sg07a_fail",
                sg_07a_receipt(
                    UserBudgetReceiptKind::NonstructuralFailureCharge,
                    "turn_sg07a_fail",
                    "cmd_sg07a_fail",
                ),
            )
            .expect("settle nonstructural failure charge");
        let failed_snapshot = sg_07a_assert_chain(&store4, &failed);
        assert_eq!(
            failed_snapshot.available_user_budget, 2,
            "3 − 1 failure charge"
        );
        assert_eq!(failed_snapshot.active_hold_count, 0);
        assert!(
            failed_snapshot
                .user_receipts
                .iter()
                .all(|receipt| receipt.kind == UserBudgetReceiptKind::NonstructuralFailureCharge)
        );

        // 态⑤：grant 终态（Resolved）＋无活动授权、同 scope checkpoint
        // 历史/已解决事实仍在、actual start 证据在 ledger。
        let store5 = sg_07a_store("project_sg07a_terminal");
        let reserved5 = sg_07a_reserve(&store5, "turn_sg07a_term", "cmd_sg07a_term");
        store5
            .lifecycle
            .settle_structural_user_budget(
                &reserved5,
                "turn_sg07a_term",
                sg_07a_receipt(
                    UserBudgetReceiptKind::ReleasedStructural,
                    "turn_sg07a_term",
                    "cmd_sg07a_term",
                ),
            )
            .expect("settle before terminal");
        let terminal = sg_07a_mutate(&store5, |record| {
            let state = record
                .structural_governance_state
                .get_or_insert_with(StructuralGovernanceState::default);
            state.current_grant = Some(StructuralRevisionGrant {
                grant_id: "structural_grant_turn_sg07a_term".to_string(),
                authorization_hash: "sha256:sg07a-authorization".to_string(),
                turn_id: Some("turn_sg07a_term".to_string()),
                command_id: Some("cmd_sg07a_term".to_string()),
                candidate_source_revision_ref: Some("sg07a-source-ref".to_string()),
                plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
                mechanical_report_ref: Some("sg07a-report-ref".to_string()),
                phase: Some(StructuralGrantPhase::Resolved),
                attempt_no: 2,
                prompt_ref: Some("sg07a-frozen-prompt-ref".to_string()),
                prompt_digest: Some("sha256:sg07a-prompt".to_string()),
                instruction_digest: Some("sha256:sg07a-instruction".to_string()),
                failing_source_hash: Some("sha256:sg07a-failing-source".to_string()),
                review_identity: None,
                granted_at: "2026-10-14T00:00:00Z".to_string(),
            });
            state.structure_checkpoints = vec![crate::product::models::StructuralCheckpoint {
                checkpoint_id: "sg07a-terminal-checkpoint".to_string(),
                source_revision_ref: Some("sg07a-source-ref".to_string()),
                plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
                structure_report_ref: Some("sg07a-report-ref".to_string()),
                artifact_version: Some(2),
                source_hash: Some("sha256:sg07a-failing-source".to_string()),
                context_digest: Some("sha256:sg07a-context".to_string()),
                authorization_hash: Some("sha256:sg07a-authorization".to_string()),
                receipt_ref: None,
                review_scope: Some("sg07a-review-scope".to_string()),
                status: crate::product::models::StructuralCheckpointStatus::Superseded,
                recorded_at: "2026-10-14T00:00:00Z".to_string(),
            }];
            state.resolved_structure_facts = vec![crate::product::models::ResolvedStructureFact {
                predicate: "dependency_cycle_resolved".to_string(),
                evidence_refs: vec!["sg07a-report-ref".to_string()],
                source_hash: None,
                resolved_at: "2026-10-14T00:00:00Z".to_string(),
            }];
            state.current_structure_report_ref = Some("sg07a-report-ref".to_string());
            state.current_context_digest = Some("sha256:sg07a-context".to_string());
            record.provider_start_ledger.push(ProviderStartLedgerEntry {
                provider_start_idempotency_key: "sg07a:start:terminal-actual".to_string(),
                started: true,
                provider: Some("fake".to_string()),
                started_at: Some("2026-10-14T00:00:00Z".to_string()),
            });
        });
        let terminal_snapshot = sg_07a_assert_chain(&store5, &terminal);
        let terminal_grant = terminal_snapshot
            .current_grant
            .expect("terminal grant kept");
        assert_eq!(terminal_grant.phase, Some(StructuralGrantPhase::Resolved));
        assert_eq!(terminal_grant.attempt_no, 2);
        assert_eq!(
            terminal_snapshot.structure_checkpoints.len(),
            1,
            "checkpoint 历史仍在"
        );
        assert_eq!(terminal_snapshot.resolved_structure_facts.len(), 1);
        assert_eq!(terminal_snapshot.actual_provider_starts, 1);
        assert_eq!(terminal_snapshot.available_user_budget, 3);

        // 4) 负例：CAS 失败（stale expected）不推进镜像——runtime 镜像
        //    保持上次成功 CAS 事实。
        let mut runtime = WorkspaceSession::from_record(reserved.clone());
        runtime
            .sync_structural_governance_from_record(&claimed)
            .expect("sync to claimed state");
        assert_eq!(
            runtime.structural_governance_state, claimed.structural_governance_state,
            "同步后镜像必须等于最新成功 CAS 事实"
        );
        let stale_settle = store.lifecycle.settle_structural_user_budget(
            &claimed,
            "turn_sg07a_live",
            UserBudgetReceipt {
                kind: UserBudgetReceiptKind::ReleasedStructural,
                turn_id: "turn_sg07a_live".to_string(),
                command_id: "cmd_sg07a_live".to_string(),
                attempt_reason: None,
                source_revision_ref: None,
                plan_candidate_ir_ref: None,
                structure_report_ref: None,
                context_digest: None,
                reason: None,
                settled_at: "2026-10-14T00:00:00Z".to_string(),
            },
        );
        assert!(
            matches!(stale_settle, Err(ProductStoreError::Conflict { .. })),
            "stale expected 的 CAS 必须失败：{stale_settle:?}"
        );
        assert_eq!(
            runtime.structural_governance_state, claimed.structural_governance_state,
            "CAS 失败后镜像保持不变"
        );

        // 5) 负例：错 scope 同步返回 IdentityMismatch 且镜像保持。
        let mut wrong_scope_record = settled.clone();
        wrong_scope_record.entity_id = "entity_other_scope".to_string();
        let sync_result = runtime.sync_structural_governance_from_record(&wrong_scope_record);
        assert!(
            matches!(sync_result, Err(ProductStoreError::IdentityMismatch { .. })),
            "错 scope 同步必须失败：{sync_result:?}"
        );
        assert_eq!(
            runtime.structural_governance_state, claimed.structural_governance_state,
            "错 scope 拒绝后镜像保持不变"
        );

        // 6) 纯函数守卫：投影不重算 accepted/actual（单源透传）。
        let state = settled.structural_governance_state.as_ref().unwrap();
        let projected = project_structural_governance_snapshot(
            state,
            settled.human_gate_snapshot.as_ref(),
            &settled.provider_start_ledger,
            settled.work_item_plan_source_revision_ref.as_deref(),
            settled.plan_candidate_ir_ref.as_deref(),
            settled.mechanical_report_ref.as_deref(),
        );
        assert_eq!(
            projected.accepted_feedback_turns,
            settled
                .human_gate_snapshot
                .as_ref()
                .and_then(|gate| gate.accepted_feedback_turns)
        );
        assert_eq!(
            projected.actual_provider_starts,
            settled
                .provider_start_ledger
                .iter()
                .filter(|entry| entry.is_actual_launch_observation())
                .count() as u32
        );
    }
}
