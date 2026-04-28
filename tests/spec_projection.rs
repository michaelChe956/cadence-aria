use cadence_aria::cross_cutting::artifact_projection::compile_spec_projection;
use cadence_aria::cross_cutting::artifact_validate::{projection_validator, ArtifactIndex};
use cadence_aria::cross_cutting::document_ops::read_document_model;
use cadence_aria::protocol::artifacts::{
    ArtifactKind, ArtifactRef, ArtifactStatus, ProjectionKind,
};
use cadence_aria::protocol::projections::ProjectionPayload;
use serde_json::Value;

#[test]
fn spec_projection_compiles_from_document_model_and_matches_golden_json() {
    let source = read_document_model("tests/fixtures/artifacts/spec.md".as_ref()).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_0001",
        "art_spec_001",
        ArtifactKind::Spec,
        &source,
    );

    let record =
        compile_spec_projection(&source, &source_ref, "N05".to_string()).expect("compile spec");

    assert_eq!(record.projection_kind, ProjectionKind::SpecProjection);
    assert_eq!(record.source_artifact_hash, source.sha256);
    assert_eq!(
        record.projection_id,
        "proj_spec_projection_art_spec_001_0001"
    );

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.functional_requirements[0].requirement_id, "req-001");
    assert_eq!(payload.functional_requirements[1].requirement_id, "req-002");
    assert_eq!(
        payload.success_criteria[0].related_requirement_ids,
        vec!["req-001".to_string(), "req-002".to_string()]
    );

    let golden: Value = serde_json::from_str(include_str!(
        "fixtures/artifacts/golden/spec_projection.json"
    ))
    .expect("golden json");
    assert_eq!(serde_json::to_value(payload).expect("payload json"), golden);

    let index = ArtifactIndex::from_active_refs(vec![source_ref]);
    let validation = projection_validator(
        &record,
        &index,
        Some("tests/fixtures/artifacts/golden/spec_projection.json".as_ref()),
    )
    .expect("projection validation");
    assert!(validation.valid);
}

#[test]
fn spec_projection_rejects_unknown_requirement_reference() {
    let source =
        read_document_model("tests/fixtures/artifacts/spec_unknown_ref.md".as_ref()).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_0002",
        "art_spec_002",
        ArtifactKind::Spec,
        &source,
    );

    let error = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect_err("unknown REQ reference should fail");

    assert_eq!(
        error.to_string(),
        "unknown projection reference req-999 in success_criteria"
    );
}

fn artifact_ref(
    artifact_ref_id: &str,
    artifact_id: &str,
    artifact_kind: ArtifactKind,
    source: &cadence_aria::protocol::document_ops::DocumentModel,
) -> ArtifactRef {
    ArtifactRef {
        artifact_ref_id: artifact_ref_id.to_string(),
        artifact_id: artifact_id.to_string(),
        artifact_kind,
        version: 1,
        path: source.source_path.clone(),
        sha256: source.sha256.clone(),
        status: ArtifactStatus::Active,
    }
}
