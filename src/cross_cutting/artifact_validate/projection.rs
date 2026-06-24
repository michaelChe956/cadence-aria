use super::types::{ArtifactIndex, ArtifactValidateError, ValidationResult};
use crate::cross_cutting::document_ops::compute_sha256;
use crate::protocol::artifacts::{ArtifactStatus, ProjectionKind};
use crate::protocol::projections::{
    ArtifactProjectionRecord, ProjectionPayload, WorkDependencyProjection, WorkPackageProjection,
};
use serde_json::Value;
use std::path::Path;

pub fn projection_validator(
    record: &ArtifactProjectionRecord,
    artifact_index: &ArtifactIndex,
    golden_fixture: Option<&Path>,
) -> Result<ValidationResult, ArtifactValidateError> {
    if record.projection_id.trim().is_empty() {
        return Err(ArtifactValidateError::ProjectionMissingField {
            field: "projection_id".to_string(),
            projection_id: record.projection_id.clone(),
        });
    }
    let expected_prefix = format!(
        "proj_{}_{}_",
        record.projection_kind.as_str(),
        record.source_artifact_ref.artifact_id
    );
    if !record.projection_id.starts_with(&expected_prefix) {
        return Err(ArtifactValidateError::ProjectionInvalidId {
            id: record.projection_id.clone(),
            reason: format!("expected prefix {expected_prefix}"),
        });
    }
    if record.payload.projection_kind() != record.projection_kind {
        return Err(ArtifactValidateError::ProjectionInvalidId {
            id: record.projection_id.clone(),
            reason: "projection_kind does not match payload".to_string(),
        });
    }
    if record.source_artifact_ref.status == ArtifactStatus::Superseded
        || artifact_index.is_superseded(&record.source_artifact_ref.artifact_ref_id)
    {
        return Err(ArtifactValidateError::InvalidInputSuperseded(
            record.source_artifact_ref.artifact_ref_id.clone(),
        ));
    }
    let active_ref = artifact_index
        .active_ref(&record.source_artifact_ref.artifact_ref_id)
        .or_else(|| {
            artifact_index
                .active_artifact_ref_ids
                .contains(&record.source_artifact_ref.artifact_ref_id)
                .then_some(&record.source_artifact_ref)
        })
        .ok_or_else(|| {
            ArtifactValidateError::ProjectionSourceNotFound(
                record.source_artifact_ref.artifact_ref_id.clone(),
            )
        })?;
    if record.source_artifact_version != active_ref.version {
        return Err(ArtifactValidateError::ProjectionReferenceUnknown {
            ref_id: active_ref.artifact_ref_id.clone(),
            context: "source_artifact_version".to_string(),
        });
    }
    let current_hash = if !active_ref.path.is_empty() && Path::new(&active_ref.path).exists() {
        std::fs::read(&active_ref.path)
            .map(|content| compute_sha256(&content))
            .unwrap_or_else(|_| active_ref.sha256.clone())
    } else {
        active_ref.sha256.clone()
    };
    if record.source_artifact_hash != current_hash {
        return Err(ArtifactValidateError::ProjectionSourceHashMismatch {
            expected: current_hash,
            got: record.source_artifact_hash.clone(),
        });
    }
    if record.compiled_at.trim().is_empty() {
        return Err(ArtifactValidateError::ProjectionMissingField {
            field: "compiled_at".to_string(),
            projection_id: record.projection_id.clone(),
        });
    }
    if record.compiled_by_node.trim().is_empty() {
        return Err(ArtifactValidateError::ProjectionMissingField {
            field: "compiled_by_node".to_string(),
            projection_id: record.projection_id.clone(),
        });
    }
    if record.payload.is_empty() {
        return Err(ArtifactValidateError::ProjectionPayloadEmpty(
            record.projection_kind,
        ));
    }
    validate_projection_payload(&record.payload)?;
    if let Some(path) = golden_fixture {
        let golden: Value = serde_json::from_slice(&std::fs::read(path).map_err(|_| {
            ArtifactValidateError::ProjectionReferenceUnknown {
                ref_id: path.display().to_string(),
                context: "golden_fixture".to_string(),
            }
        })?)
        .map_err(|_| ArtifactValidateError::ProjectionReferenceUnknown {
            ref_id: path.display().to_string(),
            context: "golden_fixture_json".to_string(),
        })?;
        if golden
            != record.payload.inner_json().map_err(|_| {
                ArtifactValidateError::ProjectionReferenceUnknown {
                    ref_id: record.projection_id.clone(),
                    context: "projection_payload_json".to_string(),
                }
            })?
        {
            return Err(ArtifactValidateError::ProjectionReferenceUnknown {
                ref_id: record.projection_id.clone(),
                context: "golden_fixture_mismatch".to_string(),
            });
        }
    }
    Ok(ValidationResult::valid())
}

fn validate_projection_payload(payload: &ProjectionPayload) -> Result<(), ArtifactValidateError> {
    match payload {
        ProjectionPayload::SpecProjection(spec) => {
            if spec.functional_requirements.is_empty() {
                return Err(ArtifactValidateError::ProjectionPayloadEmpty(
                    ProjectionKind::SpecProjection,
                ));
            }
            let known_requirements: Vec<&str> = spec
                .functional_requirements
                .iter()
                .map(|requirement| requirement.requirement_id.as_str())
                .collect();
            for criterion in &spec.success_criteria {
                for ref_id in &criterion.related_requirement_ids {
                    if !known_requirements.contains(&ref_id.as_str()) {
                        return Err(ArtifactValidateError::ProjectionReferenceUnknown {
                            ref_id: ref_id.clone(),
                            context: "success_criteria".to_string(),
                        });
                    }
                }
            }
            Ok(())
        }
        ProjectionPayload::DesignProjection(design) => {
            if design.design_decisions.is_empty() {
                return Err(ArtifactValidateError::ProjectionPayloadEmpty(
                    ProjectionKind::DesignProjection,
                ));
            }
            Ok(())
        }
        ProjectionPayload::PlanProjection(plan) => {
            if plan.work_packages.is_empty() {
                return Err(ArtifactValidateError::ProjectionPayloadEmpty(
                    ProjectionKind::PlanProjection,
                ));
            }
            validate_plan_projection(&plan.work_packages, &plan.dependencies)
        }
    }
}

fn validate_plan_projection(
    work_packages: &[WorkPackageProjection],
    dependencies: &[WorkDependencyProjection],
) -> Result<(), ArtifactValidateError> {
    let mut known = Vec::new();
    for work_package in work_packages {
        if known.contains(&work_package.work_package_id) {
            return Err(ArtifactValidateError::ProjectionInvalidId {
                id: work_package.work_package_id.clone(),
                reason: "duplicate work_package_id".to_string(),
            });
        }
        known.push(work_package.work_package_id.clone());
        if work_package.traceability_refs.is_empty() {
            return Err(ArtifactValidateError::ProjectionReferenceUnknown {
                ref_id: work_package.work_package_id.clone(),
                context: "traceability_refs".to_string(),
            });
        }
    }
    for dependency in dependencies {
        if !known.contains(&dependency.from_work_package_id)
            || !known.contains(&dependency.to_work_package_id)
        {
            return Err(ArtifactValidateError::ProjectionReferenceUnknown {
                ref_id: format!(
                    "{}->{}",
                    dependency.from_work_package_id, dependency.to_work_package_id
                ),
                context: "dependencies".to_string(),
            });
        }
    }
    Ok(())
}
