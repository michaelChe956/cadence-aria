use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::lifecycle::{IssueWorkItemPlan, WorkItemKind, WorkItemSplitFinding};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanOutline {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub source_story_spec_ids: Vec<String>,
    pub source_design_spec_ids: Vec<String>,
    pub strategy_summary: String,
    pub work_item_outlines: Vec<WorkItemOutline>,
    #[serde(default)]
    pub dependency_graph: Vec<WorkItemOutlineDependencyEdge>,
    pub risks: Vec<String>,
    pub handoff_strategy: String,
    pub status: String,
}

impl WorkItemPlanOutline {
    pub fn dependency_graph_from_depends_on(&self) -> Vec<WorkItemOutlineDependencyEdge> {
        self.work_item_outlines
            .iter()
            .flat_map(|item| {
                item.depends_on
                    .iter()
                    .map(|dependency| WorkItemOutlineDependencyEdge {
                        from_outline_id: dependency.clone(),
                        to_outline_id: item.outline_id.clone(),
                    })
            })
            .collect()
    }

    pub fn normalize_dependency_graph_from_depends_on(&mut self) {
        self.dependency_graph = self.dependency_graph_from_depends_on();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemOutline {
    pub outline_id: String,
    pub title: String,
    pub kind: WorkItemKind,
    pub goal: String,
    pub scope: Vec<String>,
    pub non_goals: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_context_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_fit: Option<WorkItemOutlineSessionFit>,
    pub source_story_spec_ids: Vec<String>,
    pub source_design_spec_ids: Vec<String>,
    pub exclusive_write_scopes: Vec<String>,
    pub forbidden_write_scopes: Vec<String>,
    pub depends_on: Vec<String>,
    pub verification_intent: Vec<String>,
    pub handoff_notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemOutlineSessionFit {
    FitsSingleAgentSession,
    TooLargeMustSplit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemOutlineDependencyEdge {
    pub from_outline_id: String,
    pub to_outline_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemDraftCandidate {
    pub outline_id: String,
    pub title: String,
    pub kind: WorkItemKind,
    pub goal: String,
    pub implementation_context: String,
    pub exclusive_write_scopes: Vec<String>,
    pub forbidden_write_scopes: Vec<String>,
    pub depends_on_outline_ids: Vec<String>,
    pub required_handoff_from_outline_ids: Vec<String>,
    pub handoff_summary: String,
    pub verification_plan: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemDraftRecord {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub draft_id: String,
    pub outline_id: String,
    pub generation_round_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    pub attempt_index: u32,
    pub outline_version_ref: String,
    pub generation_mode: WorkItemGenerationMode,
    pub candidate: WorkItemDraftCandidate,
    pub status: WorkItemDraftStatus,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by_draft_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersede_reason: Option<WorkItemDraftSupersedeReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copied_from_draft_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_verdict_ref: Option<String>,
    pub generated_from_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemDraftStatus {
    Draft,
    Accepted,
    Superseded,
    ValidationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemDraftSupersedeReason {
    DirectRewrite,
    AncestorRewritten,
    OutlineRevised,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemGenerationMode {
    Serial,
    Batch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemBatchStatus {
    Generating,
    Completed,
    ReviewPending,
    ReviewDone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemBatchRecord {
    pub batch_id: String,
    pub generation_round_id: String,
    pub mode: WorkItemGenerationMode,
    pub item_draft_ids: Vec<String>,
    pub status: WorkItemBatchStatus,
    pub validation_failed_ids: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanDraftActiveIndex {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub current_generation_round_id: String,
    pub outline_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_outline_id: Option<String>,
    pub outline_to_current_draft_id: BTreeMap<String, String>,
    pub draft_statuses: BTreeMap<String, WorkItemDraftStatus>,
    pub batches: Vec<WorkItemBatchRecord>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanCompileStatus {
    Preparing,
    Validating,
    Committing,
    Committed,
    Failed,
    RecoveryRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanCommitState {
    NotStarted,
    Committed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanCompileTransaction {
    pub compile_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub generation_round_id: String,
    pub outline_version_ref: String,
    pub active_draft_ids: Vec<String>,
    pub status: WorkItemPlanCompileStatus,
    pub plan_commit_state: WorkItemPlanCommitState,
    pub step_cursor: String,
    pub outline_to_work_item_id: BTreeMap<String, String>,
    pub outline_to_verification_plan_id: BTreeMap<String, String>,
    pub created_work_item_ids: Vec<String>,
    pub created_verification_plan_ids: Vec<String>,
    pub child_session_ids: Vec<String>,
    pub validator_findings: Vec<WorkItemSplitFinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abort_requested_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub previous_plan_snapshot: IssueWorkItemPlan,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DesignContextCapabilities {
    pub has_architecture: bool,
    pub has_module_breakdown: bool,
    pub has_tech_stack: bool,
    pub has_test_strategy: bool,
    pub has_key_paths: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OutlineContextBlockerResolution {
    pub blocker_node_id: String,
    pub resolution_node_id: String,
    pub resolution_artifact_ref: String,
    pub estimated_tokens: u32,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OutlineContextIndex {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub generation_round_id: String,
    pub blocker_resolutions: Vec<OutlineContextBlockerResolution>,
    pub design_context_gaps: Vec<String>,
    pub design_context_capabilities: DesignContextCapabilities,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SpecVersionRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub version: u32,
    pub markdown: String,
    pub provider_run_refs: Vec<String>,
    pub review_refs: Vec<String>,
    pub confirmed_by: Option<String>,
    pub created_at: String,
}
