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

/// enrollment 写入失败语义：HTTP 层按 Conflict→409 / InvalidScope→422 /
/// NotFound→404 / Store→fail-closed 其余映射。
#[derive(Debug, thiserror::Error)]
pub enum EnrollmentError {
    #[error("automation_enrollment_conflict: current_revision={current_revision:?}")]
    Conflict {
        current_revision: Option<u64>,
    },
    #[error("automation_enrollment_invalid_scope: {0}")]
    InvalidScope(String),
    #[error("automation_enrollment_not_found")]
    NotFound,
    #[error(transparent)]
    Store(#[from] ProductStoreError),
}
