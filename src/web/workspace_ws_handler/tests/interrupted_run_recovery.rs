use super::*;
use crate::product::lifecycle_store::{CreateStorySpecInput, CreateWorkspaceSessionInput};
use crate::product::models::{WorkspaceSessionRecord, WorkspaceType};
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    WorkspaceStage as WsWorkspaceStage,
};

#[test]
fn retry_interrupted_run_is_only_valid_in_prepare_context() {
    let msg = WsInMessage::RetryInterruptedRun {
        failed_node_id: "timeline_node_054".to_string(),
    };

    assert!(is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::Legacy,
        &msg,
        &WorkspaceStage::PrepareContext
    ));
    assert!(!is_message_valid_for_stage_with_flow(
        WorkItemPlanFlowKind::Legacy,
        &msg,
        &WorkspaceStage::Running
    ));
    assert_eq!(message_type(&msg), "retry_interrupted_run");
}

#[test]
fn interrupted_recovery_outcome_selects_provider_run_kind() {
    assert!(matches!(
        provider_run_kind_for_interrupted_recovery(InterruptedRunRecoveryOutcome::Review),
        ProviderRunKind::ReviewOnly
    ));
    assert!(matches!(
        provider_run_kind_for_interrupted_recovery(
            InterruptedRunRecoveryOutcome::WorkItemDraftGeneration
        ),
        ProviderRunKind::WorkItemPlanDraft { feedback: None }
    ));
}

#[tokio::test]
async fn interrupted_recovery_provider_start_failure_returns_to_prepare_context() {
    let root = tempfile::tempdir().expect("checkpoint root");
    let checkpoint_store = Arc::new(CheckpointStore::new(root.path().join("checkpoints")));
    let (tx, _rx) = mpsc::channel(8);
    let session = WorkspaceSession {
        session_id: "session_retry_start_failure".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "story_spec_0001".to_string(),
        workspace_type: WorkspaceType::Story,
        stage: WorkspaceStage::CrossReview,
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
    };
    let mut workspace_engine = WorkspaceEngine::new(checkpoint_store, tx, session);
    workspace_engine.timeline_nodes = vec![handler_interrupted_timeline_node(
        "timeline_node_006",
        TimelineNodeType::ReviewerRun,
        TimelineNodeStatus::Active,
        WsWorkspaceStage::CrossReview,
    )];
    workspace_engine.active_node_id = Some("timeline_node_006".to_string());
    let engine = Arc::new(Mutex::new(workspace_engine));

    finish_interrupted_recovery_spawn_error(&engine, "provider unavailable: Codex").await;

    let engine = engine.lock().await;
    assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
    assert_eq!(engine.timeline_nodes[0].status, TimelineNodeStatus::Failed);
    assert_eq!(
        engine.timeline_nodes[0].summary.as_deref(),
        Some("provider unavailable: Codex")
    );
}

#[tokio::test]
async fn retry_interrupted_review_starts_reviewer_provider() {
    let root = tempfile::tempdir().expect("app root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Interrupted review".to_string(),
            aggregate_codebase: None,
        })
        .expect("story spec");
    let session_record: WorkspaceSessionRecord = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id,
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("workspace session");
    let checkpoint_store = Arc::new(CheckpointStore::new(root.path().join("checkpoints")));
    let (engine_tx, _engine_rx) = mpsc::channel::<EngineEvent>(64);
    let mut session = WorkspaceSession::from_record(session_record.clone());
    session.stage = WorkspaceStage::PrepareContext;
    let payload = ArtifactPayload::Markdown {
        markdown: "# Story Spec\n".to_string(),
        diff: None,
    };
    session.artifact = Some(payload.clone());
    let mut workspace_engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle, engine_tx, session);
    workspace_engine.artifact_versions = vec![ArtifactVersion {
        version: 1,
        payload,
        generated_by: ProviderName::ClaudeCode,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-07-11T17:00:00Z".to_string(),
        source_node_id: "timeline_node_002".to_string(),
    }];
    workspace_engine.timeline_nodes = vec![
        handler_interrupted_timeline_node(
            "timeline_node_002",
            TimelineNodeType::AuthorRun,
            TimelineNodeStatus::Completed,
            WsWorkspaceStage::Running,
        ),
        handler_interrupted_timeline_node(
            "timeline_node_004",
            TimelineNodeType::ReviewerRun,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::CrossReview,
        ),
        handler_interrupted_timeline_node(
            "timeline_node_005",
            TimelineNodeType::AbortedByDisconnect,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::PrepareContext,
        ),
    ];
    let engine = Arc::new(Mutex::new(workspace_engine));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Codex,
        Arc::new(PromptRecordingProvider { input_tx }),
    );
    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(registry),
        engine: engine.clone(),
        current_run: current_run.clone(),
        workspace_runs: workspace_runs.clone(),
        session_id: session_record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths,
        session_record: session_record.clone(),
    };
    let (outbound_tx, _outbound_rx) = mpsc::channel::<OutboundControl>(64);

    handle_workspace_inbound_message(
        WorkspaceInboundContext {
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
        },
        WsInMessage::RetryInterruptedRun {
            failed_node_id: "timeline_node_004".to_string(),
        },
    )
    .await;

    let input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("reviewer provider should start")
        .expect("reviewer input");
    assert_eq!(
        input.role,
        crate::protocol::contracts::AdapterRole::Reviewer
    );
}

fn handler_interrupted_timeline_node(
    node_id: &str,
    node_type: TimelineNodeType,
    status: TimelineNodeStatus,
    stage: WsWorkspaceStage,
) -> TimelineNode {
    TimelineNode {
        node_id: node_id.to_string(),
        node_type,
        agent: Some(ProviderName::Codex),
        stage,
        round: Some(1),
        status,
        title: node_id.to_string(),
        summary: Some("连接断开，运行已中止".to_string()),
        started_at: "2026-07-11T17:00:00Z".to_string(),
        completed_at: Some("2026-07-11T17:01:00Z".to_string()),
        duration_ms: Some(60_000),
        artifact_ref: Some("artifact_current".to_string()),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        retry: None,
    }
}
