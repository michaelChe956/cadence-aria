use std::path::PathBuf;

use uuid::Uuid;

use serde::{Deserialize, Serialize};

use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};

use super::project::{IssueSharedWorktreeStatus, WorkItemStatus};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleConfirmationStatus {
    Draft,
    InReview,
    Confirmed,
    ChangeRequested,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanStatus {
    NotStarted,
    Draft,
    Confirmed,
    ChangeRequested,
}

impl WorkItemPlanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkItemPlanStatus::NotStarted => "not_started",
            WorkItemPlanStatus::Draft => "draft",
            WorkItemPlanStatus::Confirmed => "confirmed",
            WorkItemPlanStatus::ChangeRequested => "change_requested",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemKind {
    Backend,
    Frontend,
    Integration,
    E2e,
    Docs,
    Infra,
    #[default]
    Other,
}

impl WorkItemKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Frontend => "frontend",
            Self::Integration => "integration",
            Self::E2e => "e2e",
            Self::Docs => "docs",
            Self::Infra => "infra",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemExecutionPlanStatus {
    #[default]
    NotStarted,
    Draft,
    Confirmed,
    ChangeRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemContextBudget {
    pub target_context_k: String,
    pub max_summary_chars: usize,
    pub max_code_context_chars: usize,
    pub max_context_file_refs: usize,
    pub max_traceability_refs: usize,
}

impl Default for WorkItemContextBudget {
    fn default() -> Self {
        Self {
            target_context_k: "30-50".to_string(),
            max_summary_chars: 20_000,
            max_code_context_chars: 30_000,
            max_context_file_refs: 80,
            max_traceability_refs: 40,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StorySpecRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_codebase_ref: Option<Uuid>,
    #[serde(default)]
    pub involved_repository_ids: Vec<LogicalRepositoryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_repository_id: Option<LogicalRepositoryId>,
    pub current_version: Option<u32>,
    pub confirmation_status: LifecycleConfirmationStatus,
    /// D1.4（REQ-DEL-04）：单调「曾确凿确认」标记。新创建 Some(false)；任何
    /// 确凿确认/发布成功在原 write 置 Some(true)；回退/restore 单调合并 true，
    /// 不改原确认 CAS。旧记录 None＝历史未知（删除分类保守要求确认），
    /// 再 serialize 不补字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ever_confirmed: Option<bool>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DesignSpecRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub story_spec_ids: Vec<String>,
    pub title: String,
    /// 聚合代码库视野：逻辑代码库 manifest 引用。`None` 表示传统单仓 issue（向后兼容）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_codebase_ref: Option<Uuid>,
    /// AI 明确声明的涉及仓库（聚合视野，必须 ⊆ effective_member_ids）。
    #[serde(default)]
    pub involved_repository_ids: Vec<LogicalRepositoryId>,
    /// AI 显式给出的改动顺序图（执行顺序，非服务调用图，REQ-TGT-04）。Draft 可暂时
    /// 为空；Confirmed 时逻辑代码库多仓 Design（involved > 1）必须提供。WorkItem 编译时
    /// 若 Design 有 change_order 则作为 depends_on 依据（Task 9）。
    #[serde(default)]
    pub change_order: Vec<LogicalRepositoryId>,
    pub current_version: Option<u32>,
    pub confirmation_status: LifecycleConfirmationStatus,
    /// D1.4（REQ-DEL-04）：同 [`StorySpecRecord::ever_confirmed`]（单调曾确认标记）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ever_confirmed: Option<bool>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LifecycleWorkItemRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_repository_id: Option<LogicalRepositoryId>,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub title: String,
    pub plan_status: WorkItemPlanStatus,
    pub execution_status: WorkItemStatus,
    pub worktree_path: Option<PathBuf>,
    #[serde(default)]
    pub work_item_set_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_work_item_plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_outline_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_draft_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_implementation_context: Option<String>,
    #[serde(default)]
    pub kind: WorkItemKind,
    #[serde(default)]
    pub sequence_hint: Option<u32>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub exclusive_write_scopes: Vec<String>,
    #[serde(default)]
    pub forbidden_write_scopes: Vec<String>,
    #[serde(default)]
    pub context_budget: WorkItemContextBudget,
    #[serde(default)]
    pub verification_plan_ref: Option<String>,
    #[serde(default)]
    pub require_execution_plan_confirm: bool,
    #[serde(default)]
    pub execution_plan_status: WorkItemExecutionPlanStatus,
    #[serde(default)]
    pub completion_commit: Option<String>,
    #[serde(default)]
    pub completion_diff_summary_ref: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueWorkItemPlanStatus {
    Draft,
    Confirmed,
    ChangeRequested,
}

impl IssueWorkItemPlanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Confirmed => "confirmed",
            Self::ChangeRequested => "change_requested",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueSharedWorktree {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_repository_id: Option<LogicalRepositoryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_id: Option<RepositoryCheckoutId>,
    #[serde(default)]
    pub path_schema_version: u16,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub base_branch: String,
    pub status: IssueSharedWorktreeStatus,
    pub current_active_work_item_id: Option<String>,
    #[serde(deserialize_with = "super::deserialize_required_option")]
    pub current_lock_owner_id: Option<String>,
    pub last_completed_work_item_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemPlanOptions {
    pub include_integration_tests: bool,
    pub include_e2e_tests: bool,
    pub force_frontend_backend_split: bool,
    pub require_execution_plan_confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemDependencyEdge {
    pub from_work_item_id: String,
    pub to_work_item_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemSplitFindingSeverity {
    Error,
    Warning,
}

impl WorkItemSplitFindingSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemSplitFinding {
    pub severity: WorkItemSplitFindingSeverity,
    pub code: String,
    pub message: String,
    pub work_item_ids: Vec<String>,
    /// F1.2/F2b（REQ-SG-01/02/02A）：可选 typed 失败 payload。旧 JSON 无该
    /// 字段解析为 None（按 Unknown 投影，绝不产生 L1 自动资格，也不得反推
    /// kind）；F2b 起 split 预检 producer 经 `utils::error/warning` 显式
    /// kind 工厂写入（canonical 传递 / 供需三元 ID / security operational /
    /// 缺事实折叠 Unknown），不从 code/message/severity 推导，旧三参签名
    /// 无 shim 残留。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<crate::product::planning_failure::PlanningFailureKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemPlan {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub source_story_spec_ids: Vec<String>,
    pub source_design_spec_ids: Vec<String>,
    pub options: IssueWorkItemPlanOptions,
    pub status: IssueWorkItemPlanStatus,
    pub work_item_ids: Vec<String>,
    pub repository_profile_ref: Option<String>,
    pub verification_plan_ids: Vec<String>,
    pub dependency_graph: Vec<IssueWorkItemDependencyEdge>,
    pub created_from_provider_run: Option<String>,
    pub validator_findings: Vec<WorkItemSplitFinding>,
    pub review_summary: Option<String>,
    /// D1.4（REQ-DEL-04）：单调「曾确凿确认」标记。root 首次持久由真实创建
    /// 决定（Draft 创建 Some(false)，直接以 Confirmed 落档 Some(true)）；候选
    /// parser 的内存 record 保持 None，不把旧未知当新 false；确认/commit 成功
    /// 在原 write 置 Some(true)；回退/替换/restore 读取当前根单调合并 true，
    /// 不改原确认 CAS/发布条件。旧记录 None＝历史未知。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ever_confirmed: Option<bool>,
    pub created_at: String,
    pub updated_at: String,
}

/// D1.4（REQ-DEL-04）：删除前确认分类的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionConfirmationReason {
    /// 曾确凿确认过（current Confirmed / ever_confirmed=true / 真实发布在档）。
    PreviouslyConfirmed,
    /// 旧记录无 ever_confirmed 字段且无确凿证据——历史未知，保守要求确认。
    LegacyHistoryUnknown,
}

/// D1.4（REQ-DEL-04）：实体删除前三分类（`classify_entity_deletion_confirmation`）。
/// 新路径可证未确认（Some(false)）零提示；曾确认/真实发布/旧未知各自恰一次确认。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionConfirmation {
    NotRequired,
    Required { reason: DeletionConfirmationReason },
}
