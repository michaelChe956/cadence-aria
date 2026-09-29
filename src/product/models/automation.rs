//! WIG Autopilot enrollment 持久模型（P0 1.2，REQ-WIGA-01/02/08）。
//!
//! enrollment 是 issue 级独立 durable 记录：默认缺失（off），不挂在
//! `IssueRecord` 上；`policy_revision` 单调递增，CAS 修订线性化，关闭后
//! 未认领许可即失效。P0 只承载授权事实，不启动任何 provider。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::EnrollmentTarget;
use crate::product::models::lifecycle::IssueWorkItemPlanOptions;
use crate::product::models::provider::ProviderName;

/// 精确 source 引用：id + 确认时的 current_version，禁止回退 latest 猜测。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRevisionRef {
    pub id: String,
    pub version: u32,
}

/// enrollment 绑定的精确 story/design 身份集合（REQ-WIGA-01）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentSource {
    #[serde(default)]
    pub stories: Vec<SourceRevisionRef>,
    #[serde(default)]
    pub designs: Vec<SourceRevisionRef>,
}

/// enrollment 携带的 provider/计划选项快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentOptions {
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    #[serde(default)]
    pub review_rounds: u32,
    #[serde(default)]
    pub superpowers_enabled: bool,
    #[serde(default)]
    pub openspec_enabled: bool,
    pub plan_options: IssueWorkItemPlanOptions,
}

/// issue 级自动化授权的 durable 记录；`policy_revision` 0 不落盘（首个有效值为 1）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueAutomationEnrollment {
    pub enrollment_id: String,
    pub selection_key: String,
    pub project_id: String,
    pub issue_id: String,
    pub enabled: bool,
    pub policy_revision: u64,
    pub source: EnrollmentSource,
    pub options: EnrollmentOptions,
    /// C5 Task 1：enable 时显式声明的双载体 target。旧 JSON 缺字段读
    /// `None`——旧代无绑定，自动链经 `load_current_enrollment_binding`
    /// fail-closed，绝不从冗余字段猜测 `EnrollmentTarget`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<EnrollmentTarget>,
    /// 首启意图身份：与 `enrollment_id` 同源生成，P0 不消费。
    pub prepare_intent_id: String,
    pub plan_id: Option<String>,
    pub session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// C1 Task 1：版本化绑定历史；仅显式 rebind/换代追加，旧 JSON 读 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_history: Option<EnrollmentBindingHistory>,
    /// C1 Task 1：命令账本（同 command 同 payload 重放/异 payload 拒绝）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_ledger: Vec<EnrollmentCommandResult>,
}

/// 自动化归属：仅 server 端 enrollment 可代表授权（REQ-WIGA-08）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationOwner {
    Client,
    Server,
}

/// 从 durable enrollment 事实计算的会话归属投影（Task 4 消费）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationOwnership {
    pub owner: AutomationOwner,
    pub enrollment_id: Option<String>,
    pub policy_revision: Option<u64>,
    pub enabled: bool,
}

impl AutomationOwnership {
    /// 无 durable enrollment 事实或缺省投影（client/manual）；引擎独立 fixture
    /// 与测试使用——对外出口一律由 manager 以 durable 事实二次覆盖。
    pub fn client_default() -> Self {
        Self {
            owner: AutomationOwner::Client,
            enrollment_id: None,
            policy_revision: None,
            enabled: false,
        }
    }
}

/// enrollment 写入命令：Enable 携带完整授权 payload；Disable 仅关停。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EnrollmentWriteCommand {
    /// C5 Task 1（BREAKING）：`EnrollmentTarget` 是 enrollment 载体身份的
    /// 唯一权威——冗余的顶层 `logical_repository_id` 已删除；缺 `target`
    /// 的请求在反序列化层即被拒绝（HTTP 422、零写入）。
    Enable {
        selection_key: String,
        source: EnrollmentSource,
        options: EnrollmentOptions,
        target: EnrollmentTarget,
    },
    Disable,
}

/// C1 Task 1：用户显式 rebind 提交的新代绑定身份输入（wire DTO）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentBindingIdentityInput {
    pub plan_id: String,
    pub session_id: String,
    pub source: EnrollmentSource,
    pub target: EnrollmentTarget,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
}

/// C1 Task 1：durable 绑定身份——当前代或历史代的完整授权事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentBindingIdentity {
    pub binding_version: u64,
    pub enrollment_id: String,
    pub plan_id: String,
    pub session_id: String,
    pub source: EnrollmentSource,
    pub target: EnrollmentTarget,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
}

/// C1 Task 1：版本化绑定历史。`binding_version` 只在显式 rebind/换代时
/// 递增；`previous` 只追加、不改写（旧代只读可追溯）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentBindingHistory {
    pub current: EnrollmentBindingIdentity,
    pub previous: Vec<EnrollmentBindingIdentity>,
}

/// C1 统一操作结果状态（Task 6/7/9 的 lease takeover、advance retry、
/// Cockpit 动作复用同一状态机，不另造 Generation/RecoveryOperation）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Accepted,
    Replayed,
    NeedsHuman,
    Rejected,
}

/// C1 Task 1：enrollment 命令账目——同 `command_id` 同 payload 幂等重放
/// 首次 durable 结果，异 payload fail-closed。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentCommandResult {
    pub command_id: String,
    pub payload_digest: String,
    pub state: OperationState,
    pub binding_version: u64,
}

/// C1 Task 1：显式重绑/换代请求（REQ-WIGA-01）。用户必须明确提供新代
/// plan/session/source/target/provider 并携带当前 expected 版本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentRebindRequest {
    pub command_id: String,
    pub expected_policy_revision: u64,
    pub expected_binding_version: u64,
    pub binding: EnrollmentBindingIdentityInput,
    pub reason: String,
}

impl EnrollmentRebindRequest {
    /// 稳定 payload 摘要：同一 command 的异 payload 必然产生不同 digest。
    pub fn payload_digest(&self) -> String {
        let payload =
            serde_json::to_string(self).expect("rebind request payload is serializable");
        format!("sha256:{:x}", Sha256::digest(payload.as_bytes()))
    }
}

/// C1 Task 1：rebind 结果。`state=Replayed` 时返回首次 durable 结果的
/// 当前投影（enrollment 事实不被重放改变）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentRebindResult {
    pub command_id: String,
    pub state: OperationState,
    pub enrollment: IssueAutomationEnrollment,
}

// ─── C1 Task 6：租约三态与确认接管（REQ-WIGA-03）───

/// 租约三态判定。只从现有 lock record、attempt terminal/activity/status
/// 证据分类；无法证明活跃或死亡时一律 `UnknownNeedsHuman`（绝不抢占）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseDisposition {
    ActiveWait,
    DeadNeedsTakeover,
    UnknownNeedsHuman,
}

/// 三态判定 + durable 证据（owner/attempt status/释放事实）。证据只读，
/// 判定不写任何文件；`lease_id` 是判定时锁的当前 owner id。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseDecision {
    pub disposition: LeaseDisposition,
    pub lease_id: String,
    pub last_activity_at: Option<String>,
    pub evidence: Vec<String>,
}

/// C1 Task 6：死亡租约确认接管请求。携带稳定 `command_id`、当前
/// enrollment binding（过期/异载体拒绝）与判定时的 lease/attempt 身份；
/// 接管必须由用户显式确认，系统绝不自动 takeover。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseTakeoverRequest {
    pub command_id: String,
    pub expected_binding: EnrollmentBindingIdentity,
    pub expected_lease_id: String,
    pub expected_attempt_id: String,
}

impl LeaseTakeoverRequest {
    /// 稳定 payload 摘要：同一 command 的异 payload 必然产生不同 digest
    /// （命令账本按此 fail-closed）。
    pub fn payload_digest(&self) -> String {
        let payload =
            serde_json::to_string(self).expect("lease takeover request payload is serializable");
        format!("sha256:{:x}", Sha256::digest(payload.as_bytes()))
    }
}

/// C1 Task 6：接管结果。`state=NeedsHuman` 表示证据不足以证明死亡（不
/// 接管）；`Rejected` 表示租约仍活跃（只等待）；`Accepted` 表示死亡 owner
/// 已在既有 worktree 文件锁内原子清出；`Replayed` 返回同 command 首次
/// durable 结果的当前投影。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseTakeoverResult {
    pub command_id: String,
    pub state: OperationState,
    pub lease: LeaseDecision,
}

// ─── C1 Task 7：Failed advance 显式 retry-initialization（REQ-ADV-C1-RETRY）───

/// 显式 retry 请求：独立产品动作（不是重复普通 advance）。携带当前
/// enrollment binding、失败时的 attempt/checkpoint 身份与未知副作用确认
/// 标志；checkpoint/target/binding 过期一律拒绝且 durable 不变。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryInitializationRequest {
    pub command_id: String,
    pub expected_binding: EnrollmentBindingIdentity,
    pub expected_attempt_id: String,
    pub expected_checkpoint: crate::product::advance_store::AdvanceInitializationPhase,
    pub confirm_unknown_side_effect: bool,
}

impl RetryInitializationRequest {
    /// 稳定 payload 摘要（命令账本按此判同 command 异 payload）。
    pub fn payload_digest(&self) -> String {
        let payload =
            serde_json::to_string(self).expect("retry initialization request is serializable");
        format!("sha256:{:x}", Sha256::digest(payload.as_bytes()))
    }
}

/// 独立 durable retry 事实：不改写原 Failed record/journal 的失败审计；
/// `checkpoint` 是发起 retry 时锁定的续做起点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryInitializationRecord {
    pub retry_id: String,
    pub command_id: String,
    pub advance_id: String,
    pub attempt_id: String,
    pub state: OperationState,
    pub checkpoint: crate::product::advance_store::AdvanceInitializationPhase,
    pub created_at: String,
    pub updated_at: String,
}

/// retry 固定响应体；`NeedsHuman` 由 `state` 承载（此时 `outcome=None`，
/// 不重跑任何步骤）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryInitializationResult {
    pub command_id: String,
    pub state: OperationState,
    pub retry: RetryInitializationRecord,
    pub outcome: Option<crate::product::advance_store::AdvanceOutcome>,
}

/// P1 WIGA Task 4：enrollment-bound 不可变创建意图（automation-plan-intent.json）。
///
/// 先于 plan/session 持久化、与 enrollment 同源（`prepare_intent_id`），冻结
/// source/options/target 与稳定 plan/session id；重开换 payload 后当前
/// enrollment 派生的新意图与已存文件不一致即 fail-closed，绝不覆盖。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedPlanIntent {
    pub enrollment_id: String,
    pub prepare_intent_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub source: EnrollmentSource,
    pub options: EnrollmentOptions,
    /// C5 Task 1：冻结载体身份唯一来自 enrollment 声明的 target；旧意图
    /// 文件（仅含已废弃 `logical_repository_id`、无 `target`）经显式
    /// legacy reader 读为「旧代无绑定」，调用者 fail-closed。
    pub target: EnrollmentTarget,
    pub plan_id: String,
    pub session_id: String,
}

impl PreparedPlanIntent {
    /// 从当前 enrollment 派生冻结意图；稳定目标 id 只来自已持久的
    /// `prepare_intent_id`，不依赖扫描最近 plan。同 payload 重复派生幂等。
    /// C5 Task 1：target 由调用方从 `enrollment.target` 显式传入（旧代
    /// enrollment 无 target，调用者须先 fail-closed）。
    pub fn from_enrollment(
        enrollment: &IssueAutomationEnrollment,
        target: &EnrollmentTarget,
    ) -> Self {
        Self {
            enrollment_id: enrollment.enrollment_id.clone(),
            prepare_intent_id: enrollment.prepare_intent_id.clone(),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            source: enrollment.source.clone(),
            options: enrollment.options.clone(),
            target: target.clone(),
            plan_id: format!("issue_work_item_plan_auto_{}", enrollment.prepare_intent_id),
            session_id: format!("workspace_session_auto_{}", enrollment.prepare_intent_id),
        }
    }
}

/// P1 WIGA Task 5：plan 生成动作检查点（automation-generation-intent.json）。
///
/// 稳定动作键 `enrollment_id:plan_id:start_generation`；冻结 source/options/
/// target/session。`Claimed` 尚无 node/ledger 可安全重启；`EngineStarted`/
/// `ProviderDispatched` 之后进程重建无法证明外部 provider 未被触达时停在
/// `NeedsHuman`，只供人显式恢复，绝不宣称 exactly-once。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanGenerationIntent {
    pub enrollment_id: String,
    pub plan_id: String,
    pub session_id: String,
    pub action_key: String,
    pub source: EnrollmentSource,
    pub options: EnrollmentOptions,
    /// C5 Task 1：冻结载体身份唯一来自 enrollment 声明的 target；
    /// `same_identity` 以 target 全等参与漂移判定。
    pub target: EnrollmentTarget,
    pub phase: PlanGenerationPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanGenerationPhase {
    Claimed,
    EngineStarted,
    ProviderDispatched,
    NeedsHuman,
}

impl PlanGenerationIntent {
    /// 冻结身份字段（不含 phase）是否一致。
    pub fn same_identity(&self, other: &PlanGenerationIntent) -> bool {
        self.enrollment_id == other.enrollment_id
            && self.plan_id == other.plan_id
            && self.session_id == other.session_id
            && self.action_key == other.action_key
            && self.source == other.source
            && self.options == other.options
            && self.target == other.target
    }

    pub fn action_key_for(enrollment_id: &str, plan_id: &str) -> String {
        format!("{enrollment_id}:{plan_id}:start_generation")
    }
}

/// enrollment 写入失败语义：HTTP 层按 Conflict→409 / InvalidScope→422 /
/// NotFound→404 / Store→fail-closed 其余映射。
#[derive(Debug, thiserror::Error)]
pub enum EnrollmentError {
    #[error("automation_enrollment_conflict: current_revision={current_revision:?}")]
    Conflict { current_revision: Option<u64> },
    #[error("automation_enrollment_invalid_scope: {0}")]
    InvalidScope(String),
    #[error("automation_enrollment_not_found")]
    NotFound,
    #[error(transparent)]
    Store(#[from] ProductStoreError),
}
