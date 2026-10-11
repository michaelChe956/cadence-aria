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
        logical_launch_fingerprints: vec![],
        superseded_native_sessions: vec![],
        lc_resume_recovery: None,
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
            reviewer_provider: Some(ProviderName::Codex),

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
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine.clone(),
        WorkspaceRunRegistry::default(),
        session_record.id.clone(),
        app_paths,
        session_record.clone(),
    );
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

/// A2b（Task 2.6 / tasks.md 2.6，设计 §4.3/§4.5）：LC 等待占用旧 provider
/// 失败节点时，普通旁路一律后端真实入口拒收——`start_generation` 普通参数
/// 与 `retry_interrupted_run` 均 typed 拒；`recoverable_interrupted_run` 不
/// 投影被等待占用的节点（权威在后端入口，非 UI disabled）。
#[tokio::test]
async fn anchor_t07_waiting_blocks_plain_generation_and_interrupted_retry() {
    use crate::product::models::{
        LcRecoveryBusinessContext, LcRecoveryRunContext, LcResumeRecoveryRecord,
        LcResumeRecoveryState, ProviderConversationRole,
    };

    let root = tempfile::tempdir().expect("app root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Waiting blocks bypass".to_string(),
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
            reviewer_provider: Some(ProviderName::Codex),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("workspace session");

    // 与 live mismatch 同源落 waiting recovery：占用失败的 reviewer 节点
    // （timeline_node_004，下方 timeline 为既有断连恢复形态）。
    let expected = lifecycle
        .get_workspace_session(&session_record.id)
        .expect("expected record");
    let waiting = LcResumeRecoveryRecord {
        schema_version: 1,
        recovery_id: "recovery_anchor_t07".to_string(),
        failed_provider_node_id: "timeline_node_004".to_string(),
        reason_code: "resume_fingerprint_mismatch".to_string(),
        role: ProviderConversationRole::Reviewer,
        provider: ProviderName::Codex,
        superseded_native_session_id: Some("native-t07".to_string()),
        previous_fingerprint_digest: Some("sha256:t07".to_string()),
        previous_session_status: expected.status.clone(),
        previous_stage: "cross_review".to_string(),
        run_context: LcRecoveryRunContext::ReviewerRevision {
            business: LcRecoveryBusinessContext {
                entity_id: session_record.entity_id.clone(),
                plan_id: None,
                workspace_type: WorkspaceType::Story,
                business_repository_physical_id: None,
                business_repository_logical_id: None,
                business_checkout_id: None,
                business_repository_path: None,
                source_story_ids: Vec::new(),
                source_design_ids: Vec::new(),
                plan_options_ref: None,
                artifact_refs: Vec::new(),
                review_refs: Vec::new(),
                author_provider: ProviderName::ClaudeCode,
                effective_reviewer_provider: Some(ProviderName::Codex),
                original_node_id: Some("timeline_node_004".to_string()),
                original_role_run_seq: None,
            },
            original_feedback: Some("reviewer 修订反馈".to_string()),
        },
        state: LcResumeRecoveryState::Waiting,
        operation_id: None,
        fresh_node_id: None,
        fresh_native_session_id: None,
        fresh_role_run_seq: None,
        fresh_operations: Vec::new(),
        detected_at: "2026-10-11T00:00:00Z".to_string(),
        updated_at: "2026-10-11T00:00:00Z".to_string(),
    };
    lifecycle
        .compare_and_save_lc_resume_state(&expected, waiting)
        .expect("seed waiting recovery");

    // manager 重建路径：from_record 携带 waiting recovery 重建 engine。
    let checkpoint_store = Arc::new(CheckpointStore::new(root.path().join("checkpoints")));
    let (engine_tx, _engine_rx) = mpsc::channel::<EngineEvent>(64);
    let updated = lifecycle
        .get_workspace_session(&session_record.id)
        .expect("updated record");
    let mut session = WorkspaceSession::from_record(updated);
    session.stage = WorkspaceStage::PrepareContext;
    let payload = ArtifactPayload::Markdown {
        markdown: "# Story Spec\n".to_string(),
        diff: None,
    };
    session.artifact = Some(payload.clone());
    let mut workspace_engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle.clone(), engine_tx, session);
    workspace_engine.artifact_versions = vec![ArtifactVersion {
        version: 1,
        payload,
        generated_by: ProviderName::ClaudeCode,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-10-11T17:00:00Z".to_string(),
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

    // 被等待占用的旧失败节点不投影（SessionState.recoverable_interrupted_run
    // 为 None）。
    assert!(
        workspace_engine.recoverable_interrupted_run().is_none(),
        "LC 等待占用的失败节点不得投影为可恢复"
    );
    let crate::web::workspace_ws_types::WsOutMessage::SessionState {
        recoverable_interrupted_run,
        lc_resume_recovery,
        ..
    } = workspace_engine.build_session_state()
    else {
        panic!("expected SessionState");
    };
    assert!(
        recoverable_interrupted_run.is_none(),
        "SessionState 不得投影被 LC 等待占用的恢复入口"
    );
    let recovery_dto = lc_resume_recovery.expect("waiting recovery must project");
    assert_eq!(recovery_dto.state, LcResumeRecoveryState::Waiting);
    assert!(
        recovery_dto.allowed_actions.is_empty(),
        "B 齐前 waiting allowed_actions 必须为空"
    );

    // 普通参数 generation：后端真实入口拒收（A/B 未齐无可启动路径）。
    let plain_generation = workspace_engine
        .start_generation(
            ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            true,
        )
        .await;
    let error = plain_generation.expect_err("waiting 期间普通 generation 必须被拒收");
    assert!(
        error.contains("LC_RESUME_RECOVERY_ACTIVE"),
        "拒收错误必须携带稳定码 LC_RESUME_RECOVERY_ACTIVE: {error}"
    );

    // interrupted retry：typed 拒收（不受理被等待占用的旧 provider 失败节点）。
    let interrupted_retry = workspace_engine
        .retry_interrupted_run("timeline_node_004")
        .await;
    assert!(
        matches!(
            interrupted_retry,
            Err(crate::product::workspace_engine::InterruptedRunRecoveryError::LcResumeWaiting)
        ),
        "waiting 期间 interrupted retry 必须 typed 拒收: {interrupted_retry:?}"
    );

    // 事实不变：durable record 仍是 waiting，未出生新节点/未改状态。
    let after = lifecycle
        .get_workspace_session(&session_record.id)
        .expect("record after rejections");
    let recovery = after.lc_resume_recovery.expect("recovery intact");
    assert_eq!(recovery.state, LcResumeRecoveryState::Waiting);
    assert_eq!(recovery.recovery_id, "recovery_anchor_t07");
}
