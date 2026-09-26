//! WIG Autopilot enrollment 持久模型（P0 1.2，REQ-WIGA-01/02/08）。
//!
//! enrollment 是 issue 级独立 durable 记录：默认缺失（off），不挂在
//! `IssueRecord` 上；`policy_revision` 单调递增，CAS 修订线性化，关闭后
//! 未认领许可即失效。P0 只承载授权事实，不启动任何 provider。

use serde::{Deserialize, Serialize};

use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::LogicalRepositoryId;
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
    pub logical_repository_id: LogicalRepositoryId,
    /// 首启意图身份：与 `enrollment_id` 同源生成，P0 不消费。
    pub prepare_intent_id: String,
    pub plan_id: Option<String>,
    pub session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
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
    Enable {
        selection_key: String,
        source: EnrollmentSource,
        options: EnrollmentOptions,
        logical_repository_id: LogicalRepositoryId,
    },
    Disable,
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
    pub logical_repository_id: LogicalRepositoryId,
    pub plan_id: String,
    pub session_id: String,
}

impl PreparedPlanIntent {
    /// 从当前 enrollment 派生冻结意图；稳定目标 id 只来自已持久的
    /// `prepare_intent_id`，不依赖扫描最近 plan。同 payload 重复派生幂等。
    pub fn from_enrollment(enrollment: &IssueAutomationEnrollment) -> Self {
        Self {
            enrollment_id: enrollment.enrollment_id.clone(),
            prepare_intent_id: enrollment.prepare_intent_id.clone(),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            source: enrollment.source.clone(),
            options: enrollment.options.clone(),
            logical_repository_id: enrollment.logical_repository_id,
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
    pub logical_repository_id: LogicalRepositoryId,
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
            && self.logical_repository_id == other.logical_repository_id
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
