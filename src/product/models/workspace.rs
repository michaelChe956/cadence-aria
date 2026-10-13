use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::human_gate::{
    HumanGateReservation, HumanGateTurnStatus, UserBudgetHold, UserBudgetReceipt,
    UserBudgetReceiptKind,
};
use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::product::work_item_plan_policy::{
    HumanGateSnapshot, PolicyDiagnostic, ProviderStartLedgerEntry, RepairReservation,
    ReviewInvocationScope, RunHistory, RunPolicy, WorkItemPlanFlowKind,
};
use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

use super::outline::WorkItemDraftStatus;
use super::provider::{ProviderConversationRef, ProviderConversationRole, ProviderName};
use super::work_item_revision::{ChildBindingIdentity, WorkItemRuntimeBinding};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceType {
    Story,
    Design,
    WorkItem,
    WorkItemPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSessionStatus {
    Open,
    Running,
    WaitingForHuman,
    Confirmed,
    ChangeRequested,
    BlockedProviderUnavailable,
    Terminated,
    StoppedNeedsHuman,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceRolePermissionModes {
    pub author: ProviderPermissionMode,
    pub reviewer: ProviderPermissionMode,
}

impl Default for WorkspaceRolePermissionModes {
    fn default() -> Self {
        Self {
            author: ProviderPermissionMode::Auto,
            reviewer: ProviderPermissionMode::Auto,
        }
    }
}

fn default_flow_kind() -> WorkItemPlanFlowKind {
    WorkItemPlanFlowKind::Legacy
}

fn default_run_policy() -> RunPolicy {
    RunPolicy::Interactive
}

/// 单候选路径的 durable 阶段。旧 session 缺失该字段时维持 legacy 语义。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SingleCandidatePhase {
    Prepare,
    Generate,
    Evaluate,
    Approval,
    Completed,
    Failed,
}

/// 与 Work Item 单候选契约使用的名称保持兼容。
pub type WorkItemPlanSingleCandidatePhase = SingleCandidatePhase;

/// Approval 成功后确定性派生的编译三元组；同一 session 只能持久化同一值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingleCandidateCompileReservation {
    pub compile_id: String,
    pub now: String,
    pub publication_provenance_ref: String,
}

/// P2 GAP-E/G（Task 0.1）：失败 SingleCandidate 会话的人工显式重驱单发
/// 认领——同 session 文件锁内冻结 `(failed_node_id, command_id)`；异键
/// 409、同键读原结果；不可复位（重臂/再失败不清洗，只由人显式发起）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScRecoveryClaim {
    pub failed_node_id: String,
    pub command_id: String,
    pub state: ScRecoveryClaimState,
}

/// 认领的持久结果：Accepted=已受理派发；NeedsHuman=派发外部副作用是否
/// 发生不可证明，停人工分诊（不自动重试）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScRecoveryClaimState {
    Accepted,
    NeedsHuman,
}

/// A1a（Task 2.1）：LC launch 指纹的 basis。旧 basis 不与新 full 混比；
/// 缺 map/旧 basis → `resume_fingerprint_unavailable` 等待，不自动回填。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalLaunchBasis {
    FullLcProjectionV2,
}

/// launch 指纹耐久状态：Prepared=spawn 前冻结；Active=native/run 绑定后；
/// Superseded=tombstone 已落，旧 native 永不复活。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalLaunchFingerprintState {
    Prepared,
    Active,
    Superseded,
}

fn default_lc_launch_schema_version() -> u16 {
    1
}

/// A1a（Task 2.1）：同源完整 candidate 与 run-bound audit identity 就绪后、
/// session spawn 前持久化的 full 投影指纹记录——按 native/run 绑定，不只
/// 按 provider 覆旧。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalLaunchFingerprintRecord {
    #[serde(default = "default_lc_launch_schema_version")]
    pub schema_version: u16,
    pub basis: LogicalLaunchBasis,
    pub role: ProviderConversationRole,
    pub provider: ProviderName,
    pub role_run_seq: u64,
    pub node_id: String,
    /// 仅从真实 handshake/Completed/同 role_run_seq provider_start audit
    /// 绑定；Prepared 阶段为 None，不倒算、不伪造虚 ID。
    pub native_session_id: Option<String>,
    pub fingerprint_digest: String,
    pub planning_access_fingerprint: String,
    pub state: LogicalLaunchFingerprintState,
    pub created_at: String,
    pub updated_at: String,
}

/// 旧 native 会话的耐久 tombstone：多代保留；清 conversation 不代替吊销；
/// 旧 ID 不验证不造虚 ID，但 recovery waiting 仍存。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupersededNativeSessionRecord {
    pub role: ProviderConversationRole,
    pub provider: ProviderName,
    pub native_session_id: String,
    pub reason_code: String,
    pub recovery_id: String,
    pub superseded_at: String,
}

/// 显式 fresh operation 的耐久状态：已受理 ID 不可再次 claim，prepare
/// 失败回 waiting 时同 ID 只 Replay，不删除结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LcFreshOperationState {
    NotStarted,
    Reserved,
    Running,
    Completed,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcFreshOperationResult {
    pub operation_id: String,
    pub state: LcFreshOperationState,
    pub fresh_node_id: Option<String>,
    pub reason_code: Option<String>,
    pub role_run_seq: Option<u64>,
    pub accepted_at: String,
    pub updated_at: String,
}

/// recovery 状态机：Waiting=漂移判定后的持久等待；FreshReserved=显式
/// fresh claim 受理（provider start 仍为 0）；FreshRunning=prepare 成功的
/// 新 native 运行；Resolved=完整 artifact 完成、证据保留。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LcResumeRecoveryState {
    Waiting,
    FreshReserved,
    FreshRunning,
    Resolved,
}

fn default_lc_resume_recovery_schema_version() -> u16 {
    1
}

/// tagged 原 run 上下文的 durable artifact/source 引用（引用＋hash，不
/// 序列化 runtime Arc/ProviderRunKind）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcRecoveryArtifactRef {
    pub ref_id: String,
    pub version: Option<u32>,
    pub hash: Option<String>,
}

/// Legacy Draft/Batch 每稿的耐久状态引用（feedback 在 builder.take 前固化）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcRecoveryDraftRef {
    pub draft_id: String,
    pub status: WorkItemDraftStatus,
    pub version: Option<u32>,
    pub hash: Option<String>,
}

/// LegacyPlanRevision 的保留项（retained item ID/revision）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcRetainedPlanItemRef {
    pub item_id: String,
    pub revision: u32,
}

/// LegacyPlanRevision 的 redo 项（old_id/feedback）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcRecoveryRedoSpec {
    pub old_item_id: String,
    pub feedback: Option<String>,
}

/// 全族公共业务事实：entity/plan、原 business repository 已知身份、
/// story/design 源 ID、plan options、artifact/source 版本引用＋hash、
/// review nodes、author/effective reviewer、原 node/role_run_seq。未知身份
/// 留 None，不倒算。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcRecoveryBusinessContext {
    pub entity_id: String,
    pub plan_id: Option<String>,
    pub workspace_type: WorkspaceType,
    pub business_repository_physical_id: Option<String>,
    pub business_repository_logical_id: Option<String>,
    pub business_checkout_id: Option<String>,
    pub business_repository_path: Option<String>,
    pub source_story_ids: Vec<String>,
    pub source_design_ids: Vec<String>,
    pub plan_options_ref: Option<String>,
    pub artifact_refs: Vec<LcRecoveryArtifactRef>,
    pub review_refs: Vec<String>,
    pub author_provider: ProviderName,
    pub effective_reviewer_provider: Option<ProviderName>,
    pub original_node_id: Option<String>,
    pub original_role_run_seq: Option<u64>,
}

/// LegacyOutlineRebuild 的 origin payload（legacy_outline_author）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcLegacyOutlineAuthorPayload {
    pub flow: WorkItemPlanFlowKind,
    pub original_provider_node_id: Option<String>,
    pub outline_ref: Option<String>,
}

/// LegacyOutlineRebuild 的 origin payload（legacy_outline_revision）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcLegacyOutlineRevisionPayload {
    pub normalized_feedback: String,
    pub outline_ref: Option<String>,
    pub outline_version: Option<u32>,
    pub outline_hash: Option<String>,
}

/// rebuild origin 仅接受 legacy_author / outline_revision 的完整 payload，
/// 不丢 feedback。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LcOutlineRebuildOrigin {
    LegacyAuthor(LcLegacyOutlineAuthorPayload),
    OutlineRevision(LcLegacyOutlineRevisionPayload),
}

/// A1a（Task 2.1）：tagged 原 run 上下文（serde kind tag）——各 Plan 身份
/// 的可重建事实。未知 kind/非法 flow 反序列化拒收，由上层映射 typed
/// `lc_resume_recovery_context_unsupported`，不 fallback Author/plan_author。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LcRecoveryRunContext {
    Author {
        business: LcRecoveryBusinessContext,
        original_content: Option<String>,
    },
    AuthorChoiceFollowup {
        business: LcRecoveryBusinessContext,
        original_content: Option<String>,
    },
    AuthorFeedback {
        business: LcRecoveryBusinessContext,
        original_feedback: Option<String>,
    },
    ReviewerRevision {
        business: LcRecoveryBusinessContext,
        original_feedback: Option<String>,
    },
    LegacyOutlineAuthor {
        business: LcRecoveryBusinessContext,
        flow: WorkItemPlanFlowKind,
        original_provider_node_id: Option<String>,
        outline_ref: Option<String>,
    },
    LegacyOutlineRevision {
        business: LcRecoveryBusinessContext,
        normalized_feedback: String,
        outline_ref: Option<String>,
        outline_version: Option<u32>,
        outline_hash: Option<String>,
    },
    LegacyOutlineRebuild {
        business: LcRecoveryBusinessContext,
        origin: LcOutlineRebuildOrigin,
        previous_snapshot_access_fingerprint: Option<String>,
    },
    LegacyDraft {
        business: LcRecoveryBusinessContext,
        generation_round_id: String,
        active_outline_id: String,
        draft_refs: Vec<LcRecoveryDraftRef>,
        pending_feedback: Option<String>,
        outline_ref: Option<String>,
        candidate_ref: Option<String>,
    },
    LegacyBatch {
        business: LcRecoveryBusinessContext,
        generation_round_id: String,
        active_outline_id: String,
        draft_refs: Vec<LcRecoveryDraftRef>,
        pending_feedback: Option<String>,
        outline_ref: Option<String>,
        candidate_ref: Option<String>,
        batch_id: String,
        batch_item_draft_refs: Vec<LcRecoveryDraftRef>,
    },
    LegacyPlanRevision {
        business: LcRecoveryBusinessContext,
        candidate_ref: Option<String>,
        candidate_version: Option<u32>,
        candidate_hash: Option<String>,
        retained_items: Vec<LcRetainedPlanItemRef>,
        redo_specs: Vec<LcRecoveryRedoSpec>,
        overall_feedback: Option<String>,
    },
    ScAuthor {
        business: LcRecoveryBusinessContext,
        flow: WorkItemPlanFlowKind,
        single_candidate_phase: SingleCandidatePhase,
        provider_start_reservation_key: Option<String>,
        source_revision_ref: Option<String>,
        ir_ref: Option<String>,
        report_ref: Option<String>,
        candidate_source_hash: Option<String>,
        business_material_identity: Option<String>,
    },
    ScManualRevision {
        business: LcRecoveryBusinessContext,
        turn_id: String,
        command_id: String,
        source_hash: String,
        turn_status: HumanGateTurnStatus,
        attempt_no: u32,
        budget_reserved: u32,
        human_gate_reservation_key: Option<String>,
        gate_snapshot_ref: Option<String>,
        source_revision_ref: Option<String>,
        feedback_text: Option<String>,
        assembly_digest: Option<String>,
        assembly_ref: Option<String>,
    },
}

/// A1a（Task 2.1）：当前 LC 恢复的 durable 主记录——恢复身份、reason、旧
/// native/digest/失败节点、原 business 状态、tagged run_context、状态机、
/// fresh run/node/native 绑定与 operation 结果历史。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcResumeRecoveryRecord {
    #[serde(default = "default_lc_resume_recovery_schema_version")]
    pub schema_version: u16,
    pub recovery_id: String,
    pub failed_provider_node_id: String,
    pub reason_code: String,
    pub role: ProviderConversationRole,
    pub provider: ProviderName,
    pub superseded_native_session_id: Option<String>,
    pub previous_fingerprint_digest: Option<String>,
    pub previous_session_status: WorkspaceSessionStatus,
    pub previous_stage: String,
    pub run_context: LcRecoveryRunContext,
    pub state: LcResumeRecoveryState,
    pub operation_id: Option<String>,
    pub fresh_node_id: Option<String>,
    pub fresh_native_session_id: Option<String>,
    pub fresh_role_run_seq: Option<u64>,
    pub fresh_operations: Vec<LcFreshOperationResult>,
    pub detected_at: String,
    pub updated_at: String,
}

/// SG4a.0/SG4a（plan v1.1 §0.5/§0.6.1）：结构治理唯一耐久聚合状态。旧
/// session 记录整体 None/空，不回填、不授予任何执行权；SG4a 已落预算
/// 数学（`available_user_budget`）与 reserve hold/claim/settle 账本
/// （lifecycle_store），checkpoint/commit 归 SG4b/SG6，runtime 镜像与
/// from_record 接线归 SG7a。accepted_feedback_turns 单源仍为
/// HumanGateSnapshot，actual launch 单源仍为 provider_start_ledger，
/// 本聚合不另立第二总账。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralGovernanceState {
    /// 当前结构修订授权窗口；旧记录 None（无授权，不回填）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_grant: Option<StructuralRevisionGrant>,
    /// 用户结构预算总额（沿现 human gate 默认 3）；可用额数学
    /// （总额−已计费 receipt−活动 hold）归 SG4a 投影，不在本字段预存。
    #[serde(default = "default_structural_user_budget_total")]
    pub user_budget_total: u32,
    /// 活动用户预算 hold（按 turn 幂等；SG4a.0 恒空，仅耐久形状）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_holds: Vec<UserBudgetHold>,
    /// 三终态用户预算 receipt（按 turn 幂等互斥；SG4a.0 恒空，仅耐久形状）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_receipts: Vec<UserBudgetReceipt>,
    /// 跨合法业务轮次继承的结构 checkpoint（SG4a.0 恒空，仅耐久形状）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structure_checkpoints: Vec<StructuralCheckpoint>,
    /// 已解决结构事实（规则谓词＋证据；SG4a.0 恒空，仅耐久形状）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_structure_facts: Vec<ResolvedStructureFact>,
    /// 当前结构报告 ref（同当前 mechanical_report_ref 的聚合位；不另立
    /// 与 source/IR/report 三 refs 脱节的第四候选权威）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_structure_report_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_context_digest: Option<String>,
    /// 可选 turn 摘要（呈现用；accepted 单源仍为 gate 快照，不是第二总账）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_turn_summary: Option<String>,
    /// 可选 launch reason（启动动机说明；不是启动授权）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_reason: Option<String>,
    /// 可选 launch 证据（实际观察承载；旧 started 布尔不构成证据）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_evidence: Option<StructuralLaunchEvidence>,
}

fn default_structural_user_budget_total() -> u32 {
    3
}

impl Default for StructuralGovernanceState {
    /// SG4a（REQ-CG-02）：reserve 首次落 hold 时物化聚合状态；总额沿现
    /// gate 默认 3（与 serde default 同源），不构成 grant/launch 授权。
    fn default() -> Self {
        Self {
            current_grant: None,
            user_budget_total: default_structural_user_budget_total(),
            user_holds: Vec::new(),
            user_receipts: Vec::new(),
            structure_checkpoints: Vec::new(),
            resolved_structure_facts: Vec::new(),
            current_structure_report_ref: None,
            current_context_digest: None,
            current_turn_summary: None,
            launch_reason: None,
            launch_evidence: None,
        }
    }
}

/// SG4a（REQ-CG-02）：三终态 receipt＋活动 hold 的可用预算数学式——
/// 可用 = 总额 − delivered_charge − nonstructural_failure_charge − 活动
/// hold；released_structural 不计费；accepted_feedback_turns 是用户动作
/// 计数、不是计费源。投影严格沿本式，不留第二计数旁路。
impl StructuralGovernanceState {
    /// 已交付计费轮次（delivered_charge）。
    pub fn delivered_charges(&self) -> u32 {
        self.user_receipts
            .iter()
            .filter(|receipt| receipt.kind == UserBudgetReceiptKind::DeliveredCharge)
            .count() as u32
    }

    /// 原合同非结构失败扣次（nonstructural_failure_charge，不退）。
    pub fn nonstructural_failure_charges(&self) -> u32 {
        self.user_receipts
            .iter()
            .filter(|receipt| receipt.kind == UserBudgetReceiptKind::NonstructuralFailureCharge)
            .count() as u32
    }

    /// 已计费额 = delivered_charge + nonstructural_failure_charge。
    pub fn charged_receipts(&self) -> u32 {
        self.delivered_charges() + self.nonstructural_failure_charges()
    }

    /// 活动 hold 数（每 turn 至多一条，reserve 幂等写）。
    pub fn active_hold_count(&self) -> u32 {
        self.user_holds.len() as u32
    }

    /// 当前可用用户预算（saturating：历史多扣不产生负数或退款授权）。
    pub fn available_user_budget(&self) -> u32 {
        self.user_budget_total
            .saturating_sub(self.charged_receipts())
            .saturating_sub(self.active_hold_count())
    }

    /// 按 turn 幂等查询终态 receipt（至多一条，三终态互斥）。
    pub fn receipt_for_turn(&self, turn_id: &str) -> Option<&UserBudgetReceipt> {
        self.user_receipts
            .iter()
            .find(|receipt| receipt.turn_id == turn_id)
    }

    /// 按 turn 幂等查询活动 hold（settle 后移除，不复活）。
    pub fn active_hold_for_turn(&self, turn_id: &str) -> Option<&UserBudgetHold> {
        self.user_holds.iter().find(|hold| hold.turn_id == turn_id)
    }

    /// SG4b（REQ-CG-02/05）：同 turn 终态结算的纯状态转移核心，语义
    /// 单源——`LifecycleStore::settle_structural_user_budget`（独立结算
    /// 入口，SG4a）与 `commit_structurally_valid_revision`（合法交付同
    /// CAS settle 一次，SG4b）都经本核心落账：终态按 turn 幂等互斥、恰
    /// 一次；同 receipt 重放零写；不同 receipt（含已 deliver 复评失败）
    /// 终态冲突不退款；无活动 hold 不凭空造账；released_structural 不
    /// 实扣、快照可用预算同转移恢复（不超过总额，旧已扣不追溯）。
    pub fn settle_turn_receipt(
        &mut self,
        human_gate_snapshot: Option<&mut crate::product::work_item_plan_policy::HumanGateSnapshot>,
        turn_id: &str,
        receipt: &UserBudgetReceipt,
    ) -> Result<StructuralSettlement, StructuralSettlementError> {
        if let Some(existing) = self.receipt_for_turn(turn_id) {
            return if *existing == *receipt {
                Ok(StructuralSettlement::Replayed)
            } else {
                Err(StructuralSettlementError::TerminalConflict)
            };
        }
        let Some(hold) = self.active_hold_for_turn(turn_id) else {
            return Err(StructuralSettlementError::NoActiveHold);
        };
        if hold.command_id != receipt.command_id {
            return Err(StructuralSettlementError::CommandMismatch);
        }
        let user_budget_total = self.user_budget_total;
        let released_structural = receipt.kind == UserBudgetReceiptKind::ReleasedStructural;
        self.user_holds.retain(|hold| hold.turn_id != turn_id);
        self.user_receipts.push(receipt.clone());
        if released_structural && let Some(snapshot) = human_gate_snapshot {
            snapshot.manual_repairs_remaining = snapshot
                .manual_repairs_remaining
                .saturating_add(1)
                .min(user_budget_total);
        }
        Ok(StructuralSettlement::Settled)
    }

    /// SG4b（REQ-SG-04）：按确定性 id 幂等记录 checkpoint。同 id 重放保
    /// 原记录（含 `recorded_at` 审计事实）；回升/落位 Active 前其它
    /// Active 统一转 Superseded——同一时刻至多一个 Active（finalize 按
    /// Active 定位当前交付代次，双 Active 会写错 review 证据代次）；
    /// grant 终态后 checkpoint 仍保留（不清理历史）。返回是否为新追加
    /// （false＝重放零写）。
    pub fn record_structure_checkpoint(&mut self, checkpoint: StructuralCheckpoint) -> bool {
        if let Some(index) = self
            .structure_checkpoints
            .iter()
            .position(|recorded| recorded.checkpoint_id == checkpoint.checkpoint_id)
        {
            if self.structure_checkpoints[index].status != StructuralCheckpointStatus::Active {
                for (offset, recorded) in self.structure_checkpoints.iter_mut().enumerate() {
                    if offset != index && recorded.status == StructuralCheckpointStatus::Active {
                        recorded.status = StructuralCheckpointStatus::Superseded;
                    }
                }
                self.structure_checkpoints[index].status = StructuralCheckpointStatus::Active;
            }
            return false;
        }
        for recorded in &mut self.structure_checkpoints {
            if recorded.status == StructuralCheckpointStatus::Active {
                recorded.status = StructuralCheckpointStatus::Superseded;
            }
        }
        self.structure_checkpoints.push(checkpoint);
        true
    }
}

/// 结构修订授权窗口（§0.5：绑定原 authorization hash/turn/command 与中间
/// candidate/phase/attempt/prompt/review 身份）。SG4a.0 只落耐久形状：
/// 缺字段默认 None/0，不构成任何新授权；claim/attempt 语义归 SG4a。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralRevisionGrant {
    pub grant_id: String,
    /// 原授权身份摘要（typed feedback turn／原 run 等耐久授权的 hash）。
    #[serde(default)]
    pub authorization_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    /// 中间候选身份（同代 source/IR/report refs，不另立第二权威）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_source_revision_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_candidate_ir_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mechanical_report_ref: Option<String>,
    /// grant phase（§0.5：非人工门枚举；缺字段读 None=Unknown，无阶段权）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<StructuralGrantPhase>,
    #[serde(default)]
    pub attempt_no: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_digest: Option<String>,
    /// SG4a（设计 §7.2）：claim 绑定的修复指令摘要（与 prompt digest 共同
    /// 构成 attempt 身份；旧 JSON 缺失读 None，无 claim 事实）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_digest: Option<String>,
    /// SG4a（设计 §7.3）：claim 时绑定的失败候选 source hash（不覆盖原
    /// turn.source_hash 授权，不冒充新授权）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failing_source_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_identity: Option<String>,
    pub granted_at: String,
}

/// grant phase（§0.5 SG4a：AuthorRunning/…/BusinessGate，非人工门枚举）。
/// SG4a.0 仅形状；阶段推进权归 SG4a。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralGrantPhase {
    AuthorRunning,
    Assessing,
    InstructionFrozen,
    Attempt2Claimed,
    Reviewing,
    BlockedByLcRecovery,
    Resolved,
    SystemFault,
    BusinessGate,
}

/// SG4a（plan v1.1 §0.5）：`claim_structural_attempt` 的结果。`Claimed`
/// 携带本 turn 分配到的物理 attempt 与唯一 provider start key——真实启动
/// 仍单源 provider_start_ledger，claim 不代写启动事实；`Replayed`＝同
/// prompt 重复 claim（REQ-CG-02 恰一次，不加 attempt）；`Waiting`＝无
/// 活动 grant/turn 绑定缺失/anchor waiting 等等待态（零写零新授权）；
/// `Exhausted`＝同 turn 物理 attempt 上限已耗尽（REQ-SG-05：L0/L1 共用
/// 同一 attempt key，无第 3 次）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuralClaimOutcome {
    Claimed {
        attempt_no: u32,
        prompt_ref: String,
        prompt_digest: String,
        provider_start_key: String,
    },
    Replayed,
    Waiting {
        reason: String,
    },
    Exhausted,
}

/// 结构 checkpoint（§0.5：整组 refs/version/hash/context/authorization/
/// receipt/review scope/status；跨合法业务轮次继承，不重置预算）。
/// SG4a.0 恒空，仅耐久形状。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralCheckpoint {
    pub checkpoint_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_revision_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_candidate_ir_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure_report_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_scope: Option<String>,
    /// checkpoint 状态；缺字段读 Unknown（无推进权）。
    #[serde(default)]
    pub status: StructuralCheckpointStatus,
    pub recorded_at: String,
}

/// checkpoint 状态；旧/缺字段读 Unknown，不构成推进事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StructuralCheckpointStatus {
    #[default]
    Unknown,
    Active,
    Superseded,
}

/// SG4b：`settle_turn_receipt` 的结果（重放时调用方不得重复外层副作用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuralSettlement {
    /// 本次转移新落账（hold→receipt）。
    Settled,
    /// 同 receipt 重放（零写，幂等成功）。
    Replayed,
}

/// SG4b：结算核心的 typed 失败（由 store 入口映射为既有错误形状，
/// 不在 models 层引入 store 错误依赖）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuralSettlementError {
    /// 已结算 turn 的二次/改判结算（终态互斥，不退款）。
    TerminalConflict,
    /// receipt 与 hold 的 command 身份不一致。
    CommandMismatch,
    /// 无活动 hold（不凭空造账）。
    NoActiveHold,
}

/// SG4b（REQ-SG-04）：合法交付 checkpoint 的确定性身份——同 turn 提交
/// 同三 refs 得同 id（重放幂等），任一 ref 变化即新 checkpoint。持久
/// digest 配方与仓库其余内容寻址惯例一致（sha256 全量字段，前 16 位
/// 十六进制仅展示、不截断身份域）。
pub fn structural_checkpoint_id(
    turn_id: &str,
    source_revision_ref: &str,
    plan_candidate_ir_ref: &str,
    structure_report_ref: &str,
) -> String {
    let canonical = [
        "structural_checkpoint",
        turn_id,
        source_revision_ref,
        plan_candidate_ir_ref,
        structure_report_ref,
    ]
    .join("\0");
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// 已解决结构事实（§0.5：规则谓词＋证据，非固定边值）。历史恢复整组
/// 回绑边界归 SG6；SG4a.0 恒空，仅耐久形状。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedStructureFact {
    /// 规则谓词单源标识（finding code／谓词名，非边值）。
    pub predicate: String,
    /// 证据 refs（source/report/instruction 等耐久引用）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    pub resolved_at: String,
}

/// launch 实际观察证据（设计 v1.1：launch_state/launch_observed_at/
/// launch_evidence_ref）。actual launch 单源仍为 provider_start_ledger
/// ——旧 started 布尔不用于新链证据、不冒充实际 start；缺字段读 Unknown。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralLaunchEvidence {
    #[serde(default)]
    pub launch_state: StructuralLaunchState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_observed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_evidence_ref: Option<String>,
}

/// launch 观察态；缺字段读 Unknown（旧 started 布尔不映射为 Started）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StructuralLaunchState {
    #[default]
    Unknown,
    NotStarted,
    Started,
    Completed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: WorkspaceType,
    pub status: WorkspaceSessionStatus,
    pub author_provider: ProviderName,
    /// C2 Task 5（REQ-CRO-05）reviewer 三值：缺失即 None（旧 JSON 恒有值→Some），
    /// MUST NOT 回填 author。
    pub reviewer_provider: Option<ProviderName>,
    pub review_rounds: u32,
    #[serde(default)]
    pub permission_modes: WorkspaceRolePermissionModes,
    #[serde(default)]
    pub provisional_reviewer_provider: Option<ProviderName>,
    #[serde(default)]
    pub reviewer_enabled_at_start: Option<bool>,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    #[serde(default = "default_flow_kind")]
    pub flow_kind: WorkItemPlanFlowKind,
    #[serde(default = "default_run_policy")]
    pub run_policy: RunPolicy,
    #[serde(default)]
    pub run_history: RunHistory,
    #[serde(default)]
    pub review_invocation_scope: Option<ReviewInvocationScope>,
    #[serde(default)]
    pub human_gate_snapshot: Option<HumanGateSnapshot>,
    #[serde(default)]
    pub repair_reservation: Option<RepairReservation>,
    #[serde(default)]
    pub human_gate_reservation: Option<HumanGateReservation>,
    #[serde(default)]
    pub policy_diagnostics: Vec<PolicyDiagnostic>,
    #[serde(default)]
    pub provider_start_ledger: Vec<ProviderStartLedgerEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub single_candidate_phase: Option<SingleCandidatePhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_plan_source_revision_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_candidate_ir_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mechanical_report_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_provenance_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile_reservation: Option<SingleCandidateCompileReservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sc_recovery_claim: Option<ScRecoveryClaim>,
    #[serde(default)]
    pub work_item_runtime_binding: Option<WorkItemRuntimeBinding>,
    /// C1 Task 8（REQ-C1-CHILD-01）：compile child 创建时原子写入的绑定
    /// 身份；旧 JSON 读 `None`（只读兼容，不迁移、不补写）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_child_binding: Option<ChildBindingIdentity>,
    #[serde(default)]
    pub provider_conversations: Vec<ProviderConversationRef>,
    /// A1a（Task 2.1）：LC launch 指纹耐久事实（session 唯一持久源；engine
    /// 内存 map 仅索引）。旧 JSON 缺失读默认空。
    #[serde(default)]
    pub logical_launch_fingerprints: Vec<LogicalLaunchFingerprintRecord>,
    /// A1a（Task 2.1）：旧 native 会话多代 tombstone（多代保留，不复活）。
    #[serde(default)]
    pub superseded_native_sessions: Vec<SupersededNativeSessionRecord>,
    /// A1a（Task 2.1）：当前 LC 恢复记录（waiting/fresh/resolved 主状态）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lc_resume_recovery: Option<LcResumeRecoveryRecord>,
    /// SG4a.0（§0.6.1）：结构治理唯一耐久聚合状态；旧 JSON 缺失读 None
    /// （不回填、不补授权）。hold 结算/启动行为归 SG4a/4b，runtime 镜像
    /// 与 from_record 接线归 SG7a。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structural_governance_state: Option<StructuralGovernanceState>,
    pub messages: Vec<WorkspaceMessageRecord>,
    pub created_at: String,
    pub updated_at: String,
}

/// C1 Task 8（REQ-C1-CHILD-01）：compile child 精确匹配。`expected` 为
/// `Some` 时要求 session durable child binding 全等——旧代（不同
/// binding_version）或字段漂移的 child 不复用、不迁移、不覆盖；`expected`
/// 为 `None`（manual/非 enrolled 链）只匹配未绑定 child，保持既有
/// entity_id 复用语义。
pub fn match_compile_child(
    session: &WorkspaceSessionRecord,
    expected: Option<&ChildBindingIdentity>,
) -> bool {
    match expected {
        Some(identity) => session.work_item_child_binding.as_ref() == Some(identity),
        None => session.work_item_child_binding.is_none(),
    }
}

/// Durable audit event that associates an immutable stopped auto run with the
/// interactive session created by an explicit human takeover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanGateTakeoverEvent {
    pub id: String,
    pub parent_session_id: String,
    pub child_session_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionSummaryRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: WorkspaceType,
    pub status: WorkspaceSessionStatus,
    pub author_provider: ProviderName,
    /// C2 Task 5：reviewer 三值——与 session record 同形。
    pub reviewer_provider: Option<ProviderName>,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceMessageRecord {
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub name: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub artifact_id: String,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Author,
    Reviewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionEvent {
    pub request_id: String,
    pub request: serde_json::Value,
    pub response: Option<serde_json::Value>,
    pub ts: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDetail {
    pub node_id: String,
    pub session_id: String,
    pub node_type: TimelineNodeType,
    pub status: TimelineNodeStatus,
    pub agent_role: Option<AgentRole>,
    pub provider: Option<ProviderSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    pub messages: Vec<serde_json::Value>,
    pub streaming_content: String,
    pub execution_events: Vec<serde_json::Value>,
    pub permission_events: Vec<PermissionEvent>,
    pub verdict: Option<serde_json::Value>,
    pub artifact_ref: Option<ArtifactRef>,
    pub is_revision: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_feedback: Option<String>,
    pub base_artifact_ref: Option<ArtifactRef>,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderReviewRoundRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub session_id: String,
    pub round_index: u32,
    pub author_provider: ProviderName,
    /// C2 Task 5：reviewer 三值——缺失轮次记 None。
    pub reviewer_provider: Option<ProviderName>,
    pub review_result: String,
    pub revision_result: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectProviderDefaultsRecord {
    pub project_id: String,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_role_permission_modes_default_is_auto() {
        let modes = WorkspaceRolePermissionModes::default();
        assert_eq!(modes.author, ProviderPermissionMode::Auto);
        assert_eq!(modes.reviewer, ProviderPermissionMode::Auto);
    }

    #[test]
    fn old_workspace_session_record_without_permission_modes_deserializes_to_auto() {
        let json = serde_json::json!({
            "id": "s1", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
            "workspace_type": "story", "status": "open",
            "author_provider": "claude_code", "reviewer_provider": "codex",
            "review_rounds": 1, "superpowers_enabled": false, "openspec_enabled": false,
            "messages": [], "created_at": "", "updated_at": ""
        });
        let record: WorkspaceSessionRecord = serde_json::from_value(json).unwrap();
        assert_eq!(record.permission_modes.author, ProviderPermissionMode::Auto);
        assert_eq!(
            record.permission_modes.reviewer,
            ProviderPermissionMode::Auto
        );
    }

    #[test]
    fn old_workspace_session_record_defaults_work_item_plan_policy_fields() {
        let json = serde_json::json!({
            "id": "s1", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
            "workspace_type": "work_item_plan", "status": "open",
            "author_provider": "claude_code", "reviewer_provider": "codex",
            "review_rounds": 1, "superpowers_enabled": false, "openspec_enabled": false,
            "messages": [], "created_at": "", "updated_at": ""
        });

        let record: WorkspaceSessionRecord = serde_json::from_value(json).unwrap();

        assert_eq!(
            record.flow_kind,
            crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy
        );
        assert_eq!(
            record.run_policy,
            crate::product::work_item_plan_policy::RunPolicy::Interactive
        );
        assert_eq!(
            record.run_history,
            crate::product::work_item_plan_policy::RunHistory::default()
        );
    }

    #[test]
    fn workspace_session_status_serializes_new_terminal_values() {
        assert_eq!(
            serde_json::to_value(WorkspaceSessionStatus::StoppedNeedsHuman).unwrap(),
            "stopped_needs_human"
        );
        assert_eq!(
            serde_json::to_value(WorkspaceSessionStatus::Failed).unwrap(),
            "failed"
        );
    }

    // === SG4a（plan v1.1 Task SG4a）：可用预算数学式（REQ-CG-02）===

    fn sg_04a_hold(turn_id: &str) -> UserBudgetHold {
        UserBudgetHold {
            turn_id: turn_id.to_string(),
            command_id: format!("command_{turn_id}"),
            source_hash: String::new(),
            held_at: "2026-10-13T00:00:00Z".to_string(),
        }
    }

    fn sg_04a_receipt_for(turn_id: &str, kind: UserBudgetReceiptKind) -> UserBudgetReceipt {
        UserBudgetReceipt {
            kind,
            turn_id: turn_id.to_string(),
            command_id: format!("command_{turn_id}"),
            attempt_reason: None,
            source_revision_ref: None,
            plan_candidate_ir_ref: None,
            structure_report_ref: None,
            context_digest: None,
            reason: None,
            settled_at: "2026-10-13T01:00:00Z".to_string(),
        }
    }

    #[test]
    fn sg_04a_budget_math_follows_terminal_receipts_and_active_holds() {
        let mut state = StructuralGovernanceState::default();
        assert_eq!(state.user_budget_total, 3, "总额沿现 gate 默认 3");
        assert_eq!(state.available_user_budget(), 3);

        // 活动 hold 立即占用：3 → 2，且不是已交付。
        state.user_holds.push(sg_04a_hold("turn_0001"));
        assert_eq!(state.active_hold_count(), 1);
        assert_eq!(state.available_user_budget(), 2);
        assert_eq!(state.delivered_charges(), 0);

        // delivered_charge 计费：2 → 1。
        state.user_receipts.push(sg_04a_receipt_for(
            "turn_0000",
            UserBudgetReceiptKind::DeliveredCharge,
        ));
        assert_eq!(state.delivered_charges(), 1);
        assert_eq!(state.charged_receipts(), 1);
        assert_eq!(state.available_user_budget(), 1);

        // nonstructural_failure_charge 计费不退：1 → 0。
        state.user_receipts.push(sg_04a_receipt_for(
            "turn_0002",
            UserBudgetReceiptKind::NonstructuralFailureCharge,
        ));
        assert_eq!(state.nonstructural_failure_charges(), 1);
        assert_eq!(state.charged_receipts(), 2);
        assert_eq!(state.available_user_budget(), 0);

        // 超扣不产生负数（saturating）。
        state.user_holds.push(sg_04a_hold("turn_0003"));
        assert_eq!(state.available_user_budget(), 0);

        // released_structural 不计费：release 后活动 hold 释放、可用恢复。
        state.user_receipts.push(sg_04a_receipt_for(
            "turn_0004",
            UserBudgetReceiptKind::ReleasedStructural,
        ));
        assert_eq!(state.charged_receipts(), 2, "release 不进计费额");
        state.user_holds.clear();
        assert_eq!(state.available_user_budget(), 1);

        // 按 turn 幂等查询。
        assert_eq!(
            state
                .receipt_for_turn("turn_0000")
                .map(|receipt| receipt.kind),
            Some(UserBudgetReceiptKind::DeliveredCharge)
        );
        assert!(state.receipt_for_turn("turn_missing").is_none());
        assert!(state.active_hold_for_turn("turn_0001").is_none());

        // roundtrip：receipt.reason 缺省不生长字段（skip_none 字节合同）。
        let encoded = serde_json::to_value(&state).unwrap();
        assert!(
            encoded["user_receipts"][0]
                .as_object()
                .unwrap()
                .get("reason")
                .is_none()
        );
        let decoded: StructuralGovernanceState = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, state);
    }
}
