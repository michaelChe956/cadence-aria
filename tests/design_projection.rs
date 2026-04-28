use cadence_aria::cross_cutting::artifact_projection::compile_design_projection;
use cadence_aria::cross_cutting::artifact_validate::{projection_validator, ArtifactIndex};
use cadence_aria::cross_cutting::document_ops::read_document_model;
use cadence_aria::protocol::artifacts::{
    ArtifactKind, ArtifactRef, ArtifactStatus, ProjectionKind,
};
use cadence_aria::protocol::projections::ProjectionPayload;
use serde_json::Value;

#[test]
fn design_projection_compiles_decisions_components_and_risks() {
    let source =
        read_document_model("tests/fixtures/artifacts/design.md".as_ref()).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_0001",
        "art_design_001",
        ArtifactKind::Design,
        &source,
    );

    let record =
        compile_design_projection(&source, &source_ref, "N07".to_string()).expect("compile design");

    assert_eq!(record.projection_kind, ProjectionKind::DesignProjection);
    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.design_decisions[0].design_decision_id, "dd-001");
    assert_eq!(
        payload.design_decisions[0].text,
        "REPL 只作为客户端，daemon 是 runtime truth。"
    );
    assert_eq!(payload.shared_components[0].component_id, "cmp-001");
    assert_eq!(payload.risk_refs[0].severity.to_string(), "high");
    assert_eq!(
        payload.risk_refs[0].related_design_decision_ids,
        vec!["dd-001".to_string()]
    );

    let golden: Value = serde_json::from_str(include_str!(
        "fixtures/artifacts/golden/design_projection.json"
    ))
    .expect("golden json");
    assert_eq!(serde_json::to_value(payload).expect("payload json"), golden);

    let validation = projection_validator(
        &record,
        &ArtifactIndex::from_active_refs(vec![source_ref]),
        Some("tests/fixtures/artifacts/golden/design_projection.json".as_ref()),
    )
    .expect("projection validation");
    assert!(validation.valid);
}

#[test]
fn design_projection_requires_design_decisions() {
    let source =
        read_document_model("tests/fixtures/artifacts/design_missing_decision.md".as_ref())
            .expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_0002",
        "art_design_002",
        ArtifactKind::Design,
        &source,
    );

    let error = compile_design_projection(&source, &source_ref, "N07".to_string())
        .expect_err("missing decisions should fail");

    assert_eq!(
        error.to_string(),
        "missing required projection section 设计决策"
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
