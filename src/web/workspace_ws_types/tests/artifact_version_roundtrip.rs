use super::*;

use std::collections::{BTreeMap, HashMap};

use crate::product::models::{PlanProjectionBundle, WorkItemProjectionBundle};
use crate::product::work_item_contract::{
    build_dependency_contract_graph, canonical_contract_fixture,
};
use crate::product::work_item_projection::{
    CompiledWorkItemProjections, PlanProjectionCompileInput, PlanProjectionCompiler,
    ProjectionValidationReport, WorkItemProjectionCompiler,
};
use crate::web::workspace_ws_types::{
    ArtifactVersionSummary, ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus,
    TimelineNodeType, WorkItemHistoryEntryDto, WorkItemHistoryEntryKind,
    WorkItemRevisionHistoryDto, WorkspaceStage, WsProviderConfig,
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
fn workspace_artifact_version_binding_remains_stable_for_story_design_and_work_item() {
    let (_, work_item_projection) = projection_bundle_fixtures();

    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let payload = match workspace_type {
            WorkspaceType::WorkItem => ArtifactPayload::WorkItemProjection {
                projection: Box::new(work_item_projection.clone()),
            },
            _ => ArtifactPayload::Markdown {
                markdown: format!("# {workspace_type:?} artifact"),
                diff: None,
            },
        };
        let source_node_id = format!("node_{workspace_type:?}").to_lowercase();
        let artifact_ref = "artifact_version_007".to_string();
        let snapshot = workspace_snapshot(
            workspace_type.clone(),
            payload.clone(),
            source_node_id.clone(),
            artifact_ref.clone(),
        );

        let value = serde_json::to_value(snapshot).unwrap();
        let restored = serde_json::from_value::<WsOutMessage>(value).unwrap();
        let WsOutMessage::SessionState {
            workspace_type: restored_workspace_type,
            artifact,
            timeline_nodes,
            active_node_id,
            artifact_versions,
            ..
        } = restored
        else {
            panic!("expected session state");
        };

        assert_eq!(restored_workspace_type, workspace_type);
        assert_eq!(artifact, Some(payload));
        assert_eq!(active_node_id.as_deref(), Some(source_node_id.as_str()));
        assert_eq!(
            timeline_nodes[0].artifact_ref.as_deref(),
            Some(artifact_ref.as_str())
        );
        assert_eq!(artifact_versions[0].version, 7);
        assert_eq!(artifact_versions[0].source_node_id, source_node_id);

        if let ArtifactPayload::WorkItemProjection { projection } = &artifact_versions[0].payload {
            assert_eq!(
                projection.canonical_contract_hash,
                work_item_projection.canonical_contract_hash
            );
            assert_eq!(
                projection.compiler_version,
                work_item_projection.compiler_version
            );
        }
    }
}

fn workspace_snapshot(
    workspace_type: WorkspaceType,
    payload: ArtifactPayload,
    source_node_id: String,
    artifact_ref: String,
) -> WsOutMessage {
    WsOutMessage::SessionState {
        session_id: format!("session_{workspace_type:?}").to_lowercase(),
        workspace_type,
        stage: "human_confirm".to_string(),
        superpowers_enabled: true,
        openspec_enabled: true,
        messages: vec![],
        checkpoints: vec![],
        artifact: Some(payload.clone()),
        providers: WsProviderConfig {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
        },
        timeline_nodes: vec![TimelineNode {
            node_id: source_node_id.clone(),
            node_type: TimelineNodeType::HumanConfirm,
            agent: None,
            stage: WorkspaceStage::HumanConfirm,
            round: None,
            status: TimelineNodeStatus::Active,
            title: "Confirm artifact".to_string(),
            summary: None,
            started_at: "2026-07-17T00:00:05Z".to_string(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: Some(artifact_ref),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            retry: None,
        }],
        active_node_id: Some(source_node_id.clone()),
        artifact_versions: vec![ArtifactVersion {
            version: 7,
            payload,
            generated_by: ProviderName::Codex,
            reviewed_by: Some(ProviderName::ClaudeCode),
            review_verdict: Some(ReviewVerdictType::Pass),
            confirmed_by: None,
            is_current: true,
            created_at: "2026-07-17T00:00:04Z".to_string(),
            source_node_id,
        }],
        artifact_version_summaries: Vec::<ArtifactVersionSummary>::new(),
        timeline_node_details: HashMap::new(),
        timeline_node_summaries: HashMap::new(),
        active_run_id: None,
        recoverable_interrupted_run: None,
    }
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
