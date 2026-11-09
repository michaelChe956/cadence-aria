//! 知识库 operation 的数据模型：kind / 步骤 / 状态与 operation_id 派生。
//! 字段与序列化形态由 LC 知识库模块实施计划 Task 1 固定。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::product::logical_codebase::aggregate_initialization::{
    AggregateInitializationErrorRecord, AggregateProviderSelection,
};

/// operation 级别：首次建立（Bootstrap）或增量更新（Update）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeBaseOperationKind {
    Bootstrap,
    Update,
}

/// 步骤标识：Bootstrap 流程的单步，或 Update 流程的应用变更包步骤。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeBaseStepKind {
    Bootstrap,
    ApplyUpdate,
}

/// operation 生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeBaseOperationStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// 步骤状态：复用 `AggregateInitializationStepStatus` 的四态语义
/// （该枚举第五态 `Skipped` 属于聚合初始化特有判定，知识库不使用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeBaseStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// 触发本次知识库 operation 的输入。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeBaseOperationInput {
    pub idempotency_key: String,
    /// provider wire 名（pi / claude_code / codex）。
    pub provider: String,
    /// update 专用：相对聚合根的变更包目录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
}

/// 步骤执行记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeBaseStepRecord {
    pub step_id: KnowledgeBaseStepKind,
    pub status: KnowledgeBaseStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_summary: Option<String>,
}

/// 一次知识库 bootstrap / update 的持久化记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeBaseOperation {
    pub operation_id: String,
    pub project_id: String,
    pub logical_codebase_id: String,
    pub kind: KnowledgeBaseOperationKind,
    pub input: KnowledgeBaseOperationInput,
    pub status: KnowledgeBaseOperationStatus,
    pub steps: Vec<KnowledgeBaseStepRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_selection: Option<AggregateProviderSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AggregateInitializationErrorRecord>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// operation_id 派生：`sha256("{project_id}:{lc_id}:{idempotency_key}")`
/// 前 16 位小写 hex。LC 身份参与派生，保证同一 idempotency_key 在不同
/// LC 下产生不同的 operation_id。
pub fn derive_operation_id(project_id: &str, lc_id: &str, idempotency_key: &str) -> String {
    let payload = format!("{project_id}:{lc_id}:{idempotency_key}");
    let digest = format!("{:x}", Sha256::digest(payload.as_bytes()));
    digest[..16].to_string()
}
