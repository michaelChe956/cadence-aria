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

/// REQ-WSC-18（owner 2026-10-31 真机反馈「点了之后会报错，但是我看不到报错
/// 内容」）：一次 **当前候选** Final Compile 失败的 durable 事实。
///
/// 现场此前只有 timeline node 的 `summary`（`Final Compile 失败：[code] ...`）
/// 与内存 stash：门卡快照 findings 恒空、会话 `policy_diagnostics` 为空，
/// owner 在门卡上读不到任何一点原因。本记录与门卡同族落点
/// （`workspace-sessions/{session}/`）durable 落盘，`message` 为完整人话、
/// `detail` 为完整诊断原文（不截断），刷新/重连后仍可读。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCompileFailure {
    /// 内容寻址失败身份（`planning_compile_failure_<sha256>`）——同失败幂等同 id。
    pub failure_id: String,
    /// 稳定诊断码（如 `final_compile_prepare_failed`）。
    pub code: String,
    /// 面向 owner 的人话原因（完整，不截断；不从错误码反推）。
    pub message: String,
    /// 诊断原文（完整：failure_id + compile run 身份 + typed 渲染）。
    pub detail: String,
    /// 落盘时间（RFC3339 UTC）。
    pub recorded_at: String,
}

/// REQ-WSC-21：候选编译类失败的自动重写阶段（owner 2026-10-31 裁定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateAutoRewritePhase {
    /// 首次候选无效失败即进入：平台已按剩余预算自动让 author 重写。
    Running,
    /// 同一依赖环（结构指纹相同）重复出现，或预算用尽：提前停止自动重写，
    /// 升级提示并交人工门。
    Stopped,
}

/// REQ-WSC-21/22：平台侧自动重写事实（前端只呈现，不自算、不猜）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateAutoRewrite {
    pub phase: CandidateAutoRewritePhase,
    /// `Running`：还剩几次自动重写（平台权威值）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<u32>,
    /// 同一依赖环结构指纹已重复出现的次数（`Stopped` 的主要依据）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeated: Option<u32>,
}

/// REQ-WSC-18：某会话**最近一次** SC Final Compile 尝试的候选绑定与结果。
///
/// 单文件覆盖写（`candidate-compile-status.json`）：成功尝试把 `failure`
/// 置空——否则一次瞬态失败成功后仍会残留「当前候选编译不过」，把确认动作
/// 永久挡死（新的死路）。判断「是否当前候选」只认本记录的候选绑定与会话
/// durable refs 逐字相等（两侧都必须是 Some，缺一即无法证明→不构成阻断）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCompileStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_revision_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_candidate_ir_ref: Option<String>,
    /// 本次尝试的失败事实；`None`＝本次尝试没有可呈现的失败（编译成功）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<CandidateCompileFailure>,
    /// REQ-WSC-22：依赖环的**结构**指纹（成员集合 + 边集合规范化摘要）——
    /// 跨尝试稳定，不随文案/行号/编译 id 变化；非环失败为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_fingerprint: Option<String>,
    /// 同一结构指纹已连续出现的次数（首次＝1）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_count: Option<u32>,
    /// 已用掉的自动重写次数（REQ-WSC-25 **诊断计数**，**不是**授权余额）。
    /// 授权余额只有一个权威事实 = 预算域净余额（`author_rewrite_budget`）；
    /// 本计数仅供可观测与遗留侧归并，不得被任何执法点或呈现点当作余额。
    #[serde(default)]
    pub auto_rewrites_used: u32,
    /// REQ-WSC-21/22：自动重写阶段事实；无自动重写语义（非候选无效类）为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_rewrite: Option<CandidateAutoRewrite>,
    /// 落盘时间（RFC3339 UTC）。
    pub recorded_at: String,
}

impl CandidateCompileStatus {
    /// 本记录是否描述了「给定候选」（source revision + candidate IR 双侧
    /// 逐字相等）。任一侧缺席即无法证明同属一个候选，返回 false——调用方
    /// 据此 fail-open（不凭猜测拦确认）。
    pub fn belongs_to_candidate(
        &self,
        source_revision_ref: Option<&str>,
        plan_candidate_ir_ref: Option<&str>,
    ) -> bool {
        match (
            self.source_revision_ref.as_deref(),
            source_revision_ref,
            self.plan_candidate_ir_ref.as_deref(),
            plan_candidate_ir_ref,
        ) {
            (Some(status_source), Some(current_source), Some(status_ir), Some(current_ir)) => {
                status_source == current_source && status_ir == current_ir
            }
            _ => false,
        }
    }

    /// 给定会话当前候选时，该候选是否**确定编译不过**；是则给出失败事实。
    pub fn current_failure(
        &self,
        source_revision_ref: Option<&str>,
        plan_candidate_ir_ref: Option<&str>,
    ) -> Option<&CandidateCompileFailure> {
        if !self.belongs_to_candidate(source_revision_ref, plan_candidate_ir_ref) {
            return None;
        }
        self.failure.as_ref()
    }
}
