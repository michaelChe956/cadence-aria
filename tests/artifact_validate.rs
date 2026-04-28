use cadence_aria::cross_cutting::artifact_validate::{
    artifact_validation_rule, canonical_validator, ArtifactContent, ArtifactValidateError,
};
use cadence_aria::protocol::artifacts::ArtifactKind;
use serde_json::json;

#[test]
fn artifact_validation_matrix_matches_phase1_layer_rules() {
    let spec = artifact_validation_rule(ArtifactKind::Spec).expect("spec rule");
    assert!(spec.requires_canonical);
    assert!(spec.requires_projection);
    assert!(!spec.requires_phase1_profile);

    let dispatch = artifact_validation_rule(ArtifactKind::DispatchPackage).expect("dispatch rule");
    assert!(dispatch.requires_canonical);
    assert!(!dispatch.requires_projection);
    assert!(dispatch.requires_phase1_profile);

    let runtime_snapshot =
        artifact_validation_rule(ArtifactKind::RuntimeSnapshot).expect("snapshot rule");
    assert!(runtime_snapshot.requires_canonical);
    assert!(!runtime_snapshot.requires_projection);
    assert!(!runtime_snapshot.requires_phase1_profile);
}

#[test]
fn canonical_validator_rejects_missing_required_field_and_artifact_kind_mismatch() {
    let missing = canonical_validator(
        ArtifactKind::CodingReport,
        &ArtifactContent::Json(json!({
            "artifact_kind": "coding_report",
            "files_modified": ["src/lib.rs"],
            "commands_run": [],
            "candidate_traceability_refs": [],
            "status": "completed"
        })),
    )
    .expect_err("missing worktask_id");
    assert_eq!(
        missing,
        ArtifactValidateError::CanonicalMissingField {
            field: "worktask_id".to_string(),
            artifact_kind: ArtifactKind::CodingReport
        }
    );

    let mismatch = canonical_validator(
        ArtifactKind::CodingReport,
        &ArtifactContent::Json(json!({
            "artifact_kind": "testing_report",
            "worktask_id": "work_001",
            "files_modified": ["src/lib.rs"],
            "commands_run": [],
            "candidate_traceability_refs": [],
            "status": "completed"
        })),
    )
    .expect_err("artifact kind mismatch");
    assert_eq!(
        mismatch,
        ArtifactValidateError::CanonicalTypeMismatch {
            field: "artifact_kind".to_string(),
            expected: "coding_report".to_string(),
            got: "testing_report".to_string()
        }
    );
}

#[test]
fn canonical_validator_rejects_invalid_json_text_before_schema_checks() {
    let error = canonical_validator(
        ArtifactKind::CodingReport,
        &ArtifactContent::JsonText("{ not json".to_string()),
    )
    .expect_err("invalid json");

    assert_eq!(
        error,
        ArtifactValidateError::CanonicalTypeMismatch {
            field: "$".to_string(),
            expected: "valid_json".to_string(),
            got: "invalid_json".to_string()
        }
    );
}

#[test]
fn canonical_validator_does_not_validate_phase1_profile_fields() {
    let result = canonical_validator(
        ArtifactKind::CodingReport,
        &ArtifactContent::Json(json!({
            "artifact_kind": "coding_report",
            "worktask_id": "work_001",
            "files_modified": ["src/lib.rs"],
            "commands_run": [],
            "candidate_traceability_refs": [],
            "status": "completed",
            "_aria": {}
        })),
    )
    .expect("canonical layer should ignore _aria internals");

    assert!(result.valid);
    assert!(result.warnings.is_empty());
}

#[test]
fn canonical_validator_rejects_wrong_content_family_for_markdown_artifact() {
    let error = canonical_validator(
        ArtifactKind::Spec,
        &ArtifactContent::Json(json!({
            "artifact_kind": "spec"
        })),
    )
    .expect_err("spec must be markdown canonical content");

    assert_eq!(
        error,
        ArtifactValidateError::CanonicalTypeMismatch {
            field: "content".to_string(),
            expected: "markdown".to_string(),
            got: "json".to_string()
        }
    );
}
