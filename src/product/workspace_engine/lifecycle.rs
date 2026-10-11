use super::*;
use crate::product::models::{LcRecoveryRunContext, LcResumeRecoveryRecord, LcResumeRecoveryState};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;

/// T4.1（REQ-REG-15）：确认 trust 准备的同步预检产物（lifecycle.rs 私有）。
/// `Skip`=免登记（非 LC 会话/无有效 trust provider，零调用放行）；
/// `Ensure`=待执行的 registry 调用参数（owned 数据，不持 engine 借用跨
/// await——见 `ensure_confirmation_trust_ready`）。
enum ConfirmationTrustEnsure {
    Skip,
    Ensure {
        precondition:
            std::sync::Arc<dyn crate::product::logical_codebase::ProviderTrustPrecondition>,
        session_id: String,
        project_id: String,
        operation_id: String,
        registry_lc_id: String,
        canonical_root: std::path::PathBuf,
        providers: Vec<ProviderName>,
    },
}

/// spec-design-dialog-revision T6：HumanConfirm/ReviewDecision 已从 Story/Design 流程退役，
/// 存量会话恢复时迁移回 AuthorConfirm（保留产物与消息，review verdict 留在消息流）。
/// 仅迁移 stage，不触碰 timeline/持久化存储（恢复语义由消息流保留，懒迁移随下次持久化落盘）。
pub(crate) fn recover_story_design_retired_stage_fallback(
    mut session: WorkspaceSession,
) -> WorkspaceSession {
    let retired = matches!(
        session.stage,
        WorkspaceStage::HumanConfirm | WorkspaceStage::ReviewDecision
    );
    if retired
        && matches!(
            session.workspace_type,
            WorkspaceType::Story | WorkspaceType::Design
        )
    {
        session.stage = WorkspaceStage::AuthorConfirm;
    }
    session
}

fn recover_complete_artifact_misclassified_as_text_fallback(
    checkpoint_store: &CheckpointStore,
    lifecycle_store: &LifecycleStore,
    session: &mut WorkspaceSession,
    timeline_nodes: &mut Vec<TimelineNode>,
    active_node_id: &mut Option<String>,
    artifact_versions: &mut Vec<ArtifactVersion>,
) -> Result<bool, String> {
    if !matches!(
        session.workspace_type,
        WorkspaceType::Story | WorkspaceType::Design
    ) {
        return Ok(false);
    }
    let Some(source_node_id) = active_node_id.clone() else {
        return Ok(false);
    };
    let Some(source_node_index) = timeline_nodes.iter().position(|node| {
        node.node_id == source_node_id
            && matches!(
                node.node_type,
                TimelineNodeType::AuthorRun | TimelineNodeType::Revision
            )
            && node.status == TimelineNodeStatus::Paused
            && node
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("等待用户选择"))
    }) else {
        return Ok(false);
    };
    let Some(assistant_message_index) = session
        .messages
        .iter()
        .rposition(|message| message.role == "assistant")
    else {
        return Ok(false);
    };
    let full_content = &session.messages[assistant_message_index].content;
    if detect_author_choice_request(full_content, &session.workspace_type).is_some() {
        return Ok(false);
    }
    let Some(artifact_markdown) =
        selected_workspace_artifact_markdown(full_content, &session.workspace_type)
    else {
        return Ok(false);
    };

    let payload = ArtifactPayload::Markdown {
        markdown: artifact_markdown.clone(),
        diff: None,
    };
    let mut recovered_artifact_versions = artifact_versions.clone();
    let current_version = recovered_artifact_versions
        .iter()
        .find(|version| version.is_current && version.markdown() == artifact_markdown);
    let version = if let Some(current) = current_version {
        current.version
    } else {
        for version in &mut recovered_artifact_versions {
            version.is_current = false;
        }
        let version = recovered_artifact_versions
            .iter()
            .map(|version| version.version)
            .max()
            .unwrap_or(0)
            + 1;
        recovered_artifact_versions.push(ArtifactVersion {
            version,
            payload: payload.clone(),
            generated_by: session.author_provider.clone(),
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            source_node_id: source_node_id.clone(),
        });
        version
    };

    lifecycle_store
        .ensure_version(AppendSpecVersionInput {
            project_id: session.project_id.clone(),
            issue_id: session.issue_id.clone(),
            entity_id: session.entity_id.clone(),
            markdown: artifact_markdown.clone(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .map_err(|error| format!("ensure recovered artifact spec version failed: {error}"))?;

    let now = chrono::Utc::now().to_rfc3339();
    let mut recovered_timeline_nodes = timeline_nodes.clone();
    let source_node = &mut recovered_timeline_nodes[source_node_index];
    source_node.status = TimelineNodeStatus::Completed;
    source_node.summary = Some("已恢复完整 artifact".to_string());
    source_node.completed_at = Some(now.clone());
    source_node.artifact_ref = Some("artifact_current".to_string());
    let author_confirm_node_id = format!("timeline_node_{:03}", recovered_timeline_nodes.len() + 1);
    recovered_timeline_nodes.push(TimelineNode {
        node_id: author_confirm_node_id.clone(),
        node_type: TimelineNodeType::AuthorConfirm,
        agent: None,
        stage: WsWorkspaceStage::AuthorConfirm,
        round: None,
        status: TimelineNodeStatus::Active,
        title: "Author 结果确认".to_string(),
        summary: Some("已恢复误判为文本选择的完整 artifact".to_string()),
        started_at: now.clone(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: Some("artifact_current".to_string()),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: session.author_provider.clone(),
            reviewer: session.reviewer_provider.clone(),
            review_rounds: session.review_rounds,
            permission_modes: session.permission_modes.clone(),
        },
        retry: None,
    });

    lifecycle_store
        .save_artifact_versions(&session.session_id, &recovered_artifact_versions)
        .map_err(|error| format!("save recovered artifact versions failed: {error}"))?;
    lifecycle_store
        .update_workspace_session_status(
            &session.session_id,
            WorkspaceSessionStatus::WaitingForHuman,
        )
        .map_err(|error| format!("save recovered workspace status failed: {error}"))?;

    let artifact_ref = ArtifactRef {
        artifact_id: format!("artifact_version_{version:03}"),
        version,
    };
    let message_index = u32::try_from(session.messages.len()).map_err(|_| {
        "workspace message count overflow during text fallback recovery".to_string()
    })?;
    let checkpoints = checkpoint_store
        .list_checkpoints(&session.session_id)
        .map_err(|error| format!("load checkpoints for text fallback recovery failed: {error}"))?;
    let checkpoint_id = if let Some(checkpoint) = checkpoints.iter().find(|checkpoint| {
        checkpoint.message_index == message_index
            && checkpoint.stage == WorkspaceStage::AuthorConfirm.as_str()
            && checkpoint
                .artifact_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.markdown_or_empty() == artifact_markdown)
    }) {
        checkpoint.id.clone()
    } else {
        checkpoint_store
            .create_checkpoint(
                &session.session_id,
                message_index,
                Some(&payload),
                WorkspaceStage::AuthorConfirm.as_str(),
            )
            .map_err(|error| format!("save recovered artifact checkpoint failed: {error}"))?
            .id
    };
    match lifecycle_store.load_node_detail(&session.session_id, &source_node_id) {
        Ok(mut detail) => {
            detail.status = TimelineNodeStatus::Completed;
            detail.ended_at = Some(now);
            detail.artifact_ref = Some(artifact_ref);
            lifecycle_store
                .save_node_detail(&session.session_id, &source_node_id, &detail)
                .map_err(|error| format!("save recovered node detail failed: {error}"))?;
        }
        Err(ProductStoreError::NotFound { .. }) => {}
        Err(error) => return Err(format!("load recovered node detail failed: {error}")),
    }
    lifecycle_store
        .save_timeline_nodes(&session.session_id, &recovered_timeline_nodes)
        .map_err(|error| format!("commit recovered timeline nodes failed: {error}"))?;

    session.messages[assistant_message_index].checkpoint_id = Some(checkpoint_id);
    session.artifact = Some(payload);
    session.stage = WorkspaceStage::AuthorConfirm;
    *artifact_versions = recovered_artifact_versions;
    *timeline_nodes = recovered_timeline_nodes;
    *active_node_id = Some(author_confirm_node_id);
    Ok(true)
}
/// A1b1（设计 §4.4 / Plan 接口字典）：`new_persistent` 与
/// `new_persistent_for_projection` 共享纯持久读核心的派生产物——两条持久
/// 构造路径（真实恢复 / 纯投影）唯一的 session-state 组装中间态，不建
/// 第二权威 model。
struct PersistentDerivation {
    session: WorkspaceSession,
    timeline_nodes: Vec<TimelineNode>,
    active_node_id: Option<String>,
    artifact_versions: Vec<ArtifactVersion>,
    latest_review_verdict: Option<ReviewVerdict>,
    pending_author_choice: Option<PendingAuthorChoice>,
    plan_repair_snapshot: Option<PlanRepairSessionSnapshotDto>,
}

impl WorkspaceEngine {
    pub fn new(
        checkpoint_store: Arc<CheckpointStore>,
        event_tx: mpsc::Sender<EngineEvent>,
        session: WorkspaceSession,
    ) -> Self {
        let (timeline_nodes, active_node_id) = initial_timeline(&session);
        Self {
            checkpoint_store,
            lifecycle_store: None,
            event_tx,
            session,
            cancel: CancellationToken::new(),
            timeline_nodes,
            active_node_id,
            artifact_versions: Vec::new(),
            latest_review_verdict: None,
            pending_revision_context: None,
            pending_author_choice: None,
            active_run_id: None,
            stream_buffers: HashMap::new(),
            work_item_plan_author_retry_count: 0,
            work_item_plan_revision_retry_count: 0,
            work_item_draft_repair_states: HashMap::new(),
            outline_revision_recovery_error: None,
            outline_revision_crash_after: None,
            plan_repair_crash_after: None,
            plan_repair_snapshot: None,
            #[cfg(test)]
            policy_route_before_persist: None,
            logical_provider_gateway: None,
            logical_launch_fingerprints: HashMap::new(),
            last_gate_close_compile_failure: None,
            confirmation_trust_context: None,
        }
    }

    pub fn new_persistent(
        checkpoint_store: Arc<CheckpointStore>,
        lifecycle_store: LifecycleStore,
        event_tx: mpsc::Sender<EngineEvent>,
        mut session: WorkspaceSession,
    ) -> Self {
        let plan_repair_transition_recovery_error = recover_plan_repair_transition(
            &lifecycle_store,
            &session.project_id,
            &session.issue_id,
            &session.session_id,
        )
        .err()
        .map(|error| format!("plan repair journal recovery failed: {error:?}"));
        let outline_revision_recovery_error = recover_work_item_plan_outline_revision_transaction(
            &lifecycle_store,
            &session.project_id,
            &session.issue_id,
            &session.entity_id,
            &session.session_id,
        )
        .err();
        let persisted_timeline_nodes = lifecycle_store
            .load_timeline_nodes_for_issue_session(
                &session.project_id,
                &session.issue_id,
                &session.session_id,
            )
            .unwrap_or_default();
        let mut persisted_artifact_versions = lifecycle_store
            .list_artifact_versions_for_issue_session(
                &session.project_id,
                &session.issue_id,
                &session.session_id,
            )
            .unwrap_or_default();
        let (mut timeline_nodes, mut active_node_id) = Self::derive_persistent_timeline(
            &mut session,
            persisted_timeline_nodes,
            &persisted_artifact_versions,
        );
        if let Err(error) = recover_complete_artifact_misclassified_as_text_fallback(
            &checkpoint_store,
            &lifecycle_store,
            &mut session,
            &mut timeline_nodes,
            &mut active_node_id,
            &mut persisted_artifact_versions,
        ) {
            tracing::warn!(%error, "failed to recover complete artifact from text fallback state");
        }
        if let Err(error) = recover_work_item_plan_outline_review_schema_fallback(
            &lifecycle_store,
            &mut session,
            &mut timeline_nodes,
            &mut active_node_id,
        ) {
            tracing::warn!(%error, "failed to recover WorkItemPlan outline review schema fallback");
        }
        let derivation = Self::finalize_persistent_derivation(
            &lifecycle_store,
            session,
            timeline_nodes,
            active_node_id,
            persisted_artifact_versions,
            plan_repair_transition_recovery_error,
        );
        Self::persistent_engine(
            checkpoint_store,
            lifecycle_store,
            event_tx,
            derivation,
            outline_revision_recovery_error,
        )
    }

    /// A1b1（设计 §4.4 / Plan 接口字典）：`ProjectionOnly` 纯持久读构造——与
    /// `new_persistent` 共用纯读核心，但不执行任何恢复写：plan repair journal
    /// 修复、outline revision transaction reconcile、text fallback 与 outline
    /// review schema fallback 均不在内（durable 投影不得沿 fallback 偷偷恢复）。
    /// durable 读失败（IO/JSON）typed 上抛，绝不降级为初始 timeline 假态；
    /// LC 恢复等待的 reconcile 只属于真实恢复入口（A2b）。
    pub fn new_persistent_for_projection(
        checkpoint_store: Arc<CheckpointStore>,
        lifecycle_store: LifecycleStore,
        event_tx: mpsc::Sender<EngineEvent>,
        mut session: WorkspaceSession,
    ) -> Result<Self, ProductStoreError> {
        let persisted_timeline_nodes = lifecycle_store.load_timeline_nodes_for_issue_session(
            &session.project_id,
            &session.issue_id,
            &session.session_id,
        )?;
        let persisted_artifact_versions = lifecycle_store
            .list_artifact_versions_for_issue_session(
                &session.project_id,
                &session.issue_id,
                &session.session_id,
            )?;
        let (timeline_nodes, active_node_id) = Self::derive_persistent_timeline(
            &mut session,
            persisted_timeline_nodes,
            &persisted_artifact_versions,
        );
        let derivation = Self::finalize_persistent_derivation(
            &lifecycle_store,
            session,
            timeline_nodes,
            active_node_id,
            persisted_artifact_versions,
            // 纯投影不执行 plan repair journal 恢复：无恢复错误可注入。
            None,
        );
        let mut engine = Self::persistent_engine(
            checkpoint_store,
            lifecycle_store,
            event_tx,
            derivation,
            // 纯投影不执行 outline revision transaction 恢复。
            None,
        );
        // A2b（Task 2.6，设计 §4.4）：LC 等待在纯投影中只**派生**对外形态
        // ——旧 provider 节点视作已收口（无 busy、无 active provider 节点；
        // run 阶段回 PrepareContext），全部内存态、零磁盘写；真正的收口写
        // 入口只属于 manager `recover_on_creation` 与 live mismatch 收口
        // （fallback 不得成为另一写方）。Resolved 历史证据不影响派生。
        if let Some(recovery) = engine.session.lc_resume_recovery.clone()
            && recovery.state != LcResumeRecoveryState::Resolved
        {
            let failed_node_id = recovery.failed_provider_node_id;
            if !failed_node_id.is_empty() {
                let now = chrono::Utc::now().to_rfc3339();
                for node in engine.timeline_nodes.iter_mut() {
                    if node.node_id == failed_node_id && node.status == TimelineNodeStatus::Active {
                        node.status = TimelineNodeStatus::Failed;
                        node.summary = Some(Self::lc_waiting_node_summary(&recovery.reason_code));
                        node.completed_at = Some(now.clone());
                    }
                }
                if engine.active_node_id.as_deref() == Some(failed_node_id.as_str()) {
                    engine.active_node_id = None;
                }
            }
            if matches!(
                engine.session.stage,
                WorkspaceStage::Running | WorkspaceStage::CrossReview | WorkspaceStage::Revision
            ) {
                engine.session.stage = WorkspaceStage::PrepareContext;
            }
        }
        Ok(engine)
    }

    /// A1b1 共享纯读核心（载入段）：durable artifact 版本覆盖与
    /// timeline/active/stage 派生。纯内存，无 I/O、无恢复写。
    fn derive_persistent_timeline(
        session: &mut WorkspaceSession,
        persisted_timeline_nodes: Vec<TimelineNode>,
        persisted_artifact_versions: &[ArtifactVersion],
    ) -> (Vec<TimelineNode>, Option<String>) {
        if !persisted_artifact_versions.is_empty() {
            session.artifact = persisted_artifact_versions
                .iter()
                .rev()
                .find(|version| version.is_current)
                .map(|version| version.payload.clone());
        }
        if persisted_timeline_nodes.is_empty() {
            initial_timeline(session)
        } else {
            let active_node_id = active_timeline_node_id(&persisted_timeline_nodes);
            if let Some(stage) = active_node_id
                .as_ref()
                .and_then(|node_id| {
                    persisted_timeline_nodes
                        .iter()
                        .find(|node| &node.node_id == node_id)
                })
                .map(|node| workspace_stage_from_ws_stage(&node.stage))
            {
                session.stage = stage;
            }
            (persisted_timeline_nodes, active_node_id)
        }
    }

    /// A1b1 共享纯读核心（收尾段）：retired-stage 迁移、最新 review verdict、
    /// pending author choice 与 plan repair snapshot 装配。只读，无恢复写。
    fn finalize_persistent_derivation(
        lifecycle_store: &LifecycleStore,
        mut session: WorkspaceSession,
        mut timeline_nodes: Vec<TimelineNode>,
        mut active_node_id: Option<String>,
        artifact_versions: Vec<ArtifactVersion>,
        plan_repair_transition_recovery_error: Option<String>,
    ) -> PersistentDerivation {
        // 存量 Story/Design 会话若停留在已退役的 HumanConfirm/ReviewDecision 阶段，恢复时迁移回 AuthorConfirm。
        session = recover_story_design_retired_stage_fallback(session);
        let latest_review_verdict = latest_review_verdict_from_node_details(
            lifecycle_store,
            &session.project_id,
            &session.issue_id,
            &session.session_id,
            &timeline_nodes,
        )
        .or_else(|| {
            latest_review_verdict_from_messages(&session.messages, &session.workspace_type)
        });
        let pending_author_choice =
            recover_pending_author_choice(&session, active_node_id.as_deref(), &timeline_nodes);
        let mut plan_repair_snapshot = load_plan_repair_snapshot_fail_closed(
            lifecycle_store,
            &session,
            &mut timeline_nodes,
            plan_repair_transition_recovery_error,
        );
        if let Some(snapshot) = plan_repair_snapshot.as_mut() {
            snapshot.timeline_nodes = timeline_nodes.clone();
            session.stage = match snapshot.stage {
                PlanRepairSessionStage::AwaitingConfirmation
                | PlanRepairSessionStage::Published
                | PlanRepairSessionStage::AmendmentConflict
                | PlanRepairSessionStage::AmendmentApplyFailed => WorkspaceStage::HumanConfirm,
                PlanRepairSessionStage::Completed | PlanRepairSessionStage::Failed => {
                    active_node_id = None;
                    WorkspaceStage::Completed
                }
                PlanRepairSessionStage::Triaging
                | PlanRepairSessionStage::AuthoringRevision
                | PlanRepairSessionStage::ValidatingContract
                | PlanRepairSessionStage::GeneratingProjections
                | PlanRepairSessionStage::PlanReview
                | PlanRepairSessionStage::ApplyingAmendment => WorkspaceStage::Running,
            };
        }
        PersistentDerivation {
            session,
            timeline_nodes,
            active_node_id,
            artifact_versions,
            latest_review_verdict,
            pending_author_choice,
            plan_repair_snapshot,
        }
    }

    /// A1b1：由共享纯读核心产物组装引擎——两条持久构造路径（真实恢复 /
    /// 纯投影）的唯一 Self 装配点，保证 session-state 组装同源。
    fn persistent_engine(
        checkpoint_store: Arc<CheckpointStore>,
        lifecycle_store: LifecycleStore,
        event_tx: mpsc::Sender<EngineEvent>,
        derivation: PersistentDerivation,
        outline_revision_recovery_error: Option<String>,
    ) -> Self {
        Self {
            checkpoint_store,
            lifecycle_store: Some(lifecycle_store),
            event_tx,
            session: derivation.session,
            cancel: CancellationToken::new(),
            timeline_nodes: derivation.timeline_nodes,
            active_node_id: derivation.active_node_id,
            artifact_versions: derivation.artifact_versions,
            latest_review_verdict: derivation.latest_review_verdict,
            pending_revision_context: None,
            pending_author_choice: derivation.pending_author_choice,
            active_run_id: None,
            stream_buffers: HashMap::new(),
            work_item_plan_author_retry_count: 0,
            work_item_plan_revision_retry_count: 0,
            work_item_draft_repair_states: HashMap::new(),
            outline_revision_recovery_error,
            outline_revision_crash_after: None,
            plan_repair_crash_after: None,
            plan_repair_snapshot: derivation.plan_repair_snapshot,
            #[cfg(test)]
            policy_route_before_persist: None,
            logical_provider_gateway: None,
            logical_launch_fingerprints: HashMap::new(),
            last_gate_close_compile_failure: None,
            confirmation_trust_context: None,
        }
    }

    /// 为已停止的 auto WorkItemPlan 创建新的 interactive 后继运行。
    /// 原运行只作为 durable 审计来源，不会被此操作改写；WS/UI 操作入口属于阶段 3。
    pub fn takeover_stopped_needs_human(
        &self,
    ) -> Result<crate::product::models::WorkspaceSessionRecord, String> {
        let store = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        store
            .takeover_stopped_needs_human(&self.session.session_id)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn outline_revision_recovery_error(&self) -> Option<&str> {
        self.outline_revision_recovery_error.as_deref()
    }

    /// 注入逻辑代码库 provider gateway(Task 11)。Web 接入 task 为逻辑代码库 issue 构造
    /// gateway 后调用此方法,使 planning author stream 经 gateway 启动;未调用时引擎
    /// 保留直接 `provider.start` 路径。
    pub fn with_logical_provider_gateway(
        mut self,
        gateway: Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
    ) -> Self {
        self.logical_provider_gateway = Some(gateway);
        self
    }

    pub fn session(&self) -> &WorkspaceSession {
        &self.session
    }

    /// 克隆既有 `logical_provider_gateway` 字段。非空即逻辑会话(Task 11 注入)。
    pub fn logical_provider_gateway(
        &self,
    ) -> Option<Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>> {
        self.logical_provider_gateway.clone()
    }

    /// T4.1/T4.3（REQ-REG-15/D2）：注入人工确认 trust 按需登记的冻结上下文
    /// （manager create 在 gateway 装配段调用；与 aggregate initialization
    /// 生产依赖共用同一 precondition 实例）。缺省 None 时 LC 会话（有
    /// gateway）确认 fail-closed ContextWaiting，非 LC 会话免登记。
    pub(crate) fn with_confirmation_trust_context(
        mut self,
        context: super::types::ConfirmationTrustContext,
    ) -> Self {
        self.confirmation_trust_context = Some(context);
        self
    }

    /// T4.1（REQ-REG-15）：确认 trust 冻结上下文的只读快照（manager 装配
    /// 断言/测试观察锚）。返回克隆的 precondition Arc 与冻结身份。
    pub fn confirmation_trust_context(&self) -> Option<super::types::ConfirmationTrustContext> {
        self.confirmation_trust_context.clone()
    }

    /// T4.1（REQ-REG-15 方案 A/D1/D2）：合法人工确认关门前 trust 按需准备。
    ///
    /// 接线边界（D1）：SC 在 `close_human_gate` Approve 臂静态资格预检通过
    /// 后、原带写 close CAS 之前；非 SC 在 `handle_confirm` 的 HumanConfirm
    /// 臂 `validate_confirm_aggregate_spec_gate` 之后、定稿/创建子会话之前。
    /// Running 臂、SC 反馈、Abandon 不进入本方法（不登记）。
    ///
    /// - 非 LC 会话（未注入 gateway）：免登记零调用直接放行。
    /// - LC 会话缺确认上下文/precondition：ContextWaiting fail-closed。
    /// - 漂移重验（零 home 调用）：issue 原始 LC 归属重解析相等；frozen
    ///   root 可 canonicalize；同 scope manifest root 与 frozen root
    ///   canonical 相等。任一不成立 → ContextWaiting（只返回已知身份，
    ///   不编造 provider/root/key、不给错误根 trust 指引、不降级到另一
    ///   LC/成员 checkout/原路径）。
    /// - 有效 provider 集：调用方给定 author + effective reviewer（SC=
    ///   durable expected；非 SC=mutex 内锁定 session 事实；disabled
    ///   reviewer 锁定期已清空、provisional 恒不采信），过滤非 trust
    ///   provider 并去重；为空即免登记零调用（Claude-only 等不变）。
    /// - 经 3.2 取消安全入口 `ensure_ready_cancellable`（Confirmation
    ///   场景）调用共用 registry：待锁可取消——caller 取消即停止，取消的
    ///   caller 不再推进原 CAS；临界区开始（home 锁已获取=提交边界）后
    ///   detached 安全收尾、不误删。engine 互斥（manager 持有的锁）覆盖
    ///   准备与后续 CAS 至结束。operation_id=`workspace_confirm:<session>`
    ///   仅用于关联审计，非确认幂等键。
    /// - 全部 Ready 才放行原 CAS/定稿；失败返回 typed HomeWaiting
    ///   （retry_action=「修复后重新确认」，门不变、零推进副作用）。
    pub(crate) async fn ensure_confirmation_trust_ready(
        &mut self,
        author: ProviderName,
        reviewer: Option<ProviderName>,
    ) -> Result<(), super::types::ConfirmationTrustWaiting> {
        // 同步预检段：完成全部漂移重验与 provider 过滤，产出 owned 数据。
        // 不持 `&self` 跨 await（WorkspaceEngine 含测试 hook 字段非 Sync，
        // 引擎借用不可进入 Send future）；确认互斥仍由 manager 持有的
        // engine 锁覆盖准备与后续 CAS 全程。
        let request = self.prepare_confirmation_trust_request(author, reviewer)?;
        let ConfirmationTrustEnsure::Ensure {
            precondition,
            session_id,
            project_id,
            operation_id,
            registry_lc_id,
            canonical_root,
            providers,
        } = request
        else {
            return Ok(());
        };
        use crate::product::logical_codebase::ProviderTrustPreparationResult;
        use crate::product::logical_codebase::provider_trust::ProviderTrustScenario;
        match precondition
            .ensure_ready_cancellable(
                project_id.clone(),
                operation_id,
                registry_lc_id.clone(),
                canonical_root,
                providers,
                ProviderTrustScenario::Confirmation,
            )
            .await
        {
            ProviderTrustPreparationResult::Ready { .. } => Ok(()),
            ProviderTrustPreparationResult::Waiting { waiting } => {
                Err(super::types::ConfirmationTrustWaiting::Home {
                    session_id,
                    project_id,
                    lc_id: registry_lc_id,
                    waiting,
                })
            }
        }
    }

    /// [`Self::ensure_confirmation_trust_ready`] 的同步预检段（零 home
    /// 调用）：非 LC 会话/无有效 trust provider → `Skip`；漂移重验全部
    /// 通过 → `Ensure`（owned 调用参数）。漂移任一不成立返回
    /// ContextWaiting（判据见上方法文档）。
    fn prepare_confirmation_trust_request(
        &self,
        author: ProviderName,
        reviewer: Option<ProviderName>,
    ) -> Result<ConfirmationTrustEnsure, super::types::ConfirmationTrustWaiting> {
        use super::types::{ConfirmationTrustContext, ConfirmationTrustWaiting};
        use crate::product::logical_codebase::{LogicalCodebaseStore, requires_workspace_trust};

        // 非 LC 会话：未注入 gateway → 免登记（零调用放行）。
        if self.logical_provider_gateway.is_none() {
            return Ok(ConfirmationTrustEnsure::Skip);
        }
        let session_id = self.session.session_id.clone();
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let ConfirmationTrustContext {
            precondition,
            issue_lc_id,
            frozen_root,
        } = match self.confirmation_trust_context.clone() {
            Some(context) => context,
            None => {
                return Err(ConfirmationTrustWaiting::Context {
                    session_id,
                    project_id,
                    issue_id,
                    issue_lc_id: None,
                    frozen_root: None,
                    reason_code: "confirmation_trust_context_unavailable",
                    message: "LC 会话缺少确认 trust 登记上下文（manager 未注入 \
                              precondition/冻结身份）；确认停等，未触碰 home，门保持开态。"
                        .to_string(),
                });
            }
        };
        let context_waiting =
            |reason_code: &'static str, message: String| ConfirmationTrustWaiting::Context {
                session_id: session_id.clone(),
                project_id: project_id.clone(),
                issue_id: issue_id.clone(),
                issue_lc_id: issue_lc_id.clone(),
                frozen_root: Some(frozen_root.clone()),
                reason_code,
                message,
            };
        // 引擎无 lifecycle store（非持久构造，仅测试面）时无法重验冻结身份。
        let Some(store) = self.lifecycle_store.as_ref() else {
            return Err(context_waiting(
                "confirmation_trust_context_unavailable",
                "引擎无 lifecycle store，无法重验确认 trust 冻结身份；确认停等，未触碰 home。"
                    .to_string(),
            ));
        };
        let app_paths = store.app_paths();
        // 漂移重验 1：issue 原始 LC 归属（与 manager create 同源解析）。
        let current_issue_lc = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
            &app_paths,
            &project_id,
            &issue_id,
        )
        .map_err(|error| {
            context_waiting(
                "confirmation_trust_context_unavailable",
                format!(
                    "重解析 issue 原始 LC 归属失败（issue store IO：{error}）；\
                         确认停等，未触碰 home。"
                ),
            )
        })?;
        if current_issue_lc != issue_lc_id {
            return Err(context_waiting(
                "confirmation_trust_scope_changed",
                format!(
                    "issue 原始 LC 归属已改变：create 期冻结 {issue_lc_id:?}，当前 \
                     {current_issue_lc:?}；确认停等，不降级到另一 LC/成员 checkout/原路径。"
                ),
            ));
        }
        // registry 调用点 LC 标签：显式 LC，或既有 legacy 别名约定（与
        // aggregate initialization coordinator 的 scope 推导同源；legacy
        // 别名与无 scope 的 durable 布局同路径，见 `lc_scope_root`）。
        let registry_lc_id = issue_lc_id.clone().unwrap_or_else(|| {
            crate::product::logical_codebase::legacy_logical_codebase_id(&project_id)
        });
        // 漂移重验 2：frozen root 仍可 canonicalize。
        let canonical_frozen_root = std::fs::canonicalize(&frozen_root).map_err(|error| {
            context_waiting(
                "confirmation_trust_root_unavailable",
                format!(
                    "冻结聚合根 {} 无法 canonicalize（{error}）；确认停等，未触碰 home。",
                    frozen_root.display()
                ),
            )
        })?;
        // 漂移重验 3：同 scope manifest root 与冻结 root 一致。manifest
        // 缺失/不可读时一致性无法重建，按漂移 fail-closed（不伪称一致）。
        let manifest = LogicalCodebaseStore::for_lc(app_paths.clone(), registry_lc_id.clone())
            .load_manifest(&project_id)
            .map_err(|error| {
                context_waiting(
                    "confirmation_trust_root_changed",
                    format!(
                        "同 scope LC manifest 读取失败（{error}），无法复验与冻结根一致；\
                         确认停等。"
                    ),
                )
            })?;
        let Some(manifest) = manifest else {
            return Err(context_waiting(
                "confirmation_trust_root_changed",
                "同 scope LC manifest 已缺失，无法复验与冻结根一致；确认停等。".to_string(),
            ));
        };
        let canonical_manifest_root = std::fs::canonicalize(&manifest.provider_context_root)
            .map_err(|error| {
                context_waiting(
                    "confirmation_trust_root_changed",
                    format!(
                        "manifest 聚合根 {} 无法 canonicalize（{error}），与冻结根一致性\
                         无法复验；确认停等。",
                        manifest.provider_context_root.display()
                    ),
                )
            })?;
        if canonical_manifest_root != canonical_frozen_root {
            return Err(context_waiting(
                "confirmation_trust_root_changed",
                format!(
                    "同 scope manifest 聚合根 {} 与 gateway 冻结根 {} 不一致；确认停等，\
                     不给错误根提供 trust 指引。",
                    canonical_manifest_root.display(),
                    canonical_frozen_root.display()
                ),
            ));
        }
        // 有效 provider 集过滤：disabled reviewer 已在 provider 锁定期清空为
        // None（调用方传入的即 effective）；provisional 恒不采信。
        let mut selected: Vec<ProviderName> = Vec::new();
        for provider in [author].into_iter().chain(reviewer) {
            if requires_workspace_trust(&provider) && !selected.contains(&provider) {
                selected.push(provider);
            }
        }
        if selected.is_empty() {
            return Ok(ConfirmationTrustEnsure::Skip);
        }
        let operation_id = format!("workspace_confirm:{session_id}");
        Ok(ConfirmationTrustEnsure::Ensure {
            precondition,
            session_id,
            project_id,
            operation_id,
            registry_lc_id,
            canonical_root: canonical_frozen_root,
            providers: selected,
        })
    }

    /// 逻辑会话的 planning launch 标识:(project_id, target worktree 锚)。
    ///
    /// 仅当注入 `logical_provider_gateway` 且 `session.repository_path` 存在时返回
    /// `Some`。第二元素是成员 checkout（target 锚）——Task 2.8 起 provider cwd
    /// 不再取该值，恒由 gateway 冻结的 canonical root（`authority_root()`）提供
    /// （见 gateway_start.rs）；本方法保留为「是否逻辑会话」谓词与 target 锚来源。
    pub fn logical_planning_launch(&self) -> Option<(String, std::path::PathBuf)> {
        self.logical_provider_gateway.as_ref()?;
        let cwd = self.session.repository_path.clone()?;
        Some((self.session.project_id.clone(), cwd))
    }

    /// C-1 集成修复：以 durable record 重建 session 时保留引擎外注入的内存态锚点。
    ///
    /// `repository_path` 由 manager/factory 在引擎构造时注入（durable record 不含
    /// 该字段，`from_record` 恒置 None）。引擎内持久化后的 session 整体替换
    /// （start_generation SC 重臂、provider start 预留、人工门修订保存）若不保留
    /// 该字段，`logical_planning_launch`（LC planning launch 与 C-1 admission 预检的
    /// 共同谓词输入）会在运行中途变为 None，真实 LC 会话被静默降级回 legacy 直启
    /// 路径、admission 预检被跳过。引擎内的 session 整体重建 MUST 走本方法，不得
    /// 直接 `self.session = WorkspaceSession::from_record(..)`。
    pub(crate) fn reload_session_from_record(
        &mut self,
        record: crate::product::models::WorkspaceSessionRecord,
    ) {
        let repository_path = self.session.repository_path.clone();
        self.session = WorkspaceSession::from_record(record);
        self.session.repository_path = repository_path;
    }

    pub fn pending_author_choice_request_message(&self) -> Option<WsOutMessage> {
        let pending = self.pending_author_choice.as_ref()?;
        Some(WsOutMessage::ChoiceRequest {
            id: pending.id.clone(),
            prompt: pending.prompt.clone(),
            options: pending
                .options
                .iter()
                .map(|option| ChoiceOption {
                    id: option.id.clone(),
                    label: option.label.clone(),
                    description: option.description.clone(),
                })
                .collect(),
            allow_multiple: false,
            allow_free_text: true,
            questions: vec![ChoiceQuestion {
                id: "default".to_string(),
                prompt: pending.prompt.clone(),
                options: pending
                    .options
                    .iter()
                    .map(|option| ChoiceOption {
                        id: option.id.clone(),
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect(),
                allow_multiple: false,
                allow_free_text: true,
            }],
            source: ChoiceRequestSource::TextFallback.as_str().to_string(),
        })
    }

    pub(crate) fn provider_resume_session_id(
        &self,
        role: ProviderConversationRole,
        provider: &ProviderName,
    ) -> Option<String> {
        provider_conversation_session_id(&self.session.provider_conversations, &role, provider)
            .or_else(|| {
                self.lifecycle_store.as_ref().and_then(|store| {
                    store
                        .get_workspace_session(&self.session.session_id)
                        .ok()
                        .and_then(|session| {
                            provider_conversation_session_id(
                                &session.provider_conversations,
                                &role,
                                provider,
                            )
                        })
                })
            })
            // r50(#3,claude r49 plan resume 现场):会话引用(内存+durable)
            // 皆缺失时,以 gateway spawn 时点审计的最新 Author
            // provider_start 兜底还原原生 resume——fresh 轮 spawn 即落盘,
            // 不受会话引用记录丢失影响;修订 turn 因此携带 --resume 续接
            // 原生会话(CLI resume 返回同 id),resume 格「原生恢复确认」
            // 可达。仅 Author 面(Reviewer 会话不兜底,行为不变)。
            .or_else(|| self.author_provider_start_audit_native_id(role, provider))
    }

    /// 审计兜底:按工作类型取 Author 面的 AdapterRole 审计标签
    ///(Story/Design/WorkItem=orchestrator;WorkItemPlan=work_item_splitter),
    /// 读 spawn 审计最新 provider_start 的原生 id。
    fn author_provider_start_audit_native_id(
        &self,
        role: ProviderConversationRole,
        provider: &ProviderName,
    ) -> Option<String> {
        if role != ProviderConversationRole::Author {
            return None;
        }
        let store = self.lifecycle_store.as_ref()?;
        let audit_role = match self.session.workspace_type {
            crate::product::models::WorkspaceType::WorkItemPlan => "work_item_splitter",
            crate::product::models::WorkspaceType::Story
            | crate::product::models::WorkspaceType::Design
            | crate::product::models::WorkspaceType::WorkItem => "orchestrator",
        };
        let provider_canonical = serde_json::to_value(provider)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default();
        store
            .latest_provider_start_native_id(
                &self.session.session_id,
                &provider_canonical,
                audit_role,
            )
            .ok()
            .flatten()
    }

    pub(crate) async fn record_provider_session(
        &mut self,
        role: ProviderConversationRole,
        provider: ProviderName,
        provider_session_id: Option<String>,
        node_id: Option<String>,
    ) {
        let Some(provider_session_id) = provider_session_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
        else {
            return;
        };
        let now = chrono::Utc::now().to_rfc3339();
        if let Some(existing) = self
            .session
            .provider_conversations
            .iter_mut()
            .find(|conversation| conversation.role == role && conversation.provider == provider)
        {
            existing.provider_session_id = provider_session_id;
            existing.updated_at = now;
            existing.last_node_id = node_id;
        } else {
            self.session
                .provider_conversations
                .push(ProviderConversationRef {
                    role,
                    provider,
                    provider_session_id,
                    updated_at: now,
                    last_node_id: node_id,
                });
        }
        if let Some(store) = &self.lifecycle_store {
            let _ = store.replace_workspace_provider_conversations(
                &self.session.session_id,
                self.session.provider_conversations.clone(),
            );
        }
    }

    /// Task 9c:StartNew(supersede)后清理被取代旧会话的 native 引用——显式
    /// fresh(现有 `WsInMessage::StartGeneration`→`start_generation` 面)只能
    /// 启动全新会话,旧 thread 引用不得再触发 resume 决策;匹配
    /// (role,provider,session id) 的引用移除并 durable 收口
    /// (`replace_workspace_provider_conversations`),无匹配时零写入。
    pub(crate) async fn clear_superseded_provider_session(
        &mut self,
        role: ProviderConversationRole,
        provider: &ProviderName,
        superseded_session_id: &str,
    ) {
        let before = self.session.provider_conversations.len();
        self.session.provider_conversations.retain(|conversation| {
            !(conversation.role == role
                && conversation.provider == *provider
                && conversation.provider_session_id == superseded_session_id)
        });
        if self.session.provider_conversations.len() != before
            && let Some(store) = &self.lifecycle_store
        {
            let _ = store.replace_workspace_provider_conversations(
                &self.session.session_id,
                self.session.provider_conversations.clone(),
            );
        }
    }

    pub fn current_stage(&self) -> WorkspaceStage {
        self.session.stage.clone()
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn start_new_run_token(&mut self) -> CancellationToken {
        self.cancel = CancellationToken::new();
        self.cancel.clone()
    }

    pub fn use_run_token(&mut self, cancel: CancellationToken) {
        self.cancel = cancel;
    }

    pub fn mark_active_run_started(&mut self, run_id: impl Into<String>) {
        self.active_run_id = Some(run_id.into());
    }

    pub fn mark_active_run_finished(&mut self, run_id: &str) {
        if self.active_run_id.as_deref() == Some(run_id) {
            self.active_run_id = None;
            if matches!(
                self.active_node_type(),
                Some(TimelineNodeType::WorkItemDraftRun | TimelineNodeType::WorkItemBatchRun)
            ) {
                self.work_item_draft_repair_states.clear();
            }
        }
    }

    pub fn active_run_id(&self) -> Option<&str> {
        self.active_run_id.as_deref()
    }

    pub fn active_timeline_node_id(&self) -> Option<String> {
        self.active_node_id.clone()
    }

    pub(crate) fn active_node_type(&self) -> Option<TimelineNodeType> {
        let active_node_id = self.active_node_id.as_deref()?;
        self.timeline_nodes
            .iter()
            .find(|node| node.node_id == active_node_id)
            .map(|node| node.node_type.clone())
    }

    pub async fn take_pending_author_choice_prompt(
        &mut self,
        id: &str,
        selected_option_ids: Vec<String>,
        free_text: Option<String>,
    ) -> Result<String, PendingAuthorChoiceError> {
        let Some(pending) = self.pending_author_choice.as_ref() else {
            return Err(PendingAuthorChoiceError::NotFound { id: id.to_string() });
        };
        if pending.id != id {
            return Err(PendingAuthorChoiceError::IdMismatch {
                expected: pending.id.clone(),
                actual: id.to_string(),
            });
        }

        let mut selected_labels = Vec::new();
        for selected_id in &selected_option_ids {
            let Some(option) = pending
                .options
                .iter()
                .find(|option| option.id == *selected_id)
            else {
                return Err(PendingAuthorChoiceError::OptionUnmatched {
                    id: selected_id.clone(),
                });
            };
            selected_labels.push(option.label.clone());
        }

        let free_text = free_text.and_then(|text| {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let pending = self
            .pending_author_choice
            .take()
            .expect("pending author choice present");
        if let Some(node_id) = pending.source_node_id.as_deref() {
            self.update_timeline_node(
                node_id,
                TimelineNodeStatus::Completed,
                Some("已收到用户选择".to_string()),
            )
            .await;
        }

        let mut prompt = String::new();
        prompt.push_str("用户回答了 author 的确认问题：\n");
        prompt.push_str(&format!("问题：{}\n", pending.prompt));
        if !selected_labels.is_empty() {
            prompt.push_str("选择：\n");
            for label in selected_labels {
                prompt.push_str(&format!("- {label}\n"));
            }
        }
        if let Some(free_text) = free_text {
            prompt.push_str(&format!("补充：{free_text}\n"));
        }
        prompt.push_str(
            "\n请基于该回答继续生成完整候选产物；如果仍有必须由用户确认的问题，请继续发起选择请求，不要进入 reviewer。",
        );
        // F-60 P0（因素三）：choice 应答续跑的输出契约升级为出口级覆盖——本函数
        // 只产问答内容（不再自行拼契约，避免契约文本进入 session 历史造成
        // marker 污染），由 build_streaming_input 的 DeltaOnly 出口无条件装配
        // 当前类型全条款（防线 2 语义保留，覆盖扩至全部 Markdown 类型）。
        Ok(prompt)
    }

    pub async fn append_context_note(&mut self, content: String) -> Result<TimelineNode, String> {
        let msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();
        self.session.messages.push(SessionMessage {
            id: msg_id,
            role: "user".to_string(),
            content: content.clone(),
            checkpoint_id: None,
            created_at: now,
        });
        if let Some(store) = &self.lifecycle_store {
            store
                .append_workspace_message(
                    &self.session.session_id,
                    "user".to_string(),
                    content.clone(),
                )
                .map_err(|error| format!("persist context note failed: {error}"))?;
        }
        Ok(self
            .append_completed_timeline_event(
                TimelineNodeType::ContextNote,
                WorkspaceStage::PrepareContext,
                "上下文补充".to_string(),
                Some(content),
                TimelineNodeStatus::Completed,
                false,
            )
            .await)
    }

    /// A2b（Task 2.6）：durable LC 恢复是否仍 active（Waiting/
    /// FreshReserved/FreshRunning）。Resolved 只保留证据不阻断；None 无恢复。
    pub(crate) fn lc_resume_recovery_active(&self) -> bool {
        self.session
            .lc_resume_recovery
            .as_ref()
            .is_some_and(|recovery| recovery.state != LcResumeRecoveryState::Resolved)
    }

    /// A2b（Task 2.6，设计 §4.3）：LC 等待收口的旧 provider 节点终态化
    /// summary——真实收口（`finalize_lc_waiting_provider_node`）与纯投影
    /// 派生同源，不引入第二套文案。
    pub(crate) fn lc_waiting_node_summary(reason_code: &str) -> String {
        format!("LC resume recovery waiting（{reason_code}）：旧 native 会话漂移，等待显式恢复操作")
    }

    /// A2b（Plan Task 2.6 接口字典，设计 §4.3）：LC superseded 等待的
    /// durable 收口第一步——**安全 record 先落盘**（跨 session
    /// JSON/timeline/turn 非强原子事务，先保存阻止旧 resume 与旁路 spawn
    /// 的 recovery record），经 A2a 的 `compare_and_save_lc_resume_state`
    /// 整记录 CAS（single-flight；后续新漂移替换 recovery 时传新
    /// recovery_id 的完整记录）。timeline 幂等收口与 snapshot 发布由
    /// `reconcile_lc_resume_waiting` 继续；被取代旧 native 引用的清理在
    /// 调用侧（`clear_superseded_provider_session`），多代 tombstone Vec
    /// 的写入/消费归 B1a 资格门（4.1），本入口不越权。
    pub(crate) async fn persist_lc_superseded_wait(
        &mut self,
        context: LcRecoveryRunContext,
        reason: &str,
    ) -> Result<LcResumeRecoveryRecord, String> {
        let store = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| {
                format!(
                    "load session for lc superseded wait failed ({}): {error}",
                    self.session.session_id
                )
            })?;
        // A2b 的 live mismatch 路径是 author revision resume 的 supersede
        // （`drive_revision_session`）；被取代的 resume id 即旧 Author 对话。
        let role = ProviderConversationRole::Author;
        let provider = self.session.author_provider.clone();
        let superseded_native_session_id = self
            .provider_resume_session_id(role.clone(), &provider)
            .filter(|id| !id.trim().is_empty());
        let previous_fingerprint_digest = self
            .logical_launch_fingerprints
            .get(&provider)
            .map(|fingerprint| fingerprint.digest.clone());
        let failed_provider_node_id = self
            .active_node_id
            .clone()
            .or_else(|| lc_recovery_context_original_node_id(&context))
            .unwrap_or_default();
        let now = chrono::Utc::now().to_rfc3339();
        let record = LcResumeRecoveryRecord {
            schema_version: 1,
            recovery_id: format!("lc_resume_recovery_{}", uuid::Uuid::new_v4().simple()),
            failed_provider_node_id,
            reason_code: reason.to_string(),
            role,
            provider,
            superseded_native_session_id,
            previous_fingerprint_digest,
            previous_session_status: expected.status.clone(),
            previous_stage: self.session.stage.as_str().to_string(),
            run_context: context,
            state: LcResumeRecoveryState::Waiting,
            operation_id: None,
            fresh_node_id: None,
            fresh_native_session_id: None,
            fresh_role_run_seq: None,
            fresh_operations: Vec::new(),
            detected_at: now.clone(),
            updated_at: now,
        };
        let saved = store
            .compare_and_save_lc_resume_state(&expected, record)
            .map_err(|error| {
                format!(
                    "persist lc superseded wait failed ({}): {error}",
                    self.session.session_id
                )
            })?;
        // 内存镜像同步：engine 是 runtime 单一权威内存态，守卫/收口立即
        // 可见；durable 非 LC 字段以 CAS 锁内 stored 为准（A2a 合同）。
        self.session.session_status = saved.status.clone();
        self.session.lc_resume_recovery = saved.lc_resume_recovery.clone();
        saved.lc_resume_recovery.ok_or_else(|| {
            "persist lc superseded wait returned record without recovery".to_string()
        })
    }

    /// A2b（Plan Task 2.6，设计 §4.3/§4.4）：LC 恢复等待的**真实收口写
    /// 入口**——幂等终态化旧 provider 节点（仅 Active 落 Failed，已终态
    /// 节点不覆写历史）、清该 run active、durable status 仅 Running→Open、
    /// run 阶段（Running/CrossReview/Revision）对外回 PrepareContext。唯一
    /// 调用方：manager 真实创建恢复入口 `recover_on_creation`（在任何自动
    /// recover_outline/human_turn 之前）与 live mismatch 收口臂；纯投影
    /// （`new_persistent_for_projection`）只派生不落盘。SC gate/phase/turn/
    /// 预算与既有人审 CAS 不触碰——等待不等价 Failed 业务 turn（§4.3）。
    pub(crate) async fn reconcile_lc_resume_waiting(&mut self) {
        let Some(recovery) = self.session.lc_resume_recovery.clone() else {
            return;
        };
        if recovery.state == LcResumeRecoveryState::Resolved {
            return;
        }
        let failed_node_id = recovery.failed_provider_node_id;
        if !failed_node_id.is_empty() {
            self.finalize_lc_waiting_provider_node(&failed_node_id, &recovery.reason_code)
                .await;
            if self.active_node_id.as_deref() == Some(failed_node_id.as_str()) {
                self.active_node_id = None;
            }
        }
        self.active_run_id = None;
        if self.session.session_status == WorkspaceSessionStatus::Running {
            self.session.session_status = WorkspaceSessionStatus::Open;
            if let Some(store) = &self.lifecycle_store
                && let Ok(record) = store.update_workspace_session_status(
                    &self.session.session_id,
                    WorkspaceSessionStatus::Open,
                )
            {
                self.session.session_status = record.status;
            }
        }
        if matches!(
            self.session.stage,
            WorkspaceStage::Running | WorkspaceStage::CrossReview | WorkspaceStage::Revision
        ) {
            self.transition_stage(WorkspaceStage::PrepareContext).await;
        }
    }

    pub async fn start_generation(
        &mut self,
        provider_config: ProviderConfigSnapshot,
        reviewer_enabled: bool,
    ) -> Result<(TimelineNode, WsOutMessage), String> {
        // A2b（Task 2.6，设计 §4.3/§4.5）：active LC 恢复（Waiting/
        // FreshReserved/FreshRunning）期间，普通参数 generation 一律后端
        // 真实入口拒收——显式 fresh 走 B1b 的 `resume_recovery` 入参（A/B
        // 未齐前无可启动半成品，allowed_actions 恒空兜底同一纪律）；resolved
        // 只保留证据，不阻后续正常会话。守卫先于终态检查与 SingleCandidate
        // 重臂（SC 原业务态不 rearm）。
        if self.lc_resume_recovery_active() {
            let recovery = self
                .session
                .lc_resume_recovery
                .as_ref()
                .expect("lc resume recovery presence checked above");
            return Err(format!(
                "LC_RESUME_RECOVERY_ACTIVE: 会话 {} 存在进行中的 LC 恢复（recovery_id={}, state={:?}）；普通 generation 已被拒收，等待显式恢复操作",
                self.session.session_id, recovery.recovery_id, recovery.state
            ));
        }
        // F-30（v34 复验 session_0008）：终态会话重跑走未定义路径——完整轮生成
        // 后挂零决策控件门、durable 停留旧状态、timeline 零新节点。入口按
        // session_status（真终态=Confirmed/Terminated）明确拒绝；Failed 不拦——
        // 看门狗/编译失败后的显式重跑（含 SingleCandidate store 重臂）是既有语义。
        // 守卫先于 SingleCandidate 重臂分支：Completed 单卡不再落 store 层的
        // 通用 Conflict，而是拿到带 session id 与重跑出口的可诊断错误。
        match self.session.session_status {
            WorkspaceSessionStatus::Confirmed => {
                return Err(format!(
                    "SESSION_ALREADY_CONFIRMED: 会话 {} 已确认（终态），不能重新开始生成；重新生成请走修订流程或新建会话",
                    self.session.session_id
                ));
            }
            WorkspaceSessionStatus::Terminated => {
                return Err(format!(
                    "SESSION_TERMINATED: 会话 {} 已终止（终态），不能重新开始生成；请新建会话",
                    self.session.session_id
                ));
            }
            _ => {}
        }
        if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate {
            let store = self
                .lifecycle_store
                .as_ref()
                .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
            let expected = store
                .get_workspace_session(&self.session.session_id)
                .map_err(|error| format!("load session for explicit reopen failed: {error}"))?;
            let saved = store
                .rearm_failed_single_candidate_for_start_generation(&expected)
                .map_err(|error| format!("reopen SingleCandidate session rejected: {error}"))?;
            self.reload_session_from_record(saved);
        }

        let mut locked_snapshot = provider_config;
        locked_snapshot.permission_modes.author = permission_mode_for_provider(
            &locked_snapshot.author,
            locked_snapshot.permission_modes.author.clone(),
        );
        if let Some(reviewer) = &locked_snapshot.reviewer {
            locked_snapshot.permission_modes.reviewer = permission_mode_for_provider(
                reviewer,
                locked_snapshot.permission_modes.reviewer.clone(),
            );
        }
        // Capture the raw snapshot reviewer BEFORE the disabled-review clear below:
        // provisional must retain the original selection even when reviewer_enabled=false
        // (design.md §3, provisional 恢复闭环); reviewer_provider/review_rounds are still cleared.
        let provisional_reviewer = locked_snapshot.reviewer.clone();
        if !reviewer_enabled {
            locked_snapshot.reviewer = None;
            locked_snapshot.review_rounds = 0;
        }

        self.session.author_provider = locked_snapshot.author.clone();
        self.session.reviewer_provider = locked_snapshot.reviewer.clone();
        self.session.review_rounds = locked_snapshot.review_rounds;
        self.session.permission_modes = locked_snapshot.permission_modes.clone();

        self.session.provisional_reviewer_provider = provisional_reviewer;
        self.session.reviewer_enabled_at_start = Some(reviewer_enabled);
        if let Some(store) = &self.lifecycle_store {
            store
                .update_workspace_session_provisional_reviewer(
                    &self.session.session_id,
                    self.session.provisional_reviewer_provider.clone(),
                    self.session.reviewer_enabled_at_start,
                )
                .map_err(|error| format!("persist provisional reviewer failed: {error}"))?;
        }

        if let Some(store) = &self.lifecycle_store {
            // C2 Task 5（REQ-CRO-05）：reviewer 缺失持久化空 effective——
            // MUST NOT 回填 author（重启重建后 from_record 保持空，不意外启动 reviewer）。
            store
                .update_workspace_session_providers(
                    &self.session.session_id,
                    locked_snapshot.author.clone(),
                    locked_snapshot.reviewer.clone(),
                )
                .map_err(|error| format!("persist provider lock failed: {error}"))?;
            store
                .update_workspace_session_permission_modes(
                    &self.session.session_id,
                    locked_snapshot.permission_modes.clone(),
                )
                .map_err(|error| format!("persist permission mode lock failed: {error}"))?;
            if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate {
                store
                    .update_workspace_session_status(
                        &self.session.session_id,
                        WorkspaceSessionStatus::Running,
                    )
                    .map_err(|error| format!("persist workspace status failed: {error}"))?;
            }
        }

        self.complete_active_node(Some("上下文已确认".to_string()))
            .await;
        let node = self
            .append_completed_timeline_event(
                TimelineNodeType::StartGeneration,
                WorkspaceStage::PrepareContext,
                "开始生成".to_string(),
                None,
                TimelineNodeStatus::Completed,
                true,
            )
            .await;
        self.transition_stage(WorkspaceStage::Running).await;

        let locked = WsOutMessage::ProviderLocked {
            snapshot: locked_snapshot,
            locked_at: chrono::Utc::now().to_rfc3339(),
        };
        Ok((node, locked))
    }

    /// 仅供进程重启后的 stale run 恢复链记录真实 provider 死亡终态。
    /// REQ-WCR-03：workspace WebSocket 连接关闭路径禁止调用；不得传入连接级事实。
    /// `connection_id` 仅在恢复链中标记 synthetic stale connection；连接关闭不再把它
    /// 写入 timeline marker detail。
    pub async fn append_aborted_by_disconnect(
        &mut self,
        last_active_run_id: String,
        connection_id: String,
    ) -> Result<TimelineNode, String> {
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(
                &node_id,
                TimelineNodeStatus::Failed,
                Some("连接断开，运行已中止".to_string()),
            )
            .await;
        }
        self.active_run_id = None;
        Ok(self
            .append_completed_timeline_event(
                TimelineNodeType::AbortedByDisconnect,
                WorkspaceStage::PrepareContext,
                "运行因断开中止".to_string(),
                Some(format!(
                    "last_active_run_id: {last_active_run_id}; connection_id: {connection_id}"
                )),
                TimelineNodeStatus::Failed,
                true,
            )
            .await)
    }

    /// 仅供进程重启后的 stale run 恢复链整流 session stage。
    /// REQ-WCR-03：workspace WebSocket 连接关闭路径禁止调用。
    pub async fn transition_to_prepare_context_after_disconnect(&mut self) {
        self.active_run_id = None;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Open,
            );
        }
        self.transition_stage(WorkspaceStage::PrepareContext).await;
    }

    pub async fn recover_stale_active_run_after_disconnect(&mut self) {
        if !matches!(
            self.session.stage,
            WorkspaceStage::Running | WorkspaceStage::CrossReview | WorkspaceStage::Revision
        ) {
            return;
        }

        let already_recorded = self
            .timeline_nodes
            .last()
            .is_some_and(|node| node.node_type == TimelineNodeType::AbortedByDisconnect);
        if !already_recorded {
            let run_id = self
                .active_run_id
                .clone()
                .unwrap_or_else(|| "stale-connection".to_string());
            let _ = self
                .append_aborted_by_disconnect(run_id, "stale-connection".to_string())
                .await;
        }
        self.transition_to_prepare_context_after_disconnect().await;
    }

    /// F-31（回归补偿）：story/design 的 AuthorConfirm 门确认面是 HTTP confirm 端点
    /// （handler 层直写 durable，WS confirm 帧被 stage 矩阵拒收）。C3-T5 删除
    /// `handle_author_decision` 时连带删掉了原有的「确认后触发 review」（原实现按
    /// `reviewer_enabled_at_start` 命中即 complete node + start_review），F-20 只补偿了
    /// 确认通道，未补偿 review 触发——review 启用的 story 会话确认后直接定稿，
    /// ReviewerRun 节点从不创建。
    ///
    /// 该端点据此在落 Confirmed 前询问引擎处置意见：review 启用且本轮产物尚未评审时
    /// 接管本轮——完成 AuthorConfirm 节点、进入 CrossReview 并申请 `ReviewOnly`
    /// provider run（Fake reviewer 的 Skipped 快速路径由 `start_review` 内部承接，
    /// 落到 HumanConfirm）。
    async fn begin_review_after_author_confirm(&mut self) {
        self.complete_active_node(Some("已确认，进入 Review".to_string()))
            .await;
        self.start_review().await;
        if self.session.stage == WorkspaceStage::CrossReview {
            // 评审已在途：durable 状态与 workspace_status_for_stage(CrossReview)=Running
            // 同口径，避免重连/投影读到「待人工确认 + 评审中」的矛盾组合。
            if let Some(store) = &self.lifecycle_store
                && let Ok(record) = store.update_workspace_session_status(
                    &self.session.session_id,
                    WorkspaceSessionStatus::Running,
                )
            {
                self.session.session_status = record.status;
            }
            self.request_provider_run(ProviderRunKind::ReviewOnly).await;
        }
    }

    /// 会话本轮是否启用评审（F-31 纠正轮：由强制接管判定拆出）。
    ///
    /// 按 `handle_author_decision` 被删实现的原始分流判定：`reviewer_enabled_at_start`
    /// 为准——durable 记录的 `review_rounds` 恒 ≥1 且 `reviewer_provider` 恒 Some（创建
    /// 配置校验 1..=5 + 默认 reviewer），二者只反映「选过哪个 reviewer」，不反映本次生成
    /// 是否启用评审；重连后 from_record 恒 Some，只按有效态判定会误起评审。仅旧记录
    /// （None）退回有效态判定（与本文件 `ensure_reviewer_available_for_review_request`
    /// 同源的失真说明）。
    fn review_enabled_for_session(&self) -> bool {
        match self.session.reviewer_enabled_at_start {
            Some(enabled) => enabled,
            None => self.session.review_rounds > 0 && self.session.reviewer_provider.is_some(),
        }
    }

    /// 本轮产物是否尚未评审：「本轮尚未跑过」的持久化判据——评审完成恒
    /// mark_latest_artifact_reviewed，而新一轮 author 产出（含 review 报告后的反馈修订）
    /// 的 version reviewed_by 为空。无当前 version（存量异常态）时不视作待评审，
    /// 不凭空起评审。
    fn latest_artifact_unreviewed(&self) -> bool {
        self.artifact_versions
            .iter()
            .rev()
            .find(|version| version.is_current)
            .is_some_and(|version| version.reviewed_by.is_none())
    }

    /// HTTP confirm 端点的引擎裁决面（F-31 fix round k3 P1/P2；v36 纠正轮参数化）。
    ///
    /// 端点不再自行判定「能否定稿」：引擎是 `stage` 的唯一权威，端点只按裁决做 durable 写
    /// 与广播。story/design 的分流（`with_review` 缺省 false）：
    ///
    /// - 门态（`AuthorConfirm` / `HumanConfirm` / `PrepareContext`）+ `with_review=false` →
    ///   交端点定稿——**是否送审由用户选择**，review 启用不再强制接管（v36 过度实现纠正）；
    /// - `AuthorConfirm` + `with_review=true` + review 启用且本轮未评审 → 接管本轮进
    ///   CrossReview（[`Self::begin_review_after_author_confirm`]）；本轮已评审（评审回门后
    ///   的二次确认）或处 `HumanConfirm`（Fake 快速路径）/`PrepareContext`（F-25b 锚：生成
    ///   前兼容确认面，无评审轮可言）→ 评审诉求已满足/不适用，回落定稿；
    /// - 门态 + `with_review=true` + review 未启用 → [`HttpConfirmDisposition::ReviewUnavailable`]：
    ///   如实 4xx 拒绝，不得静默按定稿（否则用户以为已送审而实际跳过评审）；
    /// - 在途 stage（`CrossReview`/`Running`/`Revision`/`ReviewDecision`）→ **拒收**（与送审
    ///   意愿无关，在途守卫优先）：端点此前对非 `AuthorConfirm` 一律落到 store-only 定稿，
    ///   评审未回门就置 Confirmed，迟到 review 报告再经 `route_review_report_to_author_confirm`
    ///   把已定稿会话拖回 AuthorConfirm（Running→Confirmed→WaitingForHuman，违反 F-30 终态口径）；
    /// - `Completed`：已 Confirmed 幂等返回，其余终态（terminated/failed/blocked）拒收。
    pub(crate) async fn http_confirm_disposition(
        &mut self,
        with_review: bool,
    ) -> HttpConfirmDisposition {
        if self.session.workspace_type == WorkspaceType::WorkItemPlan {
            return self
                .work_item_plan_http_confirm_disposition(with_review)
                .await;
        }
        if !matches!(
            self.session.workspace_type,
            WorkspaceType::Story | WorkspaceType::Design
        ) {
            return HttpConfirmDisposition::NotHandled;
        }
        let stage = self.session.stage.clone();
        match stage {
            WorkspaceStage::AuthorConfirm
            | WorkspaceStage::HumanConfirm
            | WorkspaceStage::PrepareContext => {
                if with_review && !self.review_enabled_for_session() {
                    return HttpConfirmDisposition::ReviewUnavailable;
                }
                if with_review
                    && stage == WorkspaceStage::AuthorConfirm
                    && self.latest_artifact_unreviewed()
                {
                    self.begin_review_after_author_confirm().await;
                    return HttpConfirmDisposition::ReviewStarted;
                }
                HttpConfirmDisposition::Finalize
            }
            WorkspaceStage::Completed => {
                if self.session.session_status == WorkspaceSessionStatus::Confirmed {
                    HttpConfirmDisposition::AlreadyConfirmed
                } else {
                    HttpConfirmDisposition::Rejected {
                        stage: WorkspaceStage::Completed.as_str(),
                    }
                }
            }
            other => HttpConfirmDisposition::Rejected {
                stage: other.as_str(),
            },
        }
    }

    /// WorkItemPlan 的 HTTP confirm 裁决（change plan-compile-gate-visibility Task 3）。
    ///
    /// 端点对 WorkItemPlan 此前一律 `NotHandled` → store-only 兜底只写 status=Confirmed，
    /// stage 停在 `AuthorConfirm`（批次确认门）且不建子 WorkItem 会话：既不能 advance
    /// （矩阵仅放行 SC + `Completed` 的 advance）也没建子会话，是用户可见的半落状态。
    ///
    /// - 批次确认门（`AuthorConfirm` + active `WorkItemBatchConfirm`）→ 执行既有引擎确认面
    ///   [`Self::confirm_work_item_plan_and_complete`]，本裁决内落 Confirmed + 子 WorkItem
    ///   会话 + stage→Completed；`with_review=true` 先行拒收（WorkItemPlan 无送审语义，
    ///   镜像 F-31 ReviewUnavailable 模式 → 端点 422）；
    /// - `Completed` + Confirmed → 既有终态守卫幂等返回（重复 confirm 零副作用）；
    /// - 其余形态（如普通 `HumanConfirm` 对话门）维持 `NotHandled`：端点既有通路语义不变。
    async fn work_item_plan_http_confirm_disposition(
        &mut self,
        with_review: bool,
    ) -> HttpConfirmDisposition {
        let batch_confirm_gate = self.session.stage == WorkspaceStage::AuthorConfirm
            && self.active_node_type() == Some(TimelineNodeType::WorkItemBatchConfirm);
        if batch_confirm_gate {
            if with_review {
                return HttpConfirmDisposition::ReviewUnavailable;
            }
            return match self.confirm_work_item_plan_and_complete().await {
                Ok(_) => HttpConfirmDisposition::WorkItemPlanConfirmed,
                Err(message) => {
                    tracing::warn!(%message, "work item plan batch confirm failed");
                    HttpConfirmDisposition::WorkItemPlanRejected { message }
                }
            };
        }
        if self.session.stage == WorkspaceStage::Completed {
            return if self.session.session_status == WorkspaceSessionStatus::Confirmed {
                HttpConfirmDisposition::AlreadyConfirmed
            } else {
                HttpConfirmDisposition::Rejected {
                    stage: WorkspaceStage::Completed.as_str(),
                }
            };
        }
        HttpConfirmDisposition::NotHandled
    }

    /// v38 复验 #2/#3（恢复 C3 前原意，spec-design-dialog-revision T3 `Revise`
    /// 语义重承载）：story/design 在 AuthorConfirm 门上提交修订意见——完成门
    /// 节点、记录反馈、进入 Revision 阶段并创建修订节点；修订 run 由 handler 层
    /// 发起（`ProviderRunKind::Revision` → `drive_revision_session`），完成后经
    /// provider_drive 完成路径回 AuthorConfirm 门（可再评审/确认定稿）。
    ///
    /// - I-1（T5）：post-review 新反馈提交时清空 review verdict，使
    ///   `is_author_feedback_revision` 分流谓词成立（pending 存在且无 verdict），
    ///   修订 prompt 走 author 反馈修订专用增量 prompt 而非 reviewer 返修 prompt；
    /// - T7 fix1（Finding-A）：反馈全文落盘修订节点 detail（NodeDetail.
    ///   revision_feedback）——断线重连后 new_persistent 重建 engine 丢失内存态
    ///   pending_revision_context（不在 WorkspaceSessionRecord），retry 臂从该
    ///   字段重建才能走 author 反馈 prompt 分支。
    pub(crate) async fn request_story_design_revision(
        &mut self,
        feedback: Option<String>,
    ) -> Result<(), String> {
        if self.session.stage != WorkspaceStage::AuthorConfirm {
            return Err(
                "request_revision is only available during author_confirm stage".to_string(),
            );
        }
        if !matches!(
            self.session.workspace_type,
            WorkspaceType::Story | WorkspaceType::Design
        ) {
            return Err(
                "request_revision for non-work-item-plan workspaces is only supported by story/design"
                    .to_string(),
            );
        }
        let Some(trimmed) = feedback
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            return Err("revise feedback must not be empty".to_string());
        };
        let trimmed = trimmed.to_string();
        self.complete_active_node(Some("用户提交反馈，进入修订".to_string()))
            .await;
        self.pending_revision_context = Some(trimmed.clone());
        self.latest_review_verdict = None;
        self.transition_stage(WorkspaceStage::Revision).await;
        let revision_node_id = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::Revision,
                agent: Some(self.session.author_provider.clone()),
                stage: WorkspaceStage::Revision,
                round: None,
                title: "反馈修订".to_string(),
                summary: Some(trimmed.clone()),
                status: TimelineNodeStatus::Active,
            })
            .await;
        let _ = self
            .update_node_detail(&revision_node_id, |detail| {
                detail.revision_feedback = Some(trimmed.clone());
            })
            .await;
        Ok(())
    }
}

/// A2b（Task 2.6）：run context 各 tag 的公共 business 上下文原始节点
/// （fallback 供 record.failed_provider_node_id 兜底；主路径取 active 节点）。
fn lc_recovery_context_original_node_id(context: &LcRecoveryRunContext) -> Option<String> {
    match context {
        LcRecoveryRunContext::Author { business, .. }
        | LcRecoveryRunContext::AuthorChoiceFollowup { business, .. }
        | LcRecoveryRunContext::AuthorFeedback { business, .. }
        | LcRecoveryRunContext::ReviewerRevision { business, .. }
        | LcRecoveryRunContext::LegacyOutlineAuthor { business, .. }
        | LcRecoveryRunContext::LegacyOutlineRevision { business, .. }
        | LcRecoveryRunContext::LegacyOutlineRebuild { business, .. }
        | LcRecoveryRunContext::LegacyDraft { business, .. }
        | LcRecoveryRunContext::LegacyBatch { business, .. }
        | LcRecoveryRunContext::LegacyPlanRevision { business, .. }
        | LcRecoveryRunContext::ScAuthor { business, .. }
        | LcRecoveryRunContext::ScManualRevision { business, .. } => {
            business.original_node_id.clone()
        }
    }
}
include!("lifecycle/routing_reference.inc.rs");
