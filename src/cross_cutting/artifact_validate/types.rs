use crate::protocol::artifacts::{ArtifactKind, ArtifactRef, ArtifactStatus};
use crate::protocol::enums::{ArtifactRefId, ConstraintBundleId, ProjectionId};
use serde_json::Value;

pub type WorkPackageId = String;

#[derive(Debug, Clone, PartialEq)]
pub enum ArtifactContent {
    Markdown(String),
    Json(Value),
    JsonText(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationResult {
    pub valid: bool,
    pub warnings: Vec<String>,
}

impl ValidationResult {
    pub(super) fn valid() -> Self {
        Self {
            valid: true,
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactValidationRule {
    pub artifact_kind: ArtifactKind,
    pub requires_canonical: bool,
    pub requires_projection: bool,
    pub requires_phase1_profile: bool,
    pub content_family: ArtifactContentFamily,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactContentFamily {
    Markdown,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArtifactIndex {
    pub active_artifact_ref_ids: Vec<ArtifactRefId>,
    pub superseded_artifact_ref_ids: Vec<ArtifactRefId>,
    pub artifact_refs: Vec<ArtifactRef>,
}

impl ArtifactIndex {
    pub fn from_active_refs(artifact_refs: Vec<ArtifactRef>) -> Self {
        let active_artifact_ref_ids = artifact_refs
            .iter()
            .filter(|artifact_ref| artifact_ref.status == ArtifactStatus::Active)
            .map(|artifact_ref| artifact_ref.artifact_ref_id.clone())
            .collect();
        let superseded_artifact_ref_ids = artifact_refs
            .iter()
            .filter(|artifact_ref| artifact_ref.status == ArtifactStatus::Superseded)
            .map(|artifact_ref| artifact_ref.artifact_ref_id.clone())
            .collect();
        Self {
            active_artifact_ref_ids,
            superseded_artifact_ref_ids,
            artifact_refs,
        }
    }

    pub fn with_superseded_refs(
        artifact_refs: Vec<ArtifactRef>,
        superseded_artifact_ref_ids: Vec<ArtifactRefId>,
    ) -> Self {
        let mut index = Self::from_active_refs(artifact_refs);
        for artifact_ref_id in superseded_artifact_ref_ids {
            if !index.superseded_artifact_ref_ids.contains(&artifact_ref_id) {
                index.superseded_artifact_ref_ids.push(artifact_ref_id);
            }
        }
        index
    }

    pub(super) fn active_ref(&self, artifact_ref_id: &str) -> Option<&ArtifactRef> {
        if self.is_superseded(artifact_ref_id) {
            return None;
        }
        self.artifact_refs.iter().find(|artifact_ref| {
            artifact_ref.artifact_ref_id == artifact_ref_id
                && artifact_ref.status == ArtifactStatus::Active
        })
    }

    pub(super) fn is_superseded(&self, artifact_ref_id: &str) -> bool {
        self.superseded_artifact_ref_ids
            .iter()
            .any(|superseded| superseded == artifact_ref_id)
            || self.artifact_refs.iter().any(|artifact_ref| {
                artifact_ref.artifact_ref_id == artifact_ref_id
                    && artifact_ref.status == ArtifactStatus::Superseded
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectionIndex {
    pub projection_ids: Vec<ProjectionId>,
    pub work_package_ids: Vec<WorkPackageId>,
}

impl ProjectionIndex {
    pub fn with_work_packages(
        projection_ids: Vec<ProjectionId>,
        work_package_ids: Vec<WorkPackageId>,
    ) -> Self {
        Self {
            projection_ids,
            work_package_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConstraintBundleIndex {
    pub constraint_bundle_ids: Vec<ConstraintBundleId>,
    pub constraint_check_ids: Vec<String>,
}

impl ConstraintBundleIndex {
    pub fn with_checks(constraint_check_ids: Vec<String>) -> Self {
        Self {
            constraint_bundle_ids: Vec::new(),
            constraint_check_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TraceabilityIndex {
    pub traceability_ref_ids: Vec<String>,
}

impl TraceabilityIndex {
    pub fn with_known_refs(traceability_ref_ids: Vec<String>) -> Self {
        Self {
            traceability_ref_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderRunIndex {
    pub provider_run_ids: Vec<String>,
}

impl ProviderRunIndex {
    pub fn with_runs(provider_run_ids: Vec<String>) -> Self {
        Self { provider_run_ids }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactValidateError {
    InvalidInputSuperseded(ArtifactRefId),
    CanonicalMissingField {
        field: String,
        artifact_kind: ArtifactKind,
    },
    CanonicalTypeMismatch {
        field: String,
        expected: String,
        got: String,
    },
    ProjectionMissingField {
        field: String,
        projection_id: ProjectionId,
    },
    ProjectionInvalidId {
        id: String,
        reason: String,
    },
    ProjectionSourceNotFound(ArtifactRefId),
    ProjectionSourceHashMismatch {
        expected: String,
        got: String,
    },
    ProjectionReferenceUnknown {
        ref_id: String,
        context: String,
    },
    ProjectionPayloadEmpty(ProjectionKind),
    ProfileMissingAria,
    ProfileVersionMissing,
    ProfileProjectionRefUnknown(ProjectionId),
    ProfileConstraintRefUnknown(ConstraintBundleId),
    TraceabilityRefsMissing,
    TraceabilityRefUnknown(String),
    WorktaskRoutingSourceUnknown(WorkPackageId),
    CoverageSummaryMissing,
}

use crate::protocol::artifacts::ProjectionKind;

pub(super) fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
