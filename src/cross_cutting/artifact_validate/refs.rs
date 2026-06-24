use super::types::{ArtifactIndex, ArtifactValidateError, ValidationResult, json_type_name};
use crate::protocol::artifacts::{ArtifactRef, ArtifactStatus};
use crate::protocol::enums::ArtifactRefId;
use serde_json::Value;

pub fn validate_input_artifact_ref(
    artifact_ref: &ArtifactRef,
    artifact_index: &ArtifactIndex,
) -> Result<ValidationResult, ArtifactValidateError> {
    if artifact_ref.status == ArtifactStatus::Superseded
        || artifact_index.is_superseded(&artifact_ref.artifact_ref_id)
    {
        return Err(ArtifactValidateError::InvalidInputSuperseded(
            artifact_ref.artifact_ref_id.clone(),
        ));
    }
    if !artifact_index
        .active_artifact_ref_ids
        .contains(&artifact_ref.artifact_ref_id)
    {
        return Err(ArtifactValidateError::ProjectionSourceNotFound(
            artifact_ref.artifact_ref_id.clone(),
        ));
    }
    Ok(ValidationResult::valid())
}

pub fn record_superseded_artifact_ref(
    task_runtime_state: &mut Value,
    artifact_ref_id: ArtifactRefId,
) -> Result<ValidationResult, ArtifactValidateError> {
    let state_type = json_type_name(task_runtime_state);
    let object = task_runtime_state.as_object_mut().ok_or_else(|| {
        ArtifactValidateError::CanonicalTypeMismatch {
            field: "$".to_string(),
            expected: "json_object".to_string(),
            got: state_type.to_string(),
        }
    })?;
    let refs = object
        .entry("superseded_artifact_refs")
        .or_insert_with(|| Value::Array(Vec::new()));
    let refs_type = json_type_name(refs);
    let refs = refs
        .as_array_mut()
        .ok_or_else(|| ArtifactValidateError::CanonicalTypeMismatch {
            field: "superseded_artifact_refs".to_string(),
            expected: "array".to_string(),
            got: refs_type.to_string(),
        })?;
    if !refs
        .iter()
        .any(|existing| existing.as_str() == Some(artifact_ref_id.as_str()))
    {
        refs.push(Value::String(artifact_ref_id));
    }
    Ok(ValidationResult::valid())
}
