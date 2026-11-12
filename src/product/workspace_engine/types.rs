use crate::product::logical_codebase::ResolvedPlanningContext;
use crate::product::models::WorkspaceRolePermissionModes;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceStage {
    PrepareContext,
    Running,
    AuthorConfirm,
    CrossReview,
    ReviewDecision,
    Revision,
    HumanConfirm,
    Completed,
}

impl WorkspaceStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PrepareContext => "prepare_context",
            Self::Running => "running",
            Self::AuthorConfirm => "author_confirm",
            Self::CrossReview => "cross_review",
            Self::ReviewDecision => "review_decision",
            Self::Revision => "revision",
            Self::HumanConfirm => "human_confirm",
            Self::Completed => "completed",
        }
    }

    pub fn from_stage_name(s: &str) -> Option<Self> {
        match s {
            "prepare_context" => Some(Self::PrepareContext),
            "running" => Some(Self::Running),
            "author_confirm" => Some(Self::AuthorConfirm),
            "cross_review" => Some(Self::CrossReview),
            "review_decision" => Some(Self::ReviewDecision),
            "revision" => Some(Self::Revision),
            "human_confirm" => Some(Self::HumanConfirm),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedWorkspaceAmendmentTarget {
    pub entity_id: String,
    pub workspace_type: WorkspaceType,
    pub relation: crate::product::models::WorkspaceSessionRelation,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LinkedWorkspaceSessionSnapshot {
    pub link: crate::product::models::WorkspaceSessionLink,
    pub entity_id: String,
    pub workspace_type: WorkspaceType,
    pub artifact_version_id: Option<u32>,
    pub timeline_nodes: Vec<TimelineNode>,
    pub selected_timeline_node_id: Option<String>,
    pub human_confirm_state: WorkspaceSessionStatus,
}

#[derive(Debug, Clone)]
pub struct SessionMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub checkpoint_id: Option<String>,
    pub created_at: String,
}

pub struct WorkspaceSession {
    pub session_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: WorkspaceType,
    pub stage: WorkspaceStage,
    pub messages: Vec<SessionMessage>,
    pub artifact: Option<ArtifactPayload>,
    pub author_provider: ProviderName,
    pub reviewer_provider: Option<ProviderName>,
    pub review_rounds: u32,
    pub permission_modes: WorkspaceRolePermissionModes,
    pub provisional_reviewer_provider: Option<ProviderName>,
    pub reviewer_enabled_at_start: Option<bool>,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    pub session_status: WorkspaceSessionStatus,
    pub flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind,
    pub run_policy: crate::product::work_item_plan_policy::RunPolicy,
    pub run_history: crate::product::work_item_plan_policy::RunHistory,
    pub review_invocation_scope:
        Option<crate::product::work_item_plan_policy::ReviewInvocationScope>,
    pub human_gate_snapshot: Option<crate::product::work_item_plan_policy::HumanGateSnapshot>,
    pub repair_reservation: Option<crate::product::work_item_plan_policy::RepairReservation>,
    pub policy_diagnostics: Vec<crate::product::work_item_plan_policy::PolicyDiagnostic>,
    pub provider_start_ledger: Vec<crate::product::work_item_plan_policy::ProviderStartLedgerEntry>,
    pub single_candidate_phase: Option<crate::product::models::SingleCandidatePhase>,
    pub work_item_plan_source_revision_ref: Option<String>,
    pub plan_candidate_ir_ref: Option<String>,
    pub mechanical_report_ref: Option<String>,
    pub publication_provenance_ref: Option<String>,
    pub approval_attempt_id: Option<String>,
    pub approved_at: Option<String>,
    pub compile_reservation: Option<crate::product::models::SingleCandidateCompileReservation>,
    pub provider_conversations: Vec<ProviderConversationRef>,
    pub logical_launch_fingerprints: Vec<crate::product::models::LogicalLaunchFingerprintRecord>,
    pub superseded_native_sessions: Vec<crate::product::models::SupersededNativeSessionRecord>,
    pub lc_resume_recovery: Option<crate::product::models::LcResumeRecoveryRecord>,
    /// SG7a（plan v1.1 §0.6.1）：结构治理聚合状态的 runtime 只读镜像——
    /// from_record 显式逐字段移入，成功 CAS 经
    /// `sync_structural_governance_from_record` 同步；accepted/actual
    /// 单源仍在 HumanGateSnapshot 与 provider_start_ledger。
    pub structural_governance_state: Option<crate::product::models::StructuralGovernanceState>,
    /// REQ-ENV-13（设计 A 条目 17）：每会话唯一评审/author 运行身份的
    /// runtime 镜像（from_record 逐字段透传；持久源为
    /// WorkspaceSessionRecord.review_identity_id）。
    pub review_identity_id: String,
    pub repository_path: Option<PathBuf>,
}

impl WorkspaceSession {
    pub fn from_record(record: WorkspaceSessionRecord) -> Self {
        let artifact = latest_artifact_from_messages(&record.messages, &record.workspace_type);
        Self {
            session_id: record.id,
            project_id: record.project_id,
            issue_id: record.issue_id,
            entity_id: record.entity_id,
            workspace_type: record.workspace_type,
            stage: workspace_stage_for_status(&record.status),
            messages: record
                .messages
                .into_iter()
                .enumerate()
                .map(|(idx, message)| SessionMessage {
                    id: format!("msg_{:03}", idx + 1),
                    role: message.role,
                    content: message.content,
                    checkpoint_id: None,
                    created_at: message.created_at,
                })
                .collect(),
            artifact,
            author_provider: record.author_provider,
            // C2 Task 5（REQ-CRO-05）：reviewer 三值透传——不再无条件 Some()，
            // 缺失记录恢复后保持空 effective。
            reviewer_provider: record.reviewer_provider,
            review_rounds: record.review_rounds,
            permission_modes: record.permission_modes.clone(),
            provisional_reviewer_provider: record.provisional_reviewer_provider,
            reviewer_enabled_at_start: record.reviewer_enabled_at_start,
            superpowers_enabled: record.superpowers_enabled,
            openspec_enabled: record.openspec_enabled,
            session_status: record.status,
            flow_kind: record.flow_kind,
            run_policy: record.run_policy,
            run_history: record.run_history,
            review_invocation_scope: record.review_invocation_scope,
            human_gate_snapshot: record.human_gate_snapshot,
            repair_reservation: record.repair_reservation,
            policy_diagnostics: record.policy_diagnostics,
            provider_start_ledger: record.provider_start_ledger,
            single_candidate_phase: record.single_candidate_phase,
            work_item_plan_source_revision_ref: record.work_item_plan_source_revision_ref,
            plan_candidate_ir_ref: record.plan_candidate_ir_ref,
            mechanical_report_ref: record.mechanical_report_ref,
            publication_provenance_ref: record.publication_provenance_ref,
            approval_attempt_id: record.approval_attempt_id,
            approved_at: record.approved_at,
            compile_reservation: record.compile_reservation,
            provider_conversations: record.provider_conversations,
            logical_launch_fingerprints: record.logical_launch_fingerprints,
            superseded_native_sessions: record.superseded_native_sessions,
            lc_resume_recovery: record.lc_resume_recovery,
            structural_governance_state: record.structural_governance_state,
            review_identity_id: record.review_identity_id,
            repository_path: None,
        }
    }

    /// SG7a（plan v1.1 §0.6.1 Interfaces 定案）：成功 CAS 后的 runtime
    /// 镜像同步——先核 session/project/issue/entity/workspace_type 同
    /// scope（错 scope `IdentityMismatch`，镜像保持不变），再复制结构
    /// 聚合状态、同次 CAS 的 source/IR/report 三 refs 及原
    /// human_gate_snapshot/provider_start_ledger（accepted/实际 start
    /// 唯一权威）；不改 stage/active_node/repository_path/anchor/trust，
    /// 不重算 ledger、不新增 accepted 副本。CAS 失败（调用方拿不到新
    /// record）不进本方法，镜像自然保持。
    pub fn sync_structural_governance_from_record(
        &mut self,
        record: &WorkspaceSessionRecord,
    ) -> Result<(), crate::product::json_store::ProductStoreError> {
        if self.session_id != record.id
            || self.project_id != record.project_id
            || self.issue_id != record.issue_id
            || self.entity_id != record.entity_id
            || self.workspace_type != record.workspace_type
        {
            return Err(
                crate::product::json_store::ProductStoreError::IdentityMismatch {
                    kind: "workspace_session",
                    id: record.id.clone(),
                },
            );
        }
        self.structural_governance_state = record.structural_governance_state.clone();
        self.work_item_plan_source_revision_ref = record.work_item_plan_source_revision_ref.clone();
        self.plan_candidate_ir_ref = record.plan_candidate_ir_ref.clone();
        self.mechanical_report_ref = record.mechanical_report_ref.clone();
        self.human_gate_snapshot = record.human_gate_snapshot.clone();
        self.provider_start_ledger = record.provider_start_ledger.clone();
        Ok(())
    }

    pub(crate) fn replace_messages_from_records(&mut self, messages: Vec<WorkspaceMessageRecord>) {
        self.messages = messages
            .into_iter()
            .enumerate()
            .map(|(idx, message)| SessionMessage {
                id: format!("msg_{:03}", idx + 1),
                role: message.role,
                content: message.content,
                checkpoint_id: None,
                created_at: message.created_at,
            })
            .collect();
    }

    pub fn restore_checkpoint_ids(
        &mut self,
        checkpoints: &[crate::product::checkpoint_store::Checkpoint],
    ) {
        for checkpoint in checkpoints {
            let Some(message_index) = checkpoint.message_index.checked_sub(1) else {
                continue;
            };
            if let Some(message) = self.messages.get_mut(message_index as usize)
                && message.role != "user"
            {
                message.checkpoint_id = Some(checkpoint.id.clone());
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactUpdateEvent {
    pub version: u32,
    pub payload: ArtifactPayload,
}

#[derive(Clone, Debug)]
pub enum ProviderRunKind {
    Author {
        content: String,
    },
    AuthorChoiceFollowup {
        content: String,
    },
    Revision,
    ReviewOnly,
    /// Legacy WorkItemPlan 的 outline JSON author。该 variant 已固化 durable flow，
    /// provider runner 不得再读取 rollout/feature flag 重新判定。
    WorkItemPlanLegacyAuthor,
    /// SingleCandidate 的 markdown source author。该 variant 已固化 durable flow，
    /// provider runner 只走 markdown compiler/source-store 链路。
    WorkItemPlanSingleCandidateAuthor,
    WorkItemPlanOutlineRevision {
        feedback: Option<String>,
    },
    /// StaleContext 重建：携带 rebuilt 的规划上下文，启动**全新** outline run（新建节点、
    /// 使用 rebuilt cwd/inventory/policy，不沿用中断会话的 OutlineRun 节点）。
    WorkItemPlanOutlineRebuild {
        rebuilt: Box<ResolvedPlanningContext>,
    },
    WorkItemPlanDraft {
        feedback: Option<String>,
    },
    WorkItemPlanBatch,
    WorkItemPlanRevision {
        feedback: Option<String>,
    },
    /// SC interactive human-gate revision. The turn id is the durable logical
    /// key; provider attempts are tracked separately from WS events.
    HumanGateScManualRevision {
        turn_id: String,
        prompt: String,
    },
}

impl ProviderRunKind {
    #[allow(dead_code)]
    pub(crate) fn human_gate_turn_id(&self) -> Option<&str> {
        match self {
            Self::HumanGateScManualRevision { turn_id, .. } => Some(turn_id),
            _ => None,
        }
    }

    pub(crate) fn work_item_plan_author_for_durable_flow(
        flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind,
    ) -> Self {
        match flow_kind {
            crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy => {
                Self::WorkItemPlanLegacyAuthor
            }
            crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate => {
                Self::WorkItemPlanSingleCandidateAuthor
            }
        }
    }

    #[cfg(test)]
    pub(crate) const fn is_legacy_work_item_plan_author(&self) -> bool {
        matches!(self, Self::WorkItemPlanLegacyAuthor)
    }

    pub(crate) const fn is_single_candidate_work_item_plan_author(&self) -> bool {
        matches!(self, Self::WorkItemPlanSingleCandidateAuthor)
    }
}

pub enum EngineEvent {
    StreamChunk {
        role: String,
        content: String,
        node_id: Option<String>,
    },
    MessageComplete {
        message_id: String,
        checkpoint_id: String,
        node_id: Option<String>,
    },
    StageChange {
        stage: String,
    },
    ArtifactUpdate {
        version: u32,
        payload: ArtifactPayload,
    },
    ArtifactBatchUpdate {
        updates: Vec<ArtifactUpdateEvent>,
    },
    PermissionRequest {
        id: String,
        tool_name: String,
        description: String,
        risk_level: RiskLevel,
    },
    ChoiceRequest {
        id: String,
        prompt: String,
        options: Vec<ChoiceOptionData>,
        allow_multiple: bool,
        allow_free_text: bool,
        questions: Vec<ChoiceQuestionData>,
        source: ChoiceRequestSource,
    },
    ProviderStatus {
        status: ProviderStatus,
    },
    /// F-27R2：choice 挂起集移除（应答命中挂起 id，进程级登记簿已同步
    /// 摘除）。Web runtime 以全量 session_state（含 pending_choice_requests
    /// 投影）同步所有已连接页面，前端对账据此收敛已答卡。此事件不经
    /// WebSocket 对客户端序列化；仅由 socket runtime 消费。
    ChoicePendingChanged,
    /// 引擎完成持久化路由后请求 Web runtime 启动新的 provider run。
    /// `node_id` 让 runtime 可以在不等待 engine mutex 的情况下排空同一节点的重复请求，
    /// 同时保留新节点请求替换旧 run 的语义。
    /// 此事件不经 WebSocket 对客户端序列化；仅由 socket runtime 消费。
    ProviderRunRequested {
        kind: ProviderRunKind,
        node_id: Option<String>,
    },
    ExecutionEvent {
        event: ProviderExecutionEvent,
        node_id: Option<String>,
        agent: Option<ProviderName>,
    },
    TimelineNodeCreated {
        node: TimelineNode,
    },
    TimelineNodeUpdated {
        node_id: String,
        status: TimelineNodeStatus,
        summary: Option<String>,
        completed_at: Option<String>,
    },
    ReviewComplete {
        node_id: String,
        round: u32,
        verdict: ReviewVerdictType,
        comments: String,
        summary: String,
        findings: Vec<ReviewFinding>,
        review_gate: ReviewGate,
        work_item_plan_review: Option<WorkItemPlanReviewComplete>,
        structured_output_diagnostic: Option<StructuredOutputDiagnostic>,
    },
    ReviewDecisionRequired {
        node_id: String,
        round: u32,
        options: Vec<String>,
    },
    HumanGateClosed {
        decision: String,
        stage: String,
    },
    /// 人工门已开启；Web runtime 以当前全量 session_state 同步所有已连接页面。
    HumanGateOpened {
        stage: String,
    },
    /// A1c（Task 2.2）：durable LC 恢复事实变化（waiting/fresh/resolved 状态
    /// 或 operation 结果推进）。Web runtime 以全量 authoritative
    /// session_state 同步所有已连接页面，不造竞态 delta。此事件不经
    /// WebSocket 对客户端序列化；mapping.rs 仅穷举忽略（BP1 最小 None 臂），
    /// 真实广播接线由 Task 2.4（A1b2）router 串行接管。
    LcResumeRecoveryChanged,
    Error {
        message: String,
    },
    /// 指引卡简化补缺：author/reviewer 启动臂 trust 拒绝的结构化指导
    /// （同源只读等待投影：providers/canonical_root/reason_code/
    /// retry_action/command_markdown）。紧随 `EngineEvent::Error` 之后
    /// 发出（error 帧先清旧指导、此帧再建——与 SC admission waiting 路径
    /// 同 pattern）；web 层映射 `WsOutMessage::TrustMissingGuidance`，前端
    /// 「两行说明＋可复制命令窗」直渲。平铺 error/manual_guidance 原样
    /// 保留（旧客户端兼容）。仅分类桥实际命中 trust 分型时发出（非
    /// trust 错误零发出）。
    TrustMissingGuidance {
        waiting: crate::product::logical_codebase::provider_trust::ProviderTrustWaiting,
    },
    ProtocolError {
        code: String,
        message: String,
        context: Option<serde_json::Value>,
    },
    PermissionTimeout {
        permission_id: String,
        node_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewDecisionOutcome {
    StartRevision,
    StartWorkItemPlanOutline,
    StartWorkItemPlanOutlineRevision {
        feedback: Option<String>,
    },
    StartWorkItemDraft {
        feedback: Option<String>,
    },
    StartWorkItemBatch,
    HumanConfirm,
    ConfirmedWithChildSessions {
        child_sessions: Vec<WorkspaceSessionRecord>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkItemPlanOutlineRevisionSource {
    AuthorConfirm,
    ReviewDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutlineRevisionPersistencePolicy {
    AllowMissingInitialRound,
    RequireActiveRound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutlineRevisionCrashPoint {
    Status,
    ArtifactVersions,
    Timeline,
    SourceNodeDetail,
    RunNodeDetail,
    PlanDrafts,
    ActiveIndex,
    Committed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanRepairCrashPoint {
    TimelinePersisted,
    SnapshotPersisted,
    SessionPersisted,
    RequestPersisted,
    LockReleased,
}

/// T4.1/T4.3（REQ-REG-15/D2）：人工确认 trust 按需登记的冻结上下文——
/// manager create 在已解析 project/issue/LC 并 `build_readonly_for_lc` 的
/// 现有段注入（复用同一 WebAppState 缓存的 trust precondition，不新造
/// 第二套 HOME 推导）；确认动作据此重验漂移并按需登记，root 唯一权威
/// 仍是冻结 gateway `authority_root()`，不另造浏览器/root authority。
#[derive(Clone)]
pub struct ConfirmationTrustContext {
    /// 共用 trust 前置门（与 aggregate initialization 生产依赖同一实例）。
    pub(crate) precondition: Arc<dyn crate::product::logical_codebase::ProviderTrustPrecondition>,
    /// create 期冻结的 issue 原始 LC 归属（None=单仓/legacy 无别名）。
    pub(crate) issue_lc_id: Option<String>,
    /// gateway 构造期冻结的 canonical 聚合根（`authority_root()`）。
    pub(crate) frozen_root: PathBuf,
}

/// T4.1（REQ-REG-15/D4）：确认 trust 按需登记的 typed 等待——
/// - `Home`：registry/home 侧失败等待（原 reason_code、场景化
///   retry_action=「修复后重新确认」与同源 manual_guidance；retryable 恒
///   true——表示修复后可显式重新确认，不表示冲突可无条件立即重试）。
/// - `Context`：冻结身份缺失/漂移等待（零 home 调用；只返回已知身份，
///   不编造 provider/root/key、不给错误根 trust 指引）。
///
/// web 层（change 5.1）以稳定 code `workspace_confirmation_trust_waiting`
/// 投影为 WS ProtocolError / REST 非成功 ApiError；reason_code 原样保留。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmationTrustWaiting {
    Home {
        session_id: String,
        project_id: String,
        lc_id: String,
        waiting: crate::product::logical_codebase::ProviderTrustWaiting,
    },
    Context {
        session_id: String,
        project_id: String,
        issue_id: String,
        /// 已知冻结身份（如实呈现已知部分；上下文缺失时为 None）。
        issue_lc_id: Option<String>,
        frozen_root: Option<PathBuf>,
        /// 稳定原因码：confirmation_trust_context_unavailable /
        /// confirmation_trust_scope_changed / confirmation_trust_root_unavailable /
        /// confirmation_trust_root_changed。
        reason_code: &'static str,
        message: String,
    },
}

impl ConfirmationTrustWaiting {
    /// 稳定原因码（Home=原 reason_code 透传，不折叠）。
    pub(crate) fn reason_code(&self) -> &str {
        match self {
            Self::Home { waiting, .. } => waiting.reason_code.as_str(),
            Self::Context { reason_code, .. } => reason_code,
        }
    }
}

/// 指引卡「登记并重试」按钮（owner 2026-10-12 直批）的显式登记解析
/// 结果——root 服务端解析（LC 会话=确认链同一冻结根+同一漂移重验；单仓
/// 会话=canonicalized repository 根，lc 标签走 legacy 别名），**不接受
/// 客户端传路径**。调用方据此在引擎锁外执行 ensure（home IO 不楔死
/// 引擎），precondition 复用确认链/聚合初始化同一生产实例。
pub(crate) struct ExplicitTrustRegistrationTarget {
    pub(crate) precondition: Arc<dyn crate::product::logical_codebase::ProviderTrustPrecondition>,
    pub(crate) project_id: String,
    pub(crate) session_id: String,
    pub(crate) registry_lc_id: String,
    pub(crate) canonical_root: PathBuf,
}

/// 解析失败的 fail-closed 等待（零 home 调用）：稳定原因码
/// （trust_registration_context_unavailable / trust_registration_scope_changed /
/// trust_registration_root_unavailable / trust_registration_root_changed）+
/// 中文说明；web 层以稳定 code `trust_registration_waiting` 投影，
/// reason_code 原样保留。
#[derive(Debug)]
pub(crate) struct ExplicitTrustRegistrationContextWaiting {
    pub(crate) reason_code: String,
    pub(crate) message: String,
}

// 确认裁决按值上抛;TrustWaiting 携带完整停等上下文,Box 化会波及引擎/web 两侧。
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceConfirmOutcome {
    WorkItemPlan {
        child_sessions: Vec<WorkspaceSessionRecord>,
    },
    /// T4.1（REQ-REG-15/D4）：确认 trust 未 Ready 的 typed 停等——零定稿
    /// 副作用、门保持开态；由 web 层（change 5.1）映射为稳定 waiting code。
    TrustWaiting {
        waiting: ConfirmationTrustWaiting,
    },
    None,
}

/// HTTP confirm 端点的引擎裁决结果（F-31 fix round，k3 P1/P2）。
///
/// 端点不再自行判定「能否定稿」：引擎是 `stage` 的唯一权威，端点只按裁决做 durable 写与广播。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HttpConfirmDisposition {
    /// 已接管本轮：进入 CrossReview 并申请 `ReviewOnly` run（Fake reviewer 的 Skipped
    /// 快速路径落 HumanConfirm）。端点不得定稿，权威状态以广播的 session_state 收敛。
    ReviewStarted,
    /// 引擎判定本轮定稿：端点完成 durable 写（含类型化 confirm gate 错误面）后调用
    /// `WorkspaceEngine::commit_finalize_artifact` 收敛引擎面（产物 confirmed_by /
    /// 在途节点 / 终态 stage / Completed 节点）。
    Finalize,
    /// 会话已是 Confirmed 终态：幂等返回，不重复写入。
    AlreadyConfirmed,
    /// WorkItemPlan 批次确认门（`AuthorConfirm` + active `WorkItemBatchConfirm`）：引擎
    /// **已在本裁决内**经 `confirm_work_item_plan` 执行面落 Confirmed（plan 确认 + 子
    /// WorkItem 会话 + stage→Completed + Completed 节点）。端点不得再走 store-only
    /// 兜底定稿，只广播权威状态并返回当前记录。
    WorkItemPlanConfirmed,
    /// WorkItemPlan 批次确认门的引擎拒绝（如缺失 compiled WorkItems）：端点如实 409 并
    /// **携带引擎诊断 message**（可诊断、不降级为空错误），不得回落 store-only 假 Confirmed。
    WorkItemPlanRejected { message: String },
    /// 用户显式要求送审（with_review=true）但会话未启用 review：端点必须如实 4xx
    /// （workspace_session_review_not_enabled），不得静默按定稿处理。
    ReviewUnavailable,
    /// 当前 stage 不得定稿（run/评审在途，或终态未 Confirmed）：端点返回 409。
    Rejected { stage: &'static str },
    /// 非 story/design 会话：端点维持既有 store-only 定稿通路。
    NotHandled,
}

// 退役留档（T5/REQ-RET-02）：AuthorDecisionOutcome 随 handle_author_decision/
// handle_work_item_plan_outline_decision 删除（wire 唯一入口已退役）。

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkItemPlanAuthorOutcome {
    /// validate 通过（warnings 随 candidate 推送），进 AuthorConfirm。
    AuthorConfirm,
    /// validate 有 errors，需 handler 层重新调 WorkItemSplitEngine::generate 重生。
    /// findings 作为 revision feedback 注入重生 prompt。
    AutoRevision { findings: Vec<WorkItemSplitFinding> },
    /// 连续重生超阈值（3 次）仍 has_errors，交用户决策。
    HumanConfirm { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkItemDraftDecisionOutcome {
    StartDraftRun,
    StartReview,
    HumanConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkItemDraftAuthorOutcome {
    AwaitConfirmation,
    RetryOnce {
        feedback: String,
        diagnostics: crate::product::models::WorkItemDraftGenerationDiagnostics,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkItemBatchDecisionOutcome {
    StartBatchRun,
    StartDraftRun,
    StartReview,
    HumanConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkItemPlanCompileRecoveryOutcome {
    Continue,
    HumanConfirm,
}

pub(crate) struct WorkItemPlanCompileProjectionContext<'a> {
    pub(crate) outline_order: &'a [String],
    pub(crate) outline_to_work_item_id: &'a BTreeMap<String, String>,
    pub(crate) outline_to_verification_plan_id: &'a BTreeMap<String, String>,
    pub(crate) repository_id: &'a str,
    pub(crate) logical_targets: Option<
        &'a std::collections::BTreeMap<
            crate::product::logical_codebase::LogicalRepositoryId,
            String,
        >,
    >,
    pub(crate) now: &'a str,
}

/// SG3a.1（plan §0.5 接口字典）：候选结构门产出的「已备好的合法结构候选」。
/// `report` 是 SG1 纯有效图投影的同代结果；`source_hash`/`context_digest`
/// 绑定候选 source 与冻结上下文，供 SG3a.0 payload 同 refs 落盘、
/// SG3b Final Compile 复验与 SG6a.1 修订轮评估消费。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedStructuralCandidate {
    pub(crate) source: String,
    pub(crate) ir: crate::product::work_item_plan_compiler::PlanCandidateIr,
    pub(crate) report:
        crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureReport,
    pub(crate) source_hash: String,
    pub(crate) context_digest: String,
}

/// SG3a.1（plan §0.5 接口字典）：候选输出的结构评估结果——
/// - `Accepted`：全套件通过，可随 current refs 冻结结构报告；
/// - `Rejected`：typed 结构失败（事实完整），修复指令初轮为空集
///   （SG5/SG6 接线 instruction producer 后携带）；
/// - `ExistingGate`：命中既有业务门（SG6a.1 修订轮语义，SG3a.1 不产生）；
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StructuralAssessment {
    Accepted(Box<PreparedStructuralCandidate>),
    Rejected(
        crate::product::planning_failure::PlanningFailureSet,
        Vec<crate::product::workspace_engine::contract_prerevision::StructuralRepairInstruction>,
    ),
    /// 尚未接线：SG3a.1 系统门不产生该分支，SG6a.1 修订轮评估消费。
    #[allow(dead_code)]
    ExistingGate(crate::product::planning_failure::PlanningFailureSet),
    Operational(crate::product::planning_failure::PlanningFailureSet),
}

/// SG3b（plan §0.5 接口字典）：Final Compile 的 typed 失败。kind 为字典
/// 两变体 `Structure(PlanningFailureSet) | Operational{stable_code,cause}`，
/// 并按字典要求「携本次 failure_id/approval/source context」——身份字段由
/// SC Final Compile 链在返回边界统一 seal（内容寻址 failure_id），不经
/// String 渲染再 parse；Operational 保留原 cause 原文与既有 stable code。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanningCompileFailure {
    pub(crate) kind: PlanningCompileFailureKind,
    /// 内容寻址身份（sha256，seal 前为空）；同输入幂等复现。
    pub(crate) failure_id: String,
    /// 本次 close 的 approval 元组身份（approval CAS 前失败为 None）。
    pub(crate) approval_attempt_id: Option<String>,
    pub(crate) source_revision_ref: Option<String>,
    pub(crate) plan_candidate_ir_ref: Option<String>,
    pub(crate) mechanical_report_ref: Option<String>,
    /// 结构复验时的 live 上下文 digest（未达结构门为 None）。
    pub(crate) context_digest: Option<String>,
    /// 产生本次失败的 compile 运行（reservation 前失败为 None）；close 侧
    /// 仅按它匹配本运行 Failed transaction 的 validator findings。
    pub(crate) compile_run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PlanningCompileFailureKind {
    Structure(crate::product::planning_failure::PlanningFailureSet),
    Operational { stable_code: String, cause: String },
}

impl PlanningCompileFailure {
    /// 结构失败族稳定 code（单值映射，供 durable diagnostic/DTO 复用）。
    pub(crate) const STRUCTURE_STABLE_CODE: &'static str = "final_compile_structure_rejected";

    pub(crate) fn operational(stable_code: &str, cause: String) -> Self {
        Self {
            kind: PlanningCompileFailureKind::Operational {
                stable_code: stable_code.to_string(),
                cause,
            },
            failure_id: String::new(),
            approval_attempt_id: None,
            source_revision_ref: None,
            plan_candidate_ir_ref: None,
            mechanical_report_ref: None,
            context_digest: None,
            compile_run_id: None,
        }
    }

    pub(crate) fn structure(
        failures: crate::product::planning_failure::PlanningFailureSet,
    ) -> Self {
        Self {
            kind: PlanningCompileFailureKind::Structure(failures),
            failure_id: String::new(),
            approval_attempt_id: None,
            source_revision_ref: None,
            plan_candidate_ir_ref: None,
            mechanical_report_ref: None,
            context_digest: None,
            compile_run_id: None,
        }
    }

    pub(crate) fn stable_code(&self) -> &str {
        match &self.kind {
            PlanningCompileFailureKind::Operational { stable_code, .. } => stable_code,
            PlanningCompileFailureKind::Structure(_) => Self::STRUCTURE_STABLE_CODE,
        }
    }

    /// 人读渲染（时间线/事件与旧 String 边界共用；不是身份来源）。
    pub(crate) fn render(&self) -> String {
        match &self.kind {
            PlanningCompileFailureKind::Operational { stable_code, cause } => {
                format!("[{stable_code}] {cause}")
            }
            PlanningCompileFailureKind::Structure(set) => {
                let failures = set
                    .failures
                    .iter()
                    .map(|failure| {
                        let mut body = failure.evidence.affected_ids.join("->");
                        // 5.3：evidence.details 的机器事实（环 cause 原文/指纹/
                        // 机器 id 标记段）随渲染保留——旧 String 边界与
                        // `compile_failure_surface` 的环事实解析锚点不回退。
                        for (key, value) in &failure.evidence.details {
                            body.push_str(&format!(" {key}={value}"));
                        }
                        format!("{}[{body}]", failure.kind.stable_code())
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                format!("final compile structure gate rejected: {failures}")
            }
        }
    }

    /// 返回边界统一 seal：从 durable record 补 approval/source 三 refs 并
    /// 计算内容寻址 failure_id（同失败重放幂等同 id）。
    pub(crate) fn seal(
        &mut self,
        session_id: &str,
        record: &crate::product::models::WorkspaceSessionRecord,
    ) {
        self.seal_with_identity(
            session_id,
            record.approval_attempt_id.clone(),
            record.work_item_plan_source_revision_ref.clone(),
            record.plan_candidate_ir_ref.clone(),
            record.mechanical_report_ref.clone(),
        );
    }

    pub(crate) fn seal_with_identity(
        &mut self,
        session_id: &str,
        approval_attempt_id: Option<String>,
        source_revision_ref: Option<String>,
        plan_candidate_ir_ref: Option<String>,
        mechanical_report_ref: Option<String>,
    ) {
        self.approval_attempt_id = approval_attempt_id;
        self.source_revision_ref = source_revision_ref;
        self.plan_candidate_ir_ref = plan_candidate_ir_ref;
        self.mechanical_report_ref = mechanical_report_ref;
        self.failure_id = self.compute_failure_id(session_id);
    }

    fn compute_failure_id(&self, session_id: &str) -> String {
        use sha2::{Digest, Sha256};
        let kind_payload = match &self.kind {
            PlanningCompileFailureKind::Operational { stable_code, cause } => {
                serde_json::json!({ "stable_code": stable_code, "cause": cause })
            }
            PlanningCompileFailureKind::Structure(set) => serde_json::json!({
                "structure_failures": set,
            }),
        };
        let input = serde_json::json!({
            "session_id": session_id,
            "kind": kind_payload,
            "approval_attempt_id": self.approval_attempt_id,
            "source_revision_ref": self.source_revision_ref,
            "plan_candidate_ir_ref": self.plan_candidate_ir_ref,
            "mechanical_report_ref": self.mechanical_report_ref,
            "context_digest": self.context_digest,
            "compile_run_id": self.compile_run_id,
        });
        let digest = Sha256::digest(input.to_string().as_bytes());
        format!("planning_compile_failure_{digest:x}")
    }
}

impl std::fmt::Display for PlanningCompileFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.render())
    }
}
/// 5.3（REQ-WSC-24 残留）：Final Compile prepare 阶段的 typed 错误通道——
/// 依赖环/结构失败以 `Structure(PlanningFailureSet)` 透传（机器标记段原文
/// 随集合的 evidence 保留），其余按 `Operational(cause)`。失败面分流与该
/// 枚举的 impl 见 `compile/prepare_failure.rs`。类型 `pub` + 所在模块私有
/// （Task 2 可见性裁定先例）⇒ crate 内可达，不进 crate 外 API 面。
#[derive(Debug, Clone, PartialEq)]
pub enum InitialPlanCompileFailure {
    Operational(String),
    Structure(crate::product::planning_failure::PlanningFailureSet),
}

#[cfg(test)]
pub(crate) type PolicyRouteBeforePersistHook = Box<dyn FnOnce(&LifecycleStore, &str) + Send>;

pub struct WorkspaceEngine {
    pub(crate) checkpoint_store: Arc<CheckpointStore>,
    pub(crate) lifecycle_store: Option<LifecycleStore>,
    pub(crate) event_tx: mpsc::Sender<EngineEvent>,
    pub(crate) session: WorkspaceSession,
    pub(crate) cancel: CancellationToken,
    pub(crate) timeline_nodes: Vec<TimelineNode>,
    pub(crate) active_node_id: Option<String>,
    pub(crate) artifact_versions: Vec<ArtifactVersion>,
    pub(crate) latest_review_verdict: Option<ReviewVerdict>,
    pub(crate) pending_revision_context: Option<String>,
    pub(crate) pending_author_choice: Option<PendingAuthorChoice>,
    pub(crate) active_run_id: Option<String>,
    pub(crate) stream_buffers: HashMap<String, PendingStreamBuffer>,
    pub(crate) work_item_plan_author_retry_count: u32,
    pub(crate) work_item_plan_revision_retry_count: u32,
    pub(crate) work_item_draft_repair_states: HashMap<String, WorkItemDraftRepairState>,
    pub(crate) outline_revision_recovery_error: Option<String>,
    pub(crate) outline_revision_crash_after: Option<OutlineRevisionCrashPoint>,
    pub(crate) plan_repair_crash_after: Option<PlanRepairCrashPoint>,
    pub(crate) plan_repair_snapshot: Option<PlanRepairSessionSnapshotDto>,
    #[cfg(test)]
    pub(crate) policy_route_before_persist: Option<PolicyRouteBeforePersistHook>,
    /// Task 11:逻辑代码库真实 provider 启动的唯一入口。非空时,planning author stream
    /// 经 `LogicalCodebaseProviderGateway::start_streaming` 启动并留 audit;为 `None`
    /// 时(传统单仓/非逻辑 issue)保留直接 `provider.start` 路径。Web 接入 task 为
    /// 逻辑代码库 issue 注入真实 gateway。
    pub(crate) logical_provider_gateway:
        Option<Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>>,
    /// Task 2.3（REQ-ENV-04 cwd 维度）：LC gateway launch 冻结的 resume 指纹记忆
    /// （author provider → 最近一次 author 面 launch 的 cwd-inclusive
    /// fingerprint；author 首轮/choice follow-up/revision 的 envelope 同源）。
    /// revision resume 决策据此比对 cwd 漂移：指纹漂移 → supersede 旧 native
    /// session 并新建；记忆缺失（跨连接重建 engine）→ fail-closed 新会话，
    /// 不静默续接旧 thread。
    pub(crate) logical_launch_fingerprints: HashMap<
        crate::product::models::ProviderName,
        crate::product::logical_codebase::SessionResumeFingerprint,
    >,
    /// F7 项 1：最近一次 close_human_gate 中 confirm 后 compile 失败的结构化
    /// 详情（每次 close 入口清空，仅当次失败时非空）。
    pub(crate) last_gate_close_compile_failure:
        Option<super::conversational_gate::HumanGateCloseCompileFailure>,
    /// T4.1/T4.3（REQ-REG-15/D2）：人工确认 trust 按需登记的冻结上下文
    /// （manager create 与 `logical_provider_gateway` 同段注入）。LC 会话
    /// 有 gateway 却缺此上下文时确认 fail-closed ContextWaiting；非 LC
    /// 会话（无 gateway）免登记，见 `ensure_confirmation_trust_ready`。
    pub(crate) confirmation_trust_context: Option<ConfirmationTrustContext>,
    /// 批2 Task 20b(决策项⑦):规划面 reviewer 会话的活动审查窗口——
    /// `drive_review_session(_via_gateway)` 入口在材料采集前开窗并持有;
    /// repair/静默重试同窗复用;prompt 铸造 ReviewerVerify 合同的唯一
    /// 窗口来源。`None` = 无活动审查(非 review 会话/内层直调测试形态)。
    pub(crate) active_review_window:
        Option<crate::product::workspace_engine::review::ActivePlanReviewWindow>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkItemDraftRepairState {
    pub(crate) initial_validation_findings: Option<Vec<WorkItemSplitFinding>>,
    pub(crate) original_user_feedback: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingAuthorChoice {
    pub(crate) id: String,
    pub(crate) prompt: String,
    pub(crate) options: Vec<ChoiceOptionData>,
    pub(crate) source_node_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorPromptMode {
    FullConversation,
    DeltaOnly,
}

impl AuthorPromptMode {
    pub(crate) fn prompt_event_detail(self) -> &'static str {
        match self {
            Self::FullConversation => "发送给 Workspace provider 的完整提示词",
            Self::DeltaOnly => "发送给 Workspace provider 的追加提示词",
        }
    }
}

/// F-60 P0（因素三）：`build_work_item_plan_streaming_input*` 的显式输出合同族。
/// 该出口同时承载 JSON 子链与 Markdown 直发两类 invocation，禁止从 prompt 文本
/// 猜用途——由调用方声明产物合同族：
/// - [`Self::Structured`]：JSON Outline／Split／Draft 与 SC compiler-source。
///   不注入 Markdown artifact gate 合同（JSON schema 契约由各自 builder 负责）。
/// - [`Self::MarkdownArtifact`]：Markdown artifact author 直发。出口注入与
///   `build_streaming_input` 同源的完整契约装配（WorkItemPlan gate 全条款）。
///
/// 当前生产调用方全部为 Structured（JSON 子链与 SC compiler-source，后者走
/// work-item-plan compiler 而非 artifact gate，误拼 Markdown 合同会教坏格式）；
/// MarkdownArtifact 为设计要求的显式合同族入口（方案因素三「仅对 Markdown
/// author invocation 调同一装配」），由 f60_exit_contract_matrix 测试锁定两侧行为，
/// 后续 Markdown 直发调用方接入时不得绕过。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanAuthorOutputContract {
    Structured,
    #[allow(dead_code)]
    MarkdownArtifact,
}

pub(crate) struct ArtifactRetryContext {
    pub(crate) provider: Arc<dyn StreamingProviderAdapter>,
    pub(crate) input: StreamingProviderInput,
    pub(crate) attempted: bool,
}

pub(crate) struct RevisionResumeFallbackContext {
    pub(crate) provider: Arc<dyn StreamingProviderAdapter>,
    pub(crate) attempted: bool,
}

pub(crate) struct ProviderSessionDriveInput {
    pub(crate) session:
        Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError>,
    pub(crate) command_rx: mpsc::Receiver<ProviderCommand>,
    pub(crate) node_id: Option<String>,
    pub(crate) agent: Option<ProviderName>,
    pub(crate) role: ProviderConversationRole,
    pub(crate) artifact_retry: Option<ArtifactRetryContext>,
    pub(crate) revision_resume_fallback: Option<RevisionResumeFallbackContext>,
}

pub(crate) enum ReviewProviderRunResult {
    Completed(ProviderCompletion),
    Aborted,
    Failed(ReviewProviderRunFailure),
}

pub(crate) enum ReviewProviderRunFailure {
    /// reviewer 会话启动失败：保留原 adapter 错误码（GAP-H Task 0.3 分类
    /// 不从拼接后的错误文案猜 code），message 为有界 details+stderr。
    Start {
        message: String,
        code: crate::protocol::provider_errors::ProviderErrorCode,
    },
    EmptyOutput,
    /// reviewer 运行期失败：streaming 事件只带字符串消息，无结构化错误码，
    /// 分类时按无 code 处理（不误报 503 类）。
    Provider {
        message: String,
        code: Option<crate::protocol::provider_errors::ProviderErrorCode>,
    },
    PermissionTimeout(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAuthorChoiceError {
    NotFound { id: String },
    IdMismatch { expected: String, actual: String },
    OptionUnmatched { id: String },
}

impl PendingAuthorChoiceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "PENDING_AUTHOR_CHOICE_NOT_FOUND",
            Self::IdMismatch { .. } => "CHOICE_ID_UNMATCHED",
            Self::OptionUnmatched { .. } => "CHOICE_OPTION_UNMATCHED",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotFound { id } => {
                format!("choice_response id={id} has no pending author choice")
            }
            Self::IdMismatch { expected, actual } => {
                format!("choice_response id={actual} does not match pending choice id={expected}")
            }
            Self::OptionUnmatched { id } => format!("selected option id={id} is not available"),
        }
    }
}

pub(crate) struct PendingStreamBuffer {
    pub(crate) content: String,
    pub(crate) last_flush_at: Instant,
}

impl Default for PendingStreamBuffer {
    fn default() -> Self {
        Self {
            content: String::new(),
            last_flush_at: Instant::now(),
        }
    }
}

pub(crate) struct StructuredOutputDisplayFilter {
    pub(crate) pending: String,
    pub(crate) inside_structured_output: bool,
}

impl StructuredOutputDisplayFilter {
    pub(crate) fn new() -> Self {
        Self {
            pending: String::new(),
            inside_structured_output: false,
        }
    }

    pub(crate) fn push(&mut self, chunk: &str) -> String {
        self.pending.push_str(chunk);
        let mut visible = String::new();

        loop {
            if self.inside_structured_output {
                if let Some(end_index) = self.pending.find(STRUCTURED_OUTPUT_END) {
                    let drain_end = end_index + STRUCTURED_OUTPUT_END.len();
                    self.pending.drain(..drain_end);
                    self.inside_structured_output = false;
                    continue;
                }

                let keep = longest_suffix_prefix_len(&self.pending, STRUCTURED_OUTPUT_END);
                if self.pending.len() > keep {
                    self.pending.drain(..self.pending.len() - keep);
                }
                break;
            }

            if let Some(start_index) = self.pending.find(STRUCTURED_OUTPUT_START_PREFIX) {
                visible.push_str(&self.pending[..start_index]);
                let drain_end = start_index + STRUCTURED_OUTPUT_START_PREFIX.len();
                self.pending.drain(..drain_end);
                self.inside_structured_output = true;
                continue;
            }

            let keep = longest_suffix_prefix_len(&self.pending, STRUCTURED_OUTPUT_START_PREFIX);
            if self.pending.len() > keep {
                let emit: String = self.pending.drain(..self.pending.len() - keep).collect();
                visible.push_str(&emit);
            }
            break;
        }

        visible
    }

    pub(crate) fn finish(&mut self) -> String {
        if self.inside_structured_output {
            self.pending.clear();
            String::new()
        } else {
            std::mem::take(&mut self.pending)
        }
    }
}

pub(crate) fn longest_suffix_prefix_len(value: &str, pattern: &str) -> usize {
    let max_len = value.len().min(pattern.len().saturating_sub(1));
    (1..=max_len)
        .rev()
        .find(|len| value.ends_with(&pattern[..*len]))
        .unwrap_or(0)
}

pub(crate) struct TimelineNodeDraft {
    pub(crate) node_type: TimelineNodeType,
    pub(crate) agent: Option<ProviderName>,
    pub(crate) stage: WorkspaceStage,
    pub(crate) round: Option<u32>,
    pub(crate) title: String,
    pub(crate) summary: Option<String>,
    pub(crate) status: TimelineNodeStatus,
}
