use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
    pub fn as_str(&self) -> &'static str {
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
    pub max_handoff_chars: usize,
    pub max_code_context_chars: usize,
    pub max_context_file_refs: usize,
    pub max_traceability_refs: usize,
    pub max_dependency_handoffs: usize,
}

impl Default for WorkItemContextBudget {
    fn default() -> Self {
        Self {
            target_context_k: "30-50".to_string(),
            max_summary_chars: 20_000,
            max_handoff_chars: 12_000,
            max_code_context_chars: 30_000,
            max_context_file_refs: 80,
            max_traceability_refs: 40,
            max_dependency_handoffs: 3,
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
    pub current_version: Option<u32>,
    pub confirmation_status: LifecycleConfirmationStatus,
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
    pub current_version: Option<u32>,
    pub confirmation_status: LifecycleConfirmationStatus,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_handoff_summary: Option<String>,
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
    pub required_handoff_from: Vec<String>,
    #[serde(default)]
    pub verification_plan_ref: Option<String>,
    #[serde(default)]
    pub require_execution_plan_confirm: bool,
    #[serde(default)]
    pub execution_plan_status: WorkItemExecutionPlanStatus,
    #[serde(default)]
    pub handoff_summary_ref: Option<String>,
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
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub base_branch: String,
    pub status: IssueSharedWorktreeStatus,
    pub current_active_work_item_id: Option<String>,
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
    pub created_at: String,
    pub updated_at: String,
}
