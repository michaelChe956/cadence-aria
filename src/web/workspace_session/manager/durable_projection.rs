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

include!("durable_projection/sg_07a_tests.inc.rs");
