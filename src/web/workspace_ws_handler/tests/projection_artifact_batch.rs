use std::collections::BTreeMap;

use super::*;
use crate::product::models::{
    PlanProjectionBundle, WorkItemPlanCommitState, WorkItemPlanCompileStatus,
    WorkItemProjectionBundle,
};
use crate::product::work_item_contract::{
    build_dependency_contract_graph, canonical_contract_fixture,
};
use crate::product::work_item_projection::{
    CompiledWorkItemProjections, PlanProjectionCompileInput, PlanProjectionCompiler,
    ProjectionValidationReport, WorkItemProjectionCompiler,
};
use crate::product::workspace_engine::ArtifactUpdateEvent;
use crate::web::workspace_ws_types::{
    ArtifactPayload, WorkItemPlanCompileReportPayload, WorkItemRevisionHistoryDto,
};

#[test]
fn work_item_plan_projection_artifact_event_maps_to_structured_ws_update() {
    let (_, work_item_projection) = projection_bundle_fixtures();
    let payload = ArtifactPayload::WorkItemProjection {
        projection: Box::new(work_item_projection),
    };

    let message = ws_artifact_update(12, payload.clone());

    assert_eq!(
        message,
        WsOutMessage::ArtifactUpdate {
            version: 12,
            payload,
        }
    );
    let value = serde_json::to_value(message).unwrap();
    assert_eq!(value["type"], "artifact_update");
    assert_eq!(
        value["work_item_projection"]["canonical_contract_hash"],
        "sha256:handler-contract"
    );
    assert_eq!(
        value["work_item_projection"]["compiler_version"],
        "work-item-projection-v1"
    );
    assert!(value.get("markdown").is_none());
}

#[tokio::test]
async fn work_item_plan_projection_artifact_batch_sorts_and_expands_in_version_order() {
    let (plan_projection, work_item_projection) = projection_bundle_fixtures();
    let mut updates = vec![
        ArtifactUpdateEvent {
            version: 1,
            payload: ArtifactPayload::WorkItemPlanCompileReport {
                compile_report: Box::new(WorkItemPlanCompileReportPayload {
                    compile_id: "compile_handler".to_string(),
                    generation_round_id: "round_handler".to_string(),
                    status: WorkItemPlanCompileStatus::Committed,
                    plan_commit_state: WorkItemPlanCommitState::Committed,
                    work_item_ids: vec!["wi_handler_projection".to_string()],
                    verification_plan_ids: vec![],
                    child_session_ids: vec![],
                    validator_findings: vec![],
                }),
            },
        },
        ArtifactUpdateEvent {
            version: 2,
            payload: ArtifactPayload::WorkItemProjection {
                projection: Box::new(work_item_projection),
            },
        },
        ArtifactUpdateEvent {
            version: 3,
            payload: ArtifactPayload::ProjectionValidation {
                report: Box::new(ProjectionValidationReport { findings: vec![] }),
            },
        },
        ArtifactUpdateEvent {
            version: 4,
            payload: ArtifactPayload::WorkItemRevisionHistory {
                history: Box::new(WorkItemRevisionHistoryDto { entries: vec![] }),
            },
        },
        ArtifactUpdateEvent {
            version: 5,
            payload: ArtifactPayload::WorkItemPlanProjection {
                projection: Box::new(plan_projection),
            },
        },
    ];
    updates.rotate_right(2);
    let (engine_tx, engine_rx) = mpsc::channel(1);
    let (outbound_tx, mut outbound_rx) = mpsc::channel(8);
    let forward = spawn_engine_event_forward_task(
        engine_rx,
        outbound_tx,
        "session_projection_batch".to_string(),
        WorkspaceRunRegistry::default(),
        None,
    );

    engine_tx
        .send(EngineEvent::ArtifactBatchUpdate { updates })
        .await
        .unwrap();
    drop(engine_tx);

    let mut values = Vec::new();
    while let Some(control) = outbound_rx.recv().await {
        let OutboundControl::Text(text) = control else {
            panic!("expected text outbound");
        };
        values.push(serde_json::from_str::<serde_json::Value>(&text).unwrap());
    }
    forward.await.unwrap();

    assert_eq!(
        values
            .iter()
            .map(|value| value["version"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    assert!(
        values
            .iter()
            .all(|value| value["type"] == "artifact_update")
    );
    assert!(values[0].get("compile_report").is_some());
    assert!(values[1].get("work_item_projection").is_some());
    assert!(values[2].get("projection_validation").is_some());
    assert!(values[3].get("work_item_revision_history").is_some());
    assert!(values[4].get("plan_projection").is_some());
}

fn projection_bundle_fixtures() -> (PlanProjectionBundle, WorkItemProjectionBundle) {
    let mut contract = canonical_contract_fixture("wi_handler_projection");
    contract.input_contracts.clear();
    contract.handoff_contract.provided_contract_refs.clear();
    let compiled = WorkItemProjectionCompiler
        .compile(&contract, "work_item_revision_handler_projection")
        .unwrap();
    let work_item_projection = WorkItemProjectionBundle {
        id: "work_item_projection_bundle_handler".to_string(),
        work_item_revision_id: "work_item_revision_handler_projection".to_string(),
        canonical_contract_hash: "sha256:handler-contract".to_string(),
        projection_schema_version: 1,
        compiler_version: "work-item-projection-v1".to_string(),
        human_projection: compiled.human.clone(),
        coder_projection: compiled.coder.clone(),
        reviewer_projection: compiled.reviewer.clone(),
        human_projection_hash: "sha256:handler-human".to_string(),
        coder_projection_hash: "sha256:handler-coder".to_string(),
        reviewer_projection_hash: "sha256:handler-reviewer".to_string(),
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };
    let graph = build_dependency_contract_graph(&[contract]).unwrap();
    let work_items = BTreeMap::from([(
        "wi_handler_projection".to_string(),
        CompiledWorkItemProjections {
            human: compiled.human,
            coder: compiled.coder,
            reviewer: compiled.reviewer,
        },
    )]);
    let compiled_plan = PlanProjectionCompiler
        .compile(PlanProjectionCompileInput {
            plan_id: "plan_handler",
            goal: "Forward projection artifacts",
            split_reason: "Handler batch fixture",
            source_refs: vec!["design_handler".to_string()],
            dependency_graph: &graph,
            work_item_projections: &work_items,
            expected_work_item_revision_ids: BTreeMap::from([(
                "wi_handler_projection".to_string(),
                "work_item_revision_handler_projection".to_string(),
            )]),
        })
        .unwrap();
    (
        PlanProjectionBundle {
            id: "plan_projection_bundle_handler".to_string(),
            plan_revision_id: "plan_revision_handler".to_string(),
            dependency_graph_revision_id: "dependency_graph_revision_handler".to_string(),
            work_item_projection_bundle_refs: vec![work_item_projection.id.clone()],
            human_group_projection: compiled_plan.human,
            coder_group_context: compiled_plan.coder,
            reviewer_group_matrix: compiled_plan.reviewer,
            human_group_projection_hash: "sha256:handler-human-group".to_string(),
            coder_group_context_hash: "sha256:handler-coder-group".to_string(),
            reviewer_group_matrix_hash: "sha256:handler-reviewer-group".to_string(),
            compiler_version: "work-item-projection-v1".to_string(),
            created_at: "2026-07-17T00:00:01Z".to_string(),
        },
        work_item_projection,
    )
}
