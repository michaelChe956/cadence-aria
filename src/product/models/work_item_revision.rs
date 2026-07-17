use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractValidationReport, DependencyContractEdge, VerificationCheck,
};
use crate::product::work_item_projection::{
    CoderGroupContext, CoderWorkItemProjection, HumanGroupProjection, HumanWorkItemProjection,
    ProjectionValidationReport, ReviewerGroupMatrix, ReviewerWorkItemProjection,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemPlanLineage {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub story_spec_refs: Vec<String>,
    pub design_spec_refs: Vec<String>,
    pub active_revision_id: Option<String>,
    pub active_amendment_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRevisionReason {
    InitialCompile,
    RepairCurrentWorkItem,
    RepairUpstreamContract,
    SubgraphReplan,
    StoryAmendment,
    DesignAmendment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemPlanRevision {
    pub id: String,
    pub plan_id: String,
    pub revision_no: u32,
    pub supersedes: Option<String>,
    pub reason: PlanRevisionReason,
    pub work_item_bindings: BTreeMap<String, String>,
    pub dependency_graph_revision_id: String,
    pub validation_report_ref: String,
    pub plan_projection_bundle_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalWorkItem {
    pub id: String,
    pub plan_id: String,
    pub title: String,
    pub active_revision_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemDraftRevision {
    pub id: String,
    pub logical_work_item_id: String,
    pub revision_no: u32,
    pub supersedes: Option<String>,
    pub revision_reason: PlanRevisionReason,
    pub canonical_contract_candidate: CanonicalWorkItemContract,
    pub trigger_repair_request_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemDraftRevisionState {
    pub draft_revision_id: String,
    pub status: WorkItemDraftRevisionStatus,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemDraftRevisionStatus {
    Drafting,
    Reviewing,
    ChangesRequested,
    Approved,
    Rejected,
    Compiled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemRevision {
    pub id: String,
    pub logical_work_item_id: String,
    pub source_draft_revision_id: String,
    pub canonical_contract: CanonicalWorkItemContract,
    pub canonical_contract_hash: String,
    pub work_item_projection_bundle_id: String,
    pub verification_plan_revision_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationPlanRevision {
    pub id: String,
    pub logical_work_item_id: String,
    pub source_draft_revision_id: String,
    pub verification_checks: Vec<VerificationCheck>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanValidationReportArtifact {
    pub id: String,
    pub plan_id: String,
    pub contract_validation: ContractValidationReport,
    pub projection_validation: ProjectionValidationReport,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemProjectionBundle {
    pub id: String,
    pub work_item_revision_id: String,
    pub canonical_contract_hash: String,
    pub projection_schema_version: u32,
    pub compiler_version: String,
    pub human_projection: HumanWorkItemProjection,
    pub coder_projection: CoderWorkItemProjection,
    pub reviewer_projection: ReviewerWorkItemProjection,
    pub human_projection_hash: String,
    pub coder_projection_hash: String,
    pub reviewer_projection_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanProjectionBundle {
    pub id: String,
    pub plan_revision_id: String,
    pub dependency_graph_revision_id: String,
    pub work_item_projection_bundle_refs: Vec<String>,
    pub human_group_projection: HumanGroupProjection,
    pub coder_group_context: CoderGroupContext,
    pub reviewer_group_matrix: ReviewerGroupMatrix,
    pub human_group_projection_hash: String,
    pub coder_group_context_hash: String,
    pub reviewer_group_matrix_hash: String,
    pub compiler_version: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanPresentationRevision {
    pub id: String,
    pub source_plan_projection_bundle_id: Option<String>,
    pub source_work_item_projection_bundle_id: Option<String>,
    pub supersedes: Option<String>,
    pub human_summary: String,
    pub why_split: Option<String>,
    pub dependency_explanation: Vec<String>,
    pub risk_explanation: Vec<String>,
    pub source_refs: Vec<String>,
    pub normative: bool,
    pub used_by_provider: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffRevision {
    pub id: String,
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub coding_unit_run_id: String,
    pub provided_contracts: Vec<String>,
    pub provided_capabilities: BTreeMap<String, Vec<String>>,
    pub contract_hash: String,
    pub commit_sha: String,
    pub tests: Vec<String>,
    pub artifacts: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyGraphRevision {
    pub id: String,
    pub plan_id: String,
    pub edges: Vec<DependencyContractEdge>,
    pub created_at: String,
}
