use crate::protocol::enums::{ChangeId, ConstraintBundleId, IsoDateTime, NodeId, ProjectionId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenSpecBootstrapStatus {
    BootstrapPending,
    Bootstrapped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleStatus {
    BootstrapPending,
    Ready,
    Stale,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenSpecSourceKind {
    Proposal,
    Spec,
    Design,
    Tasks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OpenSpecSourceFile {
    pub path: String,
    pub kind: OpenSpecSourceKind,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OpenSpecConstraintBundle {
    pub constraint_bundle_id: ConstraintBundleId,
    pub bundle_version: String,
    pub bundle_status: BundleStatus,
    pub change_id: ChangeId,
    pub proposal_constraints: ProposalConstraints,
    pub requirement_constraints: RequirementConstraints,
    pub design_constraints: DesignConstraints,
    pub task_constraints: TaskConstraints,
    pub traceability_requirements: TraceabilityRequirements,
    pub coverage_model: CoverageModel,
    pub source_manifest: Vec<OpenSpecSourceFile>,
    pub compiled_from_projection_refs: Vec<ProjectionId>,
    pub compiled_at: IsoDateTime,
    pub compiled_by_node: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProposalConstraints {
    pub business_intent: Vec<String>,
    pub scope: Vec<String>,
    pub non_goals: Vec<String>,
    pub impacted_areas: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RequirementConstraints {
    pub requirement_ids: Vec<String>,
    pub scenario_ids: Vec<String>,
    pub success_criteria_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DesignConstraints {
    pub design_decision_ids: Vec<String>,
    pub component_ids: Vec<String>,
    pub risk_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskConstraints {
    pub task_ids: Vec<String>,
    pub task_sequence: Vec<String>,
    pub related_requirement_ids_by_task: BTreeMap<String, Vec<String>>,
    pub related_design_decision_ids_by_task: BTreeMap<String, Vec<String>>,
    pub acceptance_target_ids_by_task: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TraceabilityRequirements {
    pub required_requirement_ids: Vec<String>,
    pub required_design_decision_ids: Vec<String>,
    pub required_task_ids: Vec<String>,
    pub required_acceptance_target_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CoverageModel {
    pub required_ids: Vec<String>,
    pub covered_ids: Vec<String>,
    pub uncovered_ids: Vec<String>,
}
