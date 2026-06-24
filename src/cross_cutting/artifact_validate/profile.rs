use super::types::{
    ArtifactValidateError, ConstraintBundleIndex, ProjectionIndex, ProviderRunIndex,
    TraceabilityIndex, ValidationResult, WorkPackageId,
};
use crate::protocol::artifacts::ArtifactKind;
use crate::protocol::phase1_profile::PHASE1_PROFILE_VERSION;
use serde_json::Value;

pub fn phase1_profile_validator(
    artifact_value: &Value,
    artifact_kind: ArtifactKind,
    projection_index: &ProjectionIndex,
    constraint_bundle_index: &ConstraintBundleIndex,
    traceability_index: &TraceabilityIndex,
    provider_run_index: &ProviderRunIndex,
) -> Result<ValidationResult, ArtifactValidateError> {
    let aria = artifact_value
        .get("_aria")
        .and_then(Value::as_object)
        .ok_or(ArtifactValidateError::ProfileMissingAria)?;
    let profile_version = aria
        .get("profile_version")
        .and_then(Value::as_str)
        .ok_or(ArtifactValidateError::ProfileVersionMissing)?;
    if profile_version != PHASE1_PROFILE_VERSION {
        return Err(ArtifactValidateError::ProfileVersionMissing);
    }
    let constraint_check_ref = aria
        .get("constraint_check_ref")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if constraint_check_ref.is_empty()
        || (!constraint_bundle_index
            .constraint_check_ids
            .contains(&constraint_check_ref.to_string())
            && !constraint_bundle_index
                .constraint_bundle_ids
                .contains(&constraint_check_ref.to_string()))
    {
        return Err(ArtifactValidateError::ProfileConstraintRefUnknown(
            constraint_check_ref.to_string(),
        ));
    }

    let traceability_refs = aria
        .get("traceability_refs")
        .and_then(Value::as_array)
        .ok_or(ArtifactValidateError::TraceabilityRefsMissing)?;
    for ref_value in traceability_refs {
        let ref_id = ref_value
            .as_str()
            .ok_or_else(|| ArtifactValidateError::TraceabilityRefUnknown(ref_value.to_string()))?;
        if !traceability_index
            .traceability_ref_ids
            .iter()
            .any(|known| known == ref_id)
        {
            return Err(ArtifactValidateError::TraceabilityRefUnknown(
                ref_id.to_string(),
            ));
        }
    }
    if let Some(provider_run_refs) = aria.get("provider_run_refs").and_then(Value::as_array) {
        for provider_run_ref in provider_run_refs {
            let run_id = provider_run_ref.as_str().unwrap_or_default().to_string();
            if !provider_run_index.provider_run_ids.contains(&run_id) {
                return Err(ArtifactValidateError::TraceabilityRefUnknown(run_id));
            }
        }
    }
    if let Some(projection_refs) = aria.get("projection_refs").and_then(Value::as_array) {
        for projection_ref in projection_refs {
            let projection_id = projection_ref.as_str().unwrap_or_default().to_string();
            if !projection_index.projection_ids.contains(&projection_id) {
                return Err(ArtifactValidateError::ProfileProjectionRefUnknown(
                    projection_id,
                ));
            }
        }
    }

    match artifact_kind {
        ArtifactKind::DispatchPackage => {
            validate_worktask_routing(
                aria.get("worktask_routing"),
                &projection_index.work_package_ids,
            )?;
        }
        ArtifactKind::FinalReview => {
            validate_coverage_summary(aria.get("coverage_summary"))?;
        }
        ArtifactKind::CodingReport
        | ArtifactKind::TestingReport
        | ArtifactKind::CodeReviewReport
        | ArtifactKind::IntegrationReport
            if traceability_refs.is_empty() =>
        {
            return Err(ArtifactValidateError::TraceabilityRefsMissing);
        }
        _ => {}
    }
    Ok(ValidationResult::valid())
}

fn validate_worktask_routing(
    value: Option<&Value>,
    known_work_package_ids: &[WorkPackageId],
) -> Result<(), ArtifactValidateError> {
    let routing = value
        .and_then(Value::as_array)
        .ok_or_else(|| ArtifactValidateError::WorktaskRoutingSourceUnknown(String::new()))?;
    for item in routing {
        let source_work_package_id = item
            .get("source_work_package_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !known_work_package_ids.contains(&source_work_package_id) {
            return Err(ArtifactValidateError::WorktaskRoutingSourceUnknown(
                source_work_package_id,
            ));
        }
        for required in [
            "worktask_id",
            "execution_mode",
            "allowed_write_scope",
            "traceability_refs",
            "verification_commands",
        ] {
            if item.get(required).is_none() {
                return Err(ArtifactValidateError::WorktaskRoutingSourceUnknown(
                    source_work_package_id,
                ));
            }
        }
    }
    Ok(())
}

fn validate_coverage_summary(value: Option<&Value>) -> Result<(), ArtifactValidateError> {
    let Some(summary) = value.and_then(Value::as_object) else {
        return Err(ArtifactValidateError::CoverageSummaryMissing);
    };
    for required in ["closed", "uncovered", "exempted"] {
        if !summary.get(required).is_some_and(Value::is_array) {
            return Err(ArtifactValidateError::CoverageSummaryMissing);
        }
    }
    Ok(())
}
