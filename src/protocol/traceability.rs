use crate::protocol::enums::{ArtifactRefId, ProjectionId};
use serde::{Deserialize, Serialize};

pub type TraceabilityBindingId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactTraceabilityBinding {
    pub binding_id: TraceabilityBindingId,
    pub canonical_artifact_ref: ArtifactRefId,
    pub projection_ref: ProjectionId,
    pub related_requirement_ids: Vec<String>,
    pub related_design_decision_ids: Vec<String>,
    pub related_task_ids: Vec<String>,
    pub related_risk_ids: Vec<String>,
    pub evidence_artifact_refs: Vec<ArtifactRefId>,
    pub coverage_status: CoverageStatus,
    pub binding_status: BindingStatus,
    pub conflict_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Covered,
    Uncovered,
    Exempted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingStatus {
    Normalized,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CoverageSummary {
    pub closed: Vec<String>,
    pub uncovered: Vec<String>,
    pub exempted: Vec<String>,
    pub manual_exemptions: Vec<ManualExemption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ManualExemption {
    pub item_id: String,
    pub reason: String,
    pub approved_by: Option<String>,
}
