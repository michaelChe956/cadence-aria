use serde::{Deserialize, Serialize};

use crate::product::models::WorkItemExecutionPlanStatus;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemDependencyHandoffRef {
    pub work_item_id: String,
    pub summary_ref: Option<String>,
    pub summary: Option<String>,
    pub commit_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemExecutionPlan {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub attempt_id: String,
    pub status: WorkItemExecutionPlanStatus,
    pub goal: String,
    #[serde(default)]
    pub allowed_write_scopes: Vec<String>,
    #[serde(default)]
    pub forbidden_write_scopes: Vec<String>,
    #[serde(default)]
    pub dependency_handoffs: Vec<WorkItemDependencyHandoffRef>,
    #[serde(default)]
    pub story_refs: Vec<String>,
    #[serde(default)]
    pub design_refs: Vec<String>,
    #[serde(default)]
    pub openspec_refs: Vec<String>,
    #[serde(default)]
    pub superpowers_contract: String,
    #[serde(default)]
    pub tdd_contract: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_plan_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_summary: Option<String>,
    #[serde(default)]
    pub risk_notes: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemHandoff {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub attempt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_run_ref: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub files_changed: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub diff_summary: String,
    #[serde(default)]
    pub tests_run: Vec<String>,
    #[serde(default)]
    pub test_result_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_summary: Option<String>,
    #[serde(default)]
    pub api_or_contract_changes: Vec<String>,
    #[serde(default)]
    pub open_risks: Vec<String>,
    #[serde(default)]
    pub next_work_item_notes: Vec<String>,
    pub created_at: String,
}
