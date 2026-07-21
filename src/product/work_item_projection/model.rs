use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::product::work_item_contract::{
    AcceptanceCriterion, BlockerRoute, BlockerRule, DependencyContractEdge,
    DependencyContractGraph, DesignTraceabilityRef, EvidenceKind, HandoffContract,
    PromisedOutputContract, RequiredInputContract, VerificationCheck, WorkItemTask,
    WorkItemWritePolicy,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanContractSummary {
    pub contract_id: String,
    pub capabilities: Vec<String>,
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanScopeSummary {
    pub owned_scopes: Vec<String>,
    pub forbidden_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanWorkItemProjection {
    pub logical_work_item_id: String,
    pub title: String,
    pub goal: String,
    pub non_goals: Vec<String>,
    pub inputs: Vec<HumanContractSummary>,
    pub outputs: Vec<HumanContractSummary>,
    pub dependencies: Vec<String>,
    pub scope_summary: HumanScopeSummary,
    pub completion_summary: Vec<String>,
    pub source_refs: Vec<String>,
    pub normative: bool,
    pub used_by_provider: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderWorkItemProjection {
    pub work_item_revision_id: String,
    pub objective: String,
    pub required_input_contracts: Vec<RequiredInputContract>,
    pub task_refs: Vec<String>,
    pub tasks: Vec<WorkItemTask>,
    pub write_policy: WorkItemWritePolicy,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub verification_checks: Vec<VerificationCheck>,
    pub blocker_rules: Vec<BlockerRule>,
    pub handoff_contract: HandoffContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerWorkItemProjection {
    pub work_item_revision_id: String,
    pub criterion_refs: Vec<String>,
    pub requirement_matrix: Vec<ReviewerRequirementCheck>,
    pub scope_policy: WorkItemWritePolicy,
    pub input_contract_checks: Vec<RequiredInputContract>,
    pub output_contract_checks: Vec<PromisedOutputContract>,
    pub verification_evidence_rules: Vec<VerificationCheck>,
    pub blocker_routing: Vec<BlockerRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerRequirementCheck {
    pub criterion_id: String,
    pub requirement_refs: Vec<String>,
    pub required_evidence: Vec<EvidenceKind>,
    pub failure_route: BlockerRoute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanGroupWorkItemSummary {
    pub logical_work_item_id: String,
    pub title: String,
    pub goal: String,
    pub depends_on: Vec<String>,
    pub provides: Vec<String>,
    pub scope_summary: HumanScopeSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanContractFlowEdge {
    pub from: String,
    pub to: String,
    pub contract_id: String,
    pub required_capabilities: Vec<String>,
    pub provided_capabilities: Vec<String>,
    pub missing_capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanGroupProjection {
    pub plan_id: String,
    pub goal: String,
    pub split_reason: String,
    pub work_items: Vec<HumanGroupWorkItemSummary>,
    pub contract_flow: Vec<HumanContractFlowEdge>,
    pub risks: Vec<String>,
    pub source_refs: Vec<String>,
    pub normative: bool,
    pub used_by_provider: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderGroupContext {
    pub plan_id: String,
    pub ordered_logical_work_item_ids: Vec<String>,
    pub dependency_edges: Vec<DependencyContractEdge>,
    pub group_write_scopes: BTreeMap<String, WorkItemWritePolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerGroupMatrixEntry {
    pub logical_work_item_id: String,
    pub criterion_refs: Vec<String>,
    pub input_contract_refs: Vec<String>,
    pub output_contract_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerGroupMatrix {
    pub plan_id: String,
    pub work_items: Vec<ReviewerGroupMatrixEntry>,
    pub dependency_edges: Vec<DependencyContractEdge>,
    pub design_traceability_refs: Vec<DesignTraceabilityRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledWorkItemProjections {
    pub human: HumanWorkItemProjection,
    pub coder: CoderWorkItemProjection,
    pub reviewer: ReviewerWorkItemProjection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledPlanProjections {
    pub human: HumanGroupProjection,
    pub coder: CoderGroupContext,
    pub reviewer: ReviewerGroupMatrix,
}

pub struct PlanProjectionCompileInput<'a> {
    pub plan_id: &'a str,
    pub goal: &'a str,
    pub split_reason: &'a str,
    pub source_refs: Vec<String>,
    pub dependency_graph: &'a DependencyContractGraph,
    pub work_item_projections: &'a BTreeMap<String, CompiledWorkItemProjections>,
    pub expected_work_item_revision_ids: BTreeMap<String, String>,
}

pub struct PlanProjectionValidationInput<'a> {
    pub expected_plan_id: &'a str,
    pub expected_source_refs: &'a [String],
    pub expected_work_item_revision_ids: &'a BTreeMap<String, String>,
    pub dependency_graph: &'a DependencyContractGraph,
    pub compiled: &'a CompiledPlanProjections,
    pub work_item_projections: &'a BTreeMap<String, CompiledWorkItemProjections>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionValidationFinding {
    pub code: String,
    pub projection: String,
    pub contract_ref: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionValidationReport {
    pub findings: Vec<ProjectionValidationFinding>,
}

impl ProjectionValidationReport {
    pub fn is_valid(&self) -> bool {
        self.findings.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionHashes {
    pub human: String,
    pub coder: String,
    pub reviewer: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionCompileError {
    Validation(ProjectionValidationReport),
    InvalidHumanPresentation(String),
    Serialization(String),
}

impl std::fmt::Display for ProjectionCompileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(report) => write!(
                formatter,
                "projection validation failed with {} finding(s)",
                report.findings.len()
            ),
            Self::InvalidHumanPresentation(message) | Self::Serialization(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for ProjectionCompileError {}

pub enum HumanPresentationBase<'a> {
    Plan {
        projection_bundle_id: &'a str,
        projection: &'a HumanGroupProjection,
    },
    WorkItem {
        projection_bundle_id: &'a str,
        projection: &'a HumanWorkItemProjection,
    },
}
