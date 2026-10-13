use serde::{Deserialize, Serialize};

/// Durable lifecycle state for one human-gate feedback turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanGateTurnStatus {
    Reserved,
    Running,
    Completed,
    Failed,
}

/// Terminal failure classification for a human-gate feedback turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanGateTurnFailureClass {
    ProviderErr,
    ValidationReject,
    Timeout,
    BudgetExhausted,
    /// 蒸发链顺手项（取消臂回显化）：run 被 supersede/断连取消、drive 被
    /// drop——turn 不会自然收口，由取消臂终态化为 Failed(cancelled)。
    Cancelled,
}

/// Durable record for one accepted human-gate feedback command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanGateTurn {
    pub turn_id: String,
    pub session_id: String,
    pub command_id: String,
    pub feedback_text: String,
    pub status: HumanGateTurnStatus,
    pub attempt_no: u32,
    pub budget_reserved: u32,
    /// SHA-256 of the candidate that was current when this turn was reserved.
    /// Empty values are accepted only for backwards-compatible legacy JSON;
    /// recovery must not auto-complete such a turn.
    #[serde(default)]
    pub source_hash: String,
    #[serde(deserialize_with = "super::deserialize_required_option")]
    pub result_artifact_ref: Option<String>,
    #[serde(deserialize_with = "super::deserialize_required_option")]
    pub failure_class: Option<HumanGateTurnFailureClass>,
    pub created_at: String,
    pub updated_at: String,
}

/// The reservation committed alongside the session budget decrement and
/// provider-start ledger entry. It is retained on the session as the command's
/// idempotency anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanGateReservation {
    pub command_id: String,
    pub turn_id: String,
    pub provider_start_idempotency_key: String,
    pub reserved_at: String,
}

/// SG4a.0（plan v1.1 §0.5/§0.6.1）：用户预算 hold——接受真实反馈时占用
/// 一轮可用预算的耐久锚（按 turn 幂等）。本包只落耐久形状：旧记录恒不
/// 产生，reserve/settle 行为归 SG4a。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserBudgetHold {
    pub turn_id: String,
    pub command_id: String,
    /// hold 冻结时候选 source hash（对齐 HumanGateTurn.source_hash 语义；
    /// 空值仅旧 JSON 兼容）。
    #[serde(default)]
    pub source_hash: String,
    pub held_at: String,
}

/// 三终态用户预算 receipt（REQ-CG-02/05：按 turn 幂等互斥、恰一次）。
/// 本包只落耐久形状，结算逻辑归 SG4a。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserBudgetReceipt {
    pub kind: UserBudgetReceiptKind,
    pub turn_id: String,
    pub command_id: String,
    /// attempt 归因（REQ-CG-05：attempt reason 同 session CAS 绑定）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_reason: Option<StructuralAttemptReason>,
    /// 结算绑定的同代 refs/digest（诊断聚合，不另立第二候选权威）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_revision_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_candidate_ir_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure_report_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_digest: Option<String>,
    pub settled_at: String,
}

/// 三终态 receipt 种类（REQ-CG-02：可用预算 = 总额 − delivered_charge −
/// nonstructural_failure_charge − 活动 hold；仅 typed
/// structural_exhausted_before_delivery 释放为 released_structural）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserBudgetReceiptKind {
    DeliveredCharge,
    NonstructuralFailureCharge,
    ReleasedStructural,
}

/// 结构 attempt 归因（§0.5：HumanRevision/MechanicalRecovery/
/// StructuralRepair/ExplicitFresh）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralAttemptReason {
    HumanRevision,
    MechanicalRecovery,
    StructuralRepair,
    ExplicitFresh,
}

/// 预算分层（§0.5：user/structural/mechanical 事件分层可重建）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetLayer {
    User,
    Structural,
    Mechanical,
}
