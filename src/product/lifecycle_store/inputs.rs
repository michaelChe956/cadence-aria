use std::path::PathBuf;

use crate::product::models::{
    IssueWorkItemDependencyEdge, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, ProviderName,
    RepositoryProfileConfidence, VerificationCommand, VerificationFallbackPolicy,
    VerificationManualCheck, VerificationScope, WorkItemContextBudget, WorkItemKind,
    WorkItemPlanStatus, WorkItemSplitFinding, WorkspaceType,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateStorySpecInput {
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDesignSpecInput {
    pub project_id: String,
    pub issue_id: String,
    pub story_spec_ids: Vec<String>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorkItemInput {
    pub id: Option<String>,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub title: String,
    pub work_item_set_id: Option<String>,
    pub source_work_item_plan_id: Option<String>,
    pub source_outline_id: Option<String>,
    pub source_draft_id: Option<String>,
    pub planned_implementation_context: Option<String>,
    pub planned_handoff_summary: Option<String>,
    pub kind: WorkItemKind,
    pub sequence_hint: Option<u32>,
    pub depends_on: Vec<String>,
    pub exclusive_write_scopes: Vec<String>,
    pub forbidden_write_scopes: Vec<String>,
    pub context_budget: WorkItemContextBudget,
    pub required_handoff_from: Vec<String>,
    pub verification_plan_ref: Option<String>,
    pub require_execution_plan_confirm: bool,
    pub plan_status: WorkItemPlanStatus,
}

impl Default for CreateWorkItemInput {
    fn default() -> Self {
        Self {
            id: None,
            project_id: String::new(),
            issue_id: String::new(),
            repository_id: String::new(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: String::new(),
            work_item_set_id: None,
            source_work_item_plan_id: None,
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            planned_handoff_summary: None,
            kind: WorkItemKind::default(),
            sequence_hint: None,
            depends_on: Vec::new(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: WorkItemContextBudget::default(),
            required_handoff_from: Vec::new(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            plan_status: WorkItemPlanStatus::NotStarted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateIssueWorkItemPlanInput {
    pub id: Option<String>,
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
    pub validator_findings: Vec<crate::product::models::WorkItemSplitFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueWorkItemPlanUpdate {
    pub work_item_ids: Vec<String>,
    pub verification_plan_ids: Vec<String>,
    pub repository_profile_ref: Option<String>,
    pub dependency_graph: Vec<IssueWorkItemDependencyEdge>,
    pub created_from_provider_run: Option<String>,
    pub validator_findings: Vec<WorkItemSplitFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemPlanCandidateSnapshot {
    pub plan_id: String,
    pub work_item_ids: Vec<String>,
    pub verification_plan_ids: Vec<String>,
    pub repository_profile_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRepositoryProfileInput {
    pub id: Option<String>,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub provider_run_ref: Option<String>,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub package_managers: Vec<String>,
    pub test_frameworks: Vec<String>,
    pub build_systems: Vec<String>,
    pub verification_capabilities: Vec<String>,
    pub detected_layers: Vec<String>,
    pub split_recommendation: String,
    pub confidence: RepositoryProfileConfidence,
    pub uncertainties: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateVerificationPlanInput {
    pub id: Option<String>,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub repository_profile_ref: Option<String>,
    pub provider_run_ref: Option<String>,
    pub scope: VerificationScope,
    pub commands: Vec<VerificationCommand>,
    pub manual_checks: Vec<VerificationManualCheck>,
    pub required_gates: Vec<String>,
    pub risk_notes: Vec<String>,
    pub confidence: RepositoryProfileConfidence,
    pub fallback_policy: VerificationFallbackPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendSpecVersionInput {
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub markdown: String,
    pub provider_run_refs: Vec<String>,
    pub review_refs: Vec<String>,
    pub confirmed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendProviderReviewRoundInput {
    pub project_id: String,
    pub issue_id: String,
    pub session_id: String,
    pub round_index: u32,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_result: String,
    pub revision_result: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorkspaceSessionInput {
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: WorkspaceType,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectProviderDefaultsInput {
    pub project_id: String,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertIssueSharedWorktreeInput {
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub base_branch: String,
}
