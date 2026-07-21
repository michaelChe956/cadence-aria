use super::*;

use std::collections::BTreeMap;

use crate::product::models::{
    AmendmentResumeMode, AmendmentResumeTarget, PlanAmendmentManifest, PlanProjectionBundle,
    WorkItemProjectionBundle,
};
use crate::product::work_item_contract::{
    build_dependency_contract_graph, canonical_contract_fixture,
};
use crate::product::work_item_projection::{
    CompiledWorkItemProjections, PlanProjectionCompileInput, PlanProjectionCompiler,
    ProjectionValidationReport, WorkItemProjectionCompiler,
};
use crate::web::workspace_ws_types::{
    WorkItemHistoryEntryDto, WorkItemHistoryEntryKind, WorkItemRevisionHistoryDto,
};

#[test]
fn artifact_version_roundtrips_with_markdown_payload() {
    let version = ArtifactVersion {
        version: 1,
        payload: ArtifactPayload::Markdown {
            markdown: "# Artifact version\n".to_string(),
            diff: Some("diff".to_string()),
        },
        generated_by: ProviderName::ClaudeCode,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-06-01T00:00:00Z".to_string(),
        source_node_id: "node_001".to_string(),
    };
    let json = serde_json::to_value(&version).unwrap();
    assert_eq!(json["markdown"], "# Artifact version\n");
    assert_eq!(json["diff"], "diff");
    assert!(!json.as_object().unwrap().contains_key("payload"));

    let back: ArtifactVersion = serde_json::from_value(json).unwrap();
    assert_eq!(back, version);
}

#[test]
fn work_item_plan_projection_artifact_roundtrips_with_unique_flat_keys() {
    let (plan_projection, work_item_projection) = projection_bundle_fixtures();
    let cases = [
        (
            ArtifactPayload::WorkItemPlanProjection {
                projection: Box::new(plan_projection.clone()),
            },
            "plan_projection",
        ),
        (
            ArtifactPayload::WorkItemProjection {
                projection: Box::new(work_item_projection.clone()),
            },
            "work_item_projection",
        ),
        (
            ArtifactPayload::WorkItemRevisionHistory {
                history: Box::new(WorkItemRevisionHistoryDto {
                    entries: vec![WorkItemHistoryEntryDto {
                        kind: WorkItemHistoryEntryKind::WorkItemRevision,
                        id: "history_entry_0001".to_string(),
                        logical_work_item_id: "wi_projection".to_string(),
                        related_revision_id: Some("work_item_revision_0001".to_string()),
                        summary: "Initial work item revision".to_string(),
                        created_at: "2026-07-17T00:00:04Z".to_string(),
                    }],
                }),
            },
            "work_item_revision_history",
        ),
        (
            ArtifactPayload::ProjectionValidation {
                report: Box::new(ProjectionValidationReport { findings: vec![] }),
            },
            "projection_validation",
        ),
    ];

    for (payload, expected_key) in cases {
        let value = serde_json::to_value(&payload).unwrap();
        assert!(value.get(expected_key).is_some());
        assert!(value.get("type").is_none());
        assert_eq!(
            serde_json::from_value::<ArtifactPayload>(value).unwrap(),
            payload
        );
    }

    let update = serde_json::to_value(WsOutMessage::ArtifactUpdate {
        version: 9,
        payload: ArtifactPayload::WorkItemPlanProjection {
            projection: Box::new(plan_projection),
        },
    })
    .unwrap();
    assert_eq!(update["type"], "artifact_update");
    assert!(update.get("plan_projection").is_some());
    assert!(update.get("projection").is_none());
}

#[test]
fn plan_repair_amendment_manifest_artifact_roundtrips_with_typed_key() {
    let manifest = PlanAmendmentManifest {
        id: "plan_amendment_0001".to_string(),
        repair_request_id: "plan_repair_request_0001".to_string(),
        previous_plan_revision_id: "plan_revision_0001".to_string(),
        new_plan_revision_id: "plan_revision_0002".to_string(),
        revised_work_items: BTreeMap::new(),
        superseded_revisions: vec![],
        dependency_graph_changes: vec![],
        contract_deltas: vec![],
        unaffected_units: vec!["wi_unaffected".to_string()],
        revalidation_required_units: vec!["wi_revalidate".to_string()],
        stale_units: vec![],
        replacement_units: BTreeMap::new(),
        resume_target: AmendmentResumeTarget {
            logical_work_item_id: "wi_revalidate".to_string(),
            mode: AmendmentResumeMode::Revalidate,
        },
        created_at: "2026-07-18T00:00:00Z".to_string(),
    };
    let payload = ArtifactPayload::PlanAmendmentManifest {
        manifest: Box::new(manifest),
    };

    let value = serde_json::to_value(&payload).unwrap();
    assert!(value.get("plan_amendment_manifest").is_some());
    assert!(value.get("type").is_none());
    assert_eq!(
        serde_json::from_value::<ArtifactPayload>(value).unwrap(),
        payload
    );
}

fn projection_bundle_fixtures() -> (PlanProjectionBundle, WorkItemProjectionBundle) {
    let mut contract = canonical_contract_fixture("wi_projection");
    contract.input_contracts.clear();
    contract.handoff_contract.provided_contract_refs.clear();
    let compiled_work_item = WorkItemProjectionCompiler
        .compile(&contract, "work_item_revision_0001")
        .unwrap();
    let work_item_projection = WorkItemProjectionBundle {
        id: "work_item_projection_bundle_0001".to_string(),
        work_item_revision_id: "work_item_revision_0001".to_string(),
        canonical_contract_hash: "sha256:canonical-contract".to_string(),
        projection_schema_version: 1,
        compiler_version: "work-item-projection-v1".to_string(),
        human_projection: compiled_work_item.human.clone(),
        coder_projection: compiled_work_item.coder.clone(),
        reviewer_projection: compiled_work_item.reviewer.clone(),
        human_projection_hash: "sha256:human".to_string(),
        coder_projection_hash: "sha256:coder".to_string(),
        reviewer_projection_hash: "sha256:reviewer".to_string(),
        created_at: "2026-07-17T00:00:02Z".to_string(),
    };
    let graph = build_dependency_contract_graph(&[contract]).unwrap();
    let work_item_projections = BTreeMap::from([(
        "wi_projection".to_string(),
        CompiledWorkItemProjections {
            human: compiled_work_item.human,
            coder: compiled_work_item.coder,
            reviewer: compiled_work_item.reviewer,
        },
    )]);
    let compiled_plan = PlanProjectionCompiler
        .compile(PlanProjectionCompileInput {
            plan_id: "plan_0001",
            goal: "Expose projection artifacts",
            split_reason: "Single work item fixture",
            source_refs: vec!["design_spec_0001".to_string()],
            dependency_graph: &graph,
            work_item_projections: &work_item_projections,
            expected_work_item_revision_ids: BTreeMap::from([(
                "wi_projection".to_string(),
                "work_item_revision_0001".to_string(),
            )]),
        })
        .unwrap();
    let plan_projection = PlanProjectionBundle {
        id: "plan_projection_bundle_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        dependency_graph_revision_id: "dependency_graph_revision_0001".to_string(),
        work_item_projection_bundle_refs: vec![work_item_projection.id.clone()],
        human_group_projection: compiled_plan.human,
        coder_group_context: compiled_plan.coder,
        reviewer_group_matrix: compiled_plan.reviewer,
        human_group_projection_hash: "sha256:human-group".to_string(),
        coder_group_context_hash: "sha256:coder-group".to_string(),
        reviewer_group_matrix_hash: "sha256:reviewer-group".to_string(),
        compiler_version: "work-item-projection-v1".to_string(),
        created_at: "2026-07-17T00:00:03Z".to_string(),
    };

    (plan_projection, work_item_projection)
}
