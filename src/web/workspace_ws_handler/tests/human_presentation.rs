use std::collections::BTreeMap;

use super::*;
use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
use crate::product::models::{
    PlanProjectionBundle, WorkItemPlanLineage, WorkspaceSessionRecord, WorkspaceType,
};
use crate::product::work_item_projection::{
    CoderGroupContext, HumanGroupProjection, ReviewerGroupMatrix,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::workspace_ws_types::HumanPresentationScopeDto;

#[test]
fn human_presentation_save_is_stage_independent_and_non_plan_workspaces_stay_unsupported() {
    let message = WsInMessage::SaveHumanPresentationRevision {
        source_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        scope: HumanPresentationScopeDto::Plan,
        supersedes: None,
        human_summary: "说明".to_string(),
        why_split: None,
        dependency_explanation: vec![],
        risk_explanation: vec![],
        source_refs: vec![],
    };
    assert!(!requires_stage_validation(&message));
    assert_eq!(message_type(&message), "save_human_presentation_revision");

    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let root = tempfile::tempdir().unwrap();
        let (event_tx, _event_rx) = mpsc::channel(1);
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
            event_tx,
            WorkspaceSession {
                session_id: format!("session_{workspace_type:?}"),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "entity_0001".to_string(),
                workspace_type,
                stage: WorkspaceStage::Completed,
                messages: vec![],
                artifact: None,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
                provisional_reviewer_provider: None,
                reviewer_enabled_at_start: None,
                superpowers_enabled: false,
                openspec_enabled: false,
                session_status: crate::product::models::WorkspaceSessionStatus::Running,
                flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy,
                run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
                run_history: crate::product::work_item_plan_policy::RunHistory::default(),
                review_invocation_scope: None,
                human_gate_snapshot: None,
                repair_reservation: None,
                policy_diagnostics: vec![],
                provider_start_ledger: vec![],
                single_candidate_phase: None,
                work_item_plan_source_revision_ref: None,
                plan_candidate_ir_ref: None,
                mechanical_report_ref: None,
                publication_provenance_ref: None,
                approval_attempt_id: None,
                approved_at: None,
                compile_reservation: None,
                provider_conversations: vec![],
                repository_path: None,
            },
        );
        let error = engine
            .save_human_presentation_revision_command(
                crate::product::workspace_engine::SaveHumanPresentationRevision {
                    source_projection_bundle_id: "projection_0001".to_string(),
                    scope: crate::product::workspace_engine::HumanPresentationScope::Plan,
                    supersedes: None,
                    human_summary: "说明".to_string(),
                    why_split: None,
                    dependency_explanation: vec![],
                    risk_explanation: vec![],
                    source_refs: vec![],
                },
            )
            .unwrap_err();
        assert!(matches!(
            error,
            crate::product::workspace_engine::WorkspaceEngineError::InvalidHumanPresentationTarget
        ));
    }
}

#[tokio::test]
async fn human_presentation_save_handler_acknowledges_success_and_returns_recoverable_conflict() {
    let root = tempfile::tempdir().unwrap();
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_plan_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .unwrap();
    let store = WorkItemRevisionStore::new(app_paths.clone());
    let plan = WorkItemPlanLineage {
        id: "work_item_plan_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        story_spec_refs: vec![],
        design_spec_refs: vec![],
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    };
    store.put_plan_lineage(&plan).unwrap();
    store
        .put_plan_projection_bundle(&plan, &plan_projection_bundle())
        .unwrap();
    let checkpoint_store = Arc::new(CheckpointStore::new(root.path().join("checkpoints")));
    let (engine_tx, _engine_rx) = mpsc::channel(8);
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle,
        engine_tx,
        WorkspaceSession::from_record(session_record.clone()),
    )));
    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(ProviderRegistry::new()),
        engine: engine.clone(),
        current_run: current_run.clone(),
        workspace_runs: workspace_runs.clone(),
        session_id: session_record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths,
        session_record: WorkspaceSessionRecord {
            ..session_record.clone()
        },
    };
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<OutboundControl>(8);
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        ),
        engine,
        run_context,
        outbound_tx,
        current_run,
        workspace_runs,
        session_id: session_record.id,
    };
    let command = || WsInMessage::SaveHumanPresentationRevision {
        source_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        scope: HumanPresentationScopeDto::Plan,
        supersedes: None,
        human_summary: "更容易理解的说明".to_string(),
        why_split: Some("按契约边界拆分".to_string()),
        dependency_explanation: vec![],
        risk_explanation: vec![],
        source_refs: vec!["story:001".to_string()],
    };

    handle_workspace_inbound_message(context.clone(), command()).await;
    let first = receive_outbound(&mut outbound_rx).await;
    let first_revision = match first {
        WsOutMessage::HumanPresentationRevisionSaved { revision } => revision,
        other => panic!("expected presentation ack, got {other:?}"),
    };
    assert!(!first_revision.normative);
    assert!(!first_revision.used_by_provider);

    handle_workspace_inbound_message(context, command()).await;
    let second = receive_outbound(&mut outbound_rx).await;
    assert!(matches!(
        second,
        WsOutMessage::HumanPresentationRevisionSaveFailed {
            source_projection_bundle_id,
            message,
        } if source_projection_bundle_id == "plan_projection_bundle_0001"
            && message.contains("supersedes")
    ));
}

async fn receive_outbound(outbound_rx: &mut mpsc::Receiver<OutboundControl>) -> WsOutMessage {
    let control = tokio::time::timeout(std::time::Duration::from_secs(1), outbound_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let OutboundControl::Text(text) = control else {
        panic!("expected text outbound");
    };
    serde_json::from_str(&text).unwrap()
}

fn plan_projection_bundle() -> PlanProjectionBundle {
    PlanProjectionBundle {
        id: "plan_projection_bundle_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        dependency_graph_revision_id: "dependency_graph_revision_0001".to_string(),
        work_item_projection_bundle_refs: vec![],
        human_group_projection: HumanGroupProjection {
            plan_id: "work_item_plan_0001".to_string(),
            goal: "Goal".to_string(),
            split_reason: "Split".to_string(),
            work_items: vec![],
            contract_flow: vec![],
            risks: vec![],
            source_refs: vec!["story:001".to_string()],
            normative: false,
            used_by_provider: false,
        },
        coder_group_context: CoderGroupContext {
            plan_id: "work_item_plan_0001".to_string(),
            ordered_logical_work_item_ids: vec![],
            dependency_edges: vec![],
            group_write_scopes: BTreeMap::new(),
        },
        reviewer_group_matrix: ReviewerGroupMatrix {
            plan_id: "work_item_plan_0001".to_string(),
            work_items: vec![],
            dependency_edges: vec![],
            design_traceability_refs: vec![],
        },
        human_group_projection_hash: "human".to_string(),
        coder_group_context_hash: "coder".to_string(),
        reviewer_group_matrix_hash: "reviewer".to_string(),
        compiler_version: "projection-v1".to_string(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
    }
}
