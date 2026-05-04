use cadence_aria::cross_cutting::artifact_projection::compile_plan_projection;
use cadence_aria::cross_cutting::artifact_validate::{ArtifactIndex, projection_validator};
use cadence_aria::cross_cutting::document_ops::read_document_model;
use cadence_aria::protocol::artifacts::{
    ArtifactKind, ArtifactRef, ArtifactStatus, ProjectionKind,
};
use cadence_aria::protocol::projections::ProjectionPayload;
use serde_json::Value;

#[test]
fn plan_projection_compiles_work_packages_dependencies_and_parallelism() {
    let source = read_document_model("tests/fixtures/artifacts/plan.md".as_ref()).expect("plan");
    let source_ref = artifact_ref(
        "art_ref_plan_0001",
        "art_plan_001",
        ArtifactKind::Plan,
        &source,
    );

    let record =
        compile_plan_projection(&source, &source_ref, "N11".to_string()).expect("compile plan");

    assert_eq!(record.projection_kind, ProjectionKind::PlanProjection);
    let ProjectionPayload::PlanProjection(payload) = &record.payload else {
        panic!("expected plan projection payload");
    };
    assert_eq!(payload.work_packages[0].work_package_id, "wt-001");
    assert_eq!(
        payload.work_packages[0].traceability_refs,
        vec![
            "req-001".to_string(),
            "dd-001".to_string(),
            "task-001".to_string()
        ]
    );
    assert_eq!(payload.work_packages[0].acceptance_targets, vec!["ac-001"]);
    assert_eq!(
        payload.dependencies[0].dependency_type.to_string(),
        "blocks"
    );
    assert_eq!(payload.parallelism_groups[0].max_parallel, 1);

    let golden: Value = serde_json::from_str(include_str!(
        "fixtures/artifacts/golden/plan_projection.json"
    ))
    .expect("golden json");
    assert_eq!(serde_json::to_value(payload).expect("payload json"), golden);

    let validation = projection_validator(
        &record,
        &ArtifactIndex::from_active_refs(vec![source_ref]),
        Some("tests/fixtures/artifacts/golden/plan_projection.json".as_ref()),
    )
    .expect("projection validation");
    assert!(validation.valid);
}

#[test]
fn plan_projection_rejects_missing_dependency_endpoint() {
    let source = read_document_model("tests/fixtures/artifacts/plan_bad_dependency.md".as_ref())
        .expect("plan");
    let source_ref = artifact_ref(
        "art_ref_plan_0002",
        "art_plan_002",
        ArtifactKind::Plan,
        &source,
    );

    let error = compile_plan_projection(&source, &source_ref, "N11".to_string())
        .expect_err("missing dependency endpoint should fail");

    assert_eq!(
        error.to_string(),
        "dependency endpoint missing: wt-001 -> wt-999"
    );
}

#[test]
fn plan_projection_accepts_sequential_dependency_alias_as_blocks() {
    let source =
        read_document_model("tests/fixtures/artifacts/plan_sequential_dependency.md".as_ref())
            .expect("plan");
    let source_ref = artifact_ref(
        "art_ref_plan_0003",
        "art_plan_003",
        ArtifactKind::Plan,
        &source,
    );

    let record =
        compile_plan_projection(&source, &source_ref, "N11".to_string()).expect("compile plan");
    let ProjectionPayload::PlanProjection(payload) = &record.payload else {
        panic!("expected plan projection payload");
    };

    assert_eq!(
        payload.dependencies[0].dependency_type.to_string(),
        "blocks"
    );
}

#[test]
fn plan_projection_accepts_finish_to_start_dependency_alias_as_blocks() {
    let source =
        read_document_model("tests/fixtures/artifacts/plan_finish_to_start_dependency.md".as_ref())
            .expect("plan");
    let source_ref = artifact_ref(
        "art_ref_plan_0004",
        "art_plan_004",
        ArtifactKind::Plan,
        &source,
    );

    let record =
        compile_plan_projection(&source, &source_ref, "N11".to_string()).expect("compile plan");
    let ProjectionPayload::PlanProjection(payload) = &record.payload else {
        panic!("expected plan projection payload");
    };

    assert_eq!(
        payload.dependencies[0].dependency_type.to_string(),
        "blocks"
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
