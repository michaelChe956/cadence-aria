use crate::protocol::enums::{ArtifactRefId, IsoDateTime, NodeId, TaskId};
use crate::protocol::projections::RiskSeverity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    IntakeBrief,
    ClarificationRecord,
    Spec,
    SpecGateDecision,
    Design,
    DesignReview,
    DesignRevisionRecord,
    ReadinessCheck,
    Plan,
    DispatchPackage,
    CodingReport,
    TestingReport,
    CodeReviewReport,
    IntegrationReport,
    FinalReview,
    FinalSummary,
    RuntimeSnapshot,
}

impl ArtifactKind {
    pub fn all_phase1() -> impl Iterator<Item = ArtifactKind> {
        [
            ArtifactKind::IntakeBrief,
            ArtifactKind::ClarificationRecord,
            ArtifactKind::Spec,
            ArtifactKind::SpecGateDecision,
            ArtifactKind::Design,
            ArtifactKind::DesignReview,
            ArtifactKind::DesignRevisionRecord,
            ArtifactKind::ReadinessCheck,
            ArtifactKind::Plan,
            ArtifactKind::DispatchPackage,
            ArtifactKind::CodingReport,
            ArtifactKind::TestingReport,
            ArtifactKind::CodeReviewReport,
            ArtifactKind::IntegrationReport,
            ArtifactKind::FinalReview,
            ArtifactKind::FinalSummary,
            ArtifactKind::RuntimeSnapshot,
        ]
        .into_iter()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::IntakeBrief => "intake_brief",
            ArtifactKind::ClarificationRecord => "clarification_record",
            ArtifactKind::Spec => "spec",
            ArtifactKind::SpecGateDecision => "spec_gate_decision",
            ArtifactKind::Design => "design",
            ArtifactKind::DesignReview => "design_review",
            ArtifactKind::DesignRevisionRecord => "design_revision_record",
            ArtifactKind::ReadinessCheck => "readiness_check",
            ArtifactKind::Plan => "plan",
            ArtifactKind::DispatchPackage => "dispatch_package",
            ArtifactKind::CodingReport => "coding_report",
            ArtifactKind::TestingReport => "testing_report",
            ArtifactKind::CodeReviewReport => "code_review_report",
            ArtifactKind::IntegrationReport => "integration_report",
            ArtifactKind::FinalReview => "final_review",
            ArtifactKind::FinalSummary => "final_summary",
            ArtifactKind::RuntimeSnapshot => "runtime_snapshot",
        }
    }

    pub fn is_markdown_canonical(self) -> bool {
        matches!(
            self,
            ArtifactKind::Spec | ArtifactKind::Design | ArtifactKind::Plan
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionKind {
    SpecProjection,
    DesignProjection,
    PlanProjection,
}

impl ProjectionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectionKind::SpecProjection => "spec_projection",
            ProjectionKind::DesignProjection => "design_projection",
            ProjectionKind::PlanProjection => "plan_projection",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    Active,
    Superseded,
    Candidate,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactRef {
    pub artifact_ref_id: String,
    #[serde(default)]
    pub artifact_id: String,
    #[serde(default = "default_artifact_kind")]
    pub artifact_kind: ArtifactKind,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default = "default_artifact_status")]
    pub status: ArtifactStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskStatus {
    Open,
    Mitigated,
    Accepted,
    Escalated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RiskEntry {
    pub risk_id: String,
    pub description: String,
    pub severity: RiskSeverity,
    pub status: RiskStatus,
    pub source_artifact: Option<ArtifactRefId>,
    pub source_node: NodeId,
    pub created_at: IsoDateTime,
    pub updated_at: IsoDateTime,
    pub resolution: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RiskRegistrySnapshot {
    #[serde(default)]
    pub registry_id: String,
    pub risk_registry_ref: String,
    #[serde(default)]
    pub task_id: TaskId,
    #[serde(default)]
    pub risk_ids: Vec<String>,
    #[serde(default)]
    pub risks: Vec<RiskEntry>,
    #[serde(default)]
    pub updated_at: IsoDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RiskRegistryRef {
    pub risk_registry_ref_id: String,
    pub risk_registry_id: String,
    pub task_id: TaskId,
    pub path: String,
    pub sha256: String,
    pub version: u32,
    pub risk_count: usize,
}

pub fn risk_ids_from_artifact_refs(refs: &[String]) -> Vec<String> {
    let mut risk_ids = Vec::new();
    for ref_id in refs {
        let normalized = normalize_ref_id(ref_id);
        if normalized.starts_with("risk-") && !risk_ids.contains(&normalized) {
            risk_ids.push(normalized);
        }
    }
    risk_ids
}

fn normalize_ref_id(value: &str) -> String {
    value
        .trim()
        .trim_matches(',')
        .trim_matches(';')
        .to_ascii_lowercase()
        .replace('_', "-")
}

fn default_artifact_kind() -> ArtifactKind {
    ArtifactKind::IntakeBrief
}

fn default_artifact_status() -> ArtifactStatus {
    ArtifactStatus::Active
}
