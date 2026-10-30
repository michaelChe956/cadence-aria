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

/// SG4a.0/SG4a（plan v1.1 §0.5/§0.6.1）：用户预算 hold——接受真实反馈时
/// 由 reserve 与 session CAS 原子落盘（按 turn 幂等，见
/// `LifecycleStore::compare_and_reserve_human_gate_turn`）。
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
/// settle 由 `LifecycleStore::settle_structural_user_budget` 与 session
/// CAS 同权威落盘。
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
    /// SG4a（REQ-CG-02）：结算精确 reason（provider_err/timeout/cancelled/
    /// structural_exhausted_before_delivery 等 typed 文本；呈现/审计用，
    /// 不是第二计数源）。旧 JSON 缺失读 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 结算落盘时间（RFC3339 UTC）。
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

/// REQ-WSC-13（owner 2026-10-30 裁定，取舍 2=A）：owner 对某条 finding 的
/// 处置方式。当前唯一生产值 `Excluded`（门卡「排除此项」）；后续 owner
/// 需要新语义时按 wire 兼容纪律追加变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanFindingDispositionKind {
    Excluded,
}

/// REQ-WSC-13：durable「已处置事实清单」条目——owner 已裁定不必再处理的
/// 审查事实。`finding_key` 是平台按 finding 内容算的稳定身份；`message_summary`
/// 是该 finding 的 message 摘要（供下一轮 reviewer 与界面复述，不作身份）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanDispositionRecord {
    pub finding_key: String,
    pub message_summary: String,
    pub disposition: HumanFindingDispositionKind,
    /// 处置落盘时间（RFC3339 UTC）。
    pub recorded_at: String,
}

impl HumanDispositionRecord {
    /// finding_key 形态：16 位小写 hex（REQ-WSC-13 算法输出）。
    pub fn is_valid_finding_key(value: &str) -> bool {
        value.len() == 16
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }
}

/// REQ-WSC-13/14（owner 反馈「用户不知道如何往下继续」）：门 turn 以
/// `validation_reject` 失败时的可操作信息——被拒反馈原文、可读原因、下一步
/// 动作与逐字编译/校验诊断。durable 落盘（重启后可读），并按同族落点与
/// 门 turn 记录并列。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanGateTurnRejection {
    pub turn_id: String,
    /// 被拒的是哪一条反馈（原文逐字）。
    pub rejected_feedback: String,
    /// 可读原因（人话，不是错误码）。
    pub reason: String,
    /// 下一步动作（至少一条人话）。
    #[serde(default)]
    pub next_actions: Vec<String>,
    /// 编译/校验诊断（`compile markdown source` / `validate plan candidate
    /// IR` 的 diagnostics，逐字，不加工）。
    #[serde(default)]
    pub diagnostics: Vec<String>,
    /// 落盘时间（RFC3339 UTC）。
    pub recorded_at: String,
}
