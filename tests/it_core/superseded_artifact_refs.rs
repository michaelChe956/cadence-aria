use cadence_aria::cross_cutting::artifact_projection::compile_spec_projection;
use cadence_aria::cross_cutting::artifact_validate::{
    ArtifactIndex, ArtifactValidateError, projection_validator, record_superseded_artifact_ref,
    validate_input_artifact_ref,
};
use cadence_aria::cross_cutting::document_ops::read_document_model;
use cadence_aria::protocol::artifacts::{ArtifactKind, ArtifactRef, ArtifactStatus};
use serde_json::json;

#[test]
fn backtrack_records_old_artifact_ref_in_task_runtime_state() {
    let mut task_state = json!({
        "task_id": "task_001",
        "superseded_artifact_refs": []
    });

    record_superseded_artifact_ref(&mut task_state, "art_ref_spec_0001".to_string())
        .expect("record superseded ref");
    record_superseded_artifact_ref(&mut task_state, "art_ref_spec_0001".to_string())
        .expect("dedupe superseded ref");

    assert_eq!(
        task_state["superseded_artifact_refs"],
        json!(["art_ref_spec_0001"])
    );
}

#[test]
fn node_input_rejects_artifact_ref_listed_as_superseded_even_if_status_is_active() {
    let artifact_ref = artifact_ref("art_ref_spec_0001", ArtifactStatus::Active);
    let index = ArtifactIndex::with_superseded_refs(
        vec![artifact_ref.clone()],
        vec!["art_ref_spec_0001".to_string()],
    );

    let error = validate_input_artifact_ref(&artifact_ref, &index)
        .expect_err("superseded ref must not enter a node");

    assert_eq!(
        error,
        ArtifactValidateError::InvalidInputSuperseded("art_ref_spec_0001".to_string())
    );
}

#[test]
fn node_input_rejects_artifact_ref_marked_superseded_by_status() {
    let artifact_ref = artifact_ref("art_ref_design_0001", ArtifactStatus::Superseded);
    let index = ArtifactIndex::from_active_refs(vec![artifact_ref.clone()]);

    let error = validate_input_artifact_ref(&artifact_ref, &index)
        .expect_err("superseded artifact status must be rejected");

    assert_eq!(
        error,
        ArtifactValidateError::InvalidInputSuperseded("art_ref_design_0001".to_string())
    );
}

#[test]
fn projection_validator_rejects_superseded_source_artifact() {
    let source = read_document_model("tests/fixtures/artifacts/spec.md".as_ref()).expect("spec");
    let source_ref = ArtifactRef {
        artifact_ref_id: "art_ref_spec_0001".to_string(),
        artifact_id: "art_spec_001".to_string(),
        artifact_kind: ArtifactKind::Spec,
        version: 1,
        path: source.source_path.clone(),
        sha256: source.sha256.clone(),
        status: ArtifactStatus::Superseded,
    };
    let record =
        compile_spec_projection(&source, &source_ref, "N05".to_string()).expect("projection");
    let index = ArtifactIndex::from_active_refs(vec![source_ref]);

    let error = projection_validator(&record, &index, None)
        .expect_err("projection source must not be superseded");

    assert_eq!(
        error,
        ArtifactValidateError::InvalidInputSuperseded("art_ref_spec_0001".to_string())
    );
}

fn artifact_ref(artifact_ref_id: &str, status: ArtifactStatus) -> ArtifactRef {
    ArtifactRef {
        artifact_ref_id: artifact_ref_id.to_string(),
        artifact_id: artifact_ref_id.replace("art_ref_", "art_"),
        artifact_kind: ArtifactKind::Spec,
        version: 1,
        path: String::new(),
        sha256: String::new(),
        status,
    }
}
