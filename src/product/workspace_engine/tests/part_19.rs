#[test]
fn interrupted_work_item_draft_review_remains_recoverable_after_failed_outline_run() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_interrupted_draft_review");
    let outline_id = "outline_a";
    let draft_id = "draft_012";
    let draft_record = test_work_item_draft_record(
        &plan_id,
        outline_id,
        draft_id,
        WorkItemDraftStatus::Accepted,
        WorkItemGenerationMode::Serial,
        None,
    );
    let store = engine.work_item_plan_store().expect("work item plan store");
    store
        .put_draft_record(&draft_record)
        .expect("persist accepted draft");
    store
        .save_active_index(&WorkItemPlanDraftActiveIndex {
            project_id: engine.session.project_id.clone(),
            issue_id: engine.session.issue_id.clone(),
            plan_id: plan_id.clone(),
            current_generation_round_id: "round_0001".to_string(),
            outline_state: "confirmed".to_string(),
            active_outline_id: Some(outline_id.to_string()),
            outline_to_current_draft_id: BTreeMap::from([(
                outline_id.to_string(),
                draft_id.to_string(),
            )]),
            draft_statuses: BTreeMap::from([(
                draft_id.to_string(),
                WorkItemDraftStatus::Accepted,
            )]),
            batches: vec![],
            updated_at: "2026-07-11T17:40:10Z".to_string(),
        })
        .expect("persist active index");

    let draft_payload = work_item_draft_artifact_payload(
        &plan_id,
        outline_id,
        draft_id,
        WorkItemDraftStatus::Accepted,
    );
    engine.session.stage = WorkspaceStage::PrepareContext;
    engine.session.artifact = Some(draft_payload.clone());
    engine.artifact_versions = vec![ArtifactVersion {
        version: 14,
        payload: draft_payload,
        generated_by: ProviderName::Codex,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-07-11T17:36:02Z".to_string(),
        source_node_id: "timeline_node_052".to_string(),
    }];
    engine.timeline_nodes = vec![
        interrupted_recovery_timeline_node(
            "timeline_node_052",
            TimelineNodeType::WorkItemDraftRun,
            TimelineNodeStatus::Completed,
            WsWorkspaceStage::Running,
            Some(format!("{outline_id} · {draft_id} · draft")),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_053",
            TimelineNodeType::WorkItemDraftConfirm,
            TimelineNodeStatus::Completed,
            WsWorkspaceStage::AuthorConfirm,
            Some("Work Item Draft 已接受".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_054",
            TimelineNodeType::WorkItemDraftReview,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::CrossReview,
            Some("连接断开，运行已中止".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_055",
            TimelineNodeType::AbortedByDisconnect,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::PrepareContext,
            Some("last_active_run_id: run-18".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_056",
            TimelineNodeType::StartGeneration,
            TimelineNodeStatus::Completed,
            WsWorkspaceStage::PrepareContext,
            None,
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_057",
            TimelineNodeType::WorkItemPlanOutlineRun,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::Running,
            Some("连接断开，运行已中止".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_058",
            TimelineNodeType::AbortedByDisconnect,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::PrepareContext,
            Some("last_active_run_id: run-1".to_string()),
        ),
    ];

    let recovery = engine
        .recoverable_interrupted_run()
        .expect("draft review should remain recoverable");

    assert_eq!(recovery.failed_node_id, "timeline_node_054");
    assert_eq!(
        recovery.operation,
        RecoverableInterruptedOperation::Review
    );
    assert_eq!(recovery.label, "重试中断审核");
}

#[test]
fn interrupted_work_item_draft_run_retries_same_active_outline() {
    let (_tmp, engine) = interrupted_work_item_draft_generation_engine();

    let recovery = engine
        .recoverable_interrupted_run()
        .expect("draft generation should be recoverable");

    assert_eq!(recovery.failed_node_id, "timeline_node_020");
    assert_eq!(
        recovery.operation,
        RecoverableInterruptedOperation::WorkItemDraftGeneration
    );
    assert_eq!(recovery.label, "重新生成中断的 Work Item Draft");
}

#[tokio::test]
async fn retry_interrupted_work_item_draft_run_creates_linked_retry_node() {
    let (_tmp, mut engine) = interrupted_work_item_draft_generation_engine();

    let outcome = engine
        .retry_interrupted_run("timeline_node_020")
        .await
        .expect("retry interrupted draft generation");

    assert_eq!(
        outcome,
        InterruptedRunRecoveryOutcome::WorkItemDraftGeneration
    );
    assert_eq!(engine.session.stage, WorkspaceStage::Running);
    let retry_node = engine.timeline_nodes.last().expect("retry draft node");
    assert_eq!(retry_node.node_type, TimelineNodeType::WorkItemDraftRun);
    assert_eq!(retry_node.status, TimelineNodeStatus::Active);
    let retry = retry_node.retry.as_ref().expect("retry metadata");
    assert_eq!(retry.retry_of_node_id, "timeline_node_020");
    assert_eq!(retry.retry_attempt, 1);
}

#[test]
fn session_state_exposes_recoverable_interrupted_run() {
    let (_tmp, engine) = interrupted_work_item_draft_generation_engine();

    let WsOutMessage::SessionState {
        recoverable_interrupted_run,
        ..
    } = engine.build_session_state()
    else {
        panic!("expected session state");
    };

    let recovery = recoverable_interrupted_run.expect("recoverable interrupted run");
    assert_eq!(recovery.failed_node_id, "timeline_node_020");
    assert_eq!(
        recovery.operation,
        RecoverableInterruptedOperation::WorkItemDraftGeneration
    );
}

#[test]
fn interrupted_shared_reviewer_run_is_recoverable_for_story_design_and_work_item() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, checkpoint_store) = setup();
        let (tx, _rx) = mpsc::channel(8);
        let mut session = make_session("sess_shared_review_recovery");
        session.workspace_type = workspace_type.clone();
        session.stage = WorkspaceStage::PrepareContext;
        let payload = artifact_payload("# Current artifact");
        session.artifact = Some(payload.clone());
        let mut engine = WorkspaceEngine::new(checkpoint_store, tx, session);
        engine.artifact_versions = vec![ArtifactVersion {
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
        engine.timeline_nodes = vec![
            interrupted_recovery_timeline_node(
                "timeline_node_002",
                TimelineNodeType::AuthorRun,
                TimelineNodeStatus::Completed,
                WsWorkspaceStage::Running,
                Some("artifact generated".to_string()),
            ),
            interrupted_recovery_timeline_node(
                "timeline_node_004",
                TimelineNodeType::ReviewerRun,
                TimelineNodeStatus::Failed,
                WsWorkspaceStage::CrossReview,
                Some("连接断开，运行已中止".to_string()),
            ),
            interrupted_recovery_timeline_node(
                "timeline_node_005",
                TimelineNodeType::AbortedByDisconnect,
                TimelineNodeStatus::Failed,
                WsWorkspaceStage::PrepareContext,
                Some("last_active_run_id: run-2".to_string()),
            ),
        ];

        let recovery = engine
            .recoverable_interrupted_run()
            .unwrap_or_else(|| panic!("{workspace_type:?} review should be recoverable"));
        assert_eq!(recovery.failed_node_id, "timeline_node_004");
        assert_eq!(recovery.operation, RecoverableInterruptedOperation::Review);
    }
}

#[test]
fn successful_new_artifact_supersedes_old_interrupted_review() {
    let (_tmp, checkpoint_store) = setup();
    let (tx, _rx) = mpsc::channel(8);
    let mut session = make_session("sess_superseded_review_recovery");
    session.stage = WorkspaceStage::PrepareContext;
    let latest_payload = artifact_payload("# New artifact");
    session.artifact = Some(latest_payload.clone());
    let mut engine = WorkspaceEngine::new(checkpoint_store, tx, session);
    engine.artifact_versions = vec![ArtifactVersion {
        version: 2,
        payload: latest_payload,
        generated_by: ProviderName::ClaudeCode,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-07-11T17:10:00Z".to_string(),
        source_node_id: "timeline_node_006".to_string(),
    }];
    engine.timeline_nodes = vec![
        interrupted_recovery_timeline_node(
            "timeline_node_002",
            TimelineNodeType::AuthorRun,
            TimelineNodeStatus::Completed,
            WsWorkspaceStage::Running,
            Some("old artifact".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_004",
            TimelineNodeType::ReviewerRun,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::CrossReview,
            Some("连接断开，运行已中止".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_005",
            TimelineNodeType::AbortedByDisconnect,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::PrepareContext,
            Some("last_active_run_id: run-2".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_006",
            TimelineNodeType::AuthorRun,
            TimelineNodeStatus::Completed,
            WsWorkspaceStage::Running,
            Some("new artifact".to_string()),
        ),
    ];

    assert!(engine.recoverable_interrupted_run().is_none());
}

#[tokio::test]
async fn retry_interrupted_review_creates_linked_retry_node_once() {
    let (_tmp, checkpoint_store) = setup();
    let (tx, _rx) = mpsc::channel(8);
    let mut session = make_session("sess_retry_interrupted_review");
    session.stage = WorkspaceStage::PrepareContext;
    let payload = artifact_payload("# Current artifact");
    session.artifact = Some(payload.clone());
    let mut engine = WorkspaceEngine::new(checkpoint_store, tx, session);
    engine.artifact_versions = vec![ArtifactVersion {
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
    engine.timeline_nodes = vec![
        interrupted_recovery_timeline_node(
            "timeline_node_002",
            TimelineNodeType::AuthorRun,
            TimelineNodeStatus::Completed,
            WsWorkspaceStage::Running,
            Some("artifact generated".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_004",
            TimelineNodeType::ReviewerRun,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::CrossReview,
            Some("连接断开，运行已中止".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_005",
            TimelineNodeType::AbortedByDisconnect,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::PrepareContext,
            Some("last_active_run_id: run-2".to_string()),
        ),
    ];

    let stale = engine
        .retry_interrupted_run("timeline_node_999")
        .await
        .expect_err("stale failed node id must be rejected");
    assert_eq!(stale.code(), "INTERRUPTED_RUN_STATE_CHANGED");

    let outcome = engine
        .retry_interrupted_run("timeline_node_004")
        .await
        .expect("retry interrupted review");

    assert_eq!(outcome, InterruptedRunRecoveryOutcome::Review);
    assert_eq!(engine.session.stage, WorkspaceStage::CrossReview);
    let retry_node = engine.timeline_nodes.last().expect("retry node");
    assert_eq!(retry_node.node_type, TimelineNodeType::ReviewerRun);
    assert_eq!(retry_node.status, TimelineNodeStatus::Active);
    let retry = retry_node.retry.as_ref().expect("retry metadata");
    assert_eq!(retry.retry_of_node_id, "timeline_node_004");
    assert_eq!(retry.retry_attempt, 1);
    assert_eq!(retry.retry_reason, "aborted_by_disconnect");

    let duplicate = engine
        .retry_interrupted_run("timeline_node_004")
        .await
        .expect_err("active retry must reject duplicate click");
    assert_eq!(duplicate.code(), "INTERRUPTED_RUN_ALREADY_ACTIVE");
}

fn interrupted_work_item_draft_generation_engine() -> (TempDir, WorkspaceEngine) {
    let (tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_interrupted_draft_generation");
    let outline_id = "outline_b";
    let store = engine.work_item_plan_store().expect("work item plan store");
    store
        .save_active_index(&WorkItemPlanDraftActiveIndex {
            project_id: engine.session.project_id.clone(),
            issue_id: engine.session.issue_id.clone(),
            plan_id,
            current_generation_round_id: "round_0001".to_string(),
            outline_state: "confirmed".to_string(),
            active_outline_id: Some(outline_id.to_string()),
            outline_to_current_draft_id: BTreeMap::new(),
            draft_statuses: BTreeMap::new(),
            batches: vec![],
            updated_at: "2026-07-11T17:20:00Z".to_string(),
        })
        .expect("persist active index");
    engine.session.stage = WorkspaceStage::PrepareContext;
    engine.session.artifact = Some(work_item_plan_outline_artifact());
    engine.artifact_versions = vec![ArtifactVersion {
        version: 1,
        payload: work_item_plan_outline_artifact(),
        generated_by: ProviderName::Codex,
        reviewed_by: Some(ProviderName::ClaudeCode),
        review_verdict: Some(ReviewVerdictType::Pass),
        confirmed_by: None,
        is_current: true,
        created_at: "2026-07-11T17:00:00Z".to_string(),
        source_node_id: "timeline_node_010".to_string(),
    }];
    engine.timeline_nodes = vec![
        interrupted_recovery_timeline_node(
            "timeline_node_010",
            TimelineNodeType::WorkItemPlanOutlineRun,
            TimelineNodeStatus::Completed,
            WsWorkspaceStage::Running,
            Some("confirmed outline".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_020",
            TimelineNodeType::WorkItemDraftRun,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::Running,
            Some("连接断开，运行已中止".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_021",
            TimelineNodeType::AbortedByDisconnect,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::PrepareContext,
            Some("last_active_run_id: run-7".to_string()),
        ),
    ];
    (tmp, engine)
}

fn interrupted_recovery_timeline_node(
    node_id: &str,
    node_type: TimelineNodeType,
    status: TimelineNodeStatus,
    stage: WsWorkspaceStage,
    summary: Option<String>,
) -> TimelineNode {
    TimelineNode {
        node_id: node_id.to_string(),
        node_type,
        agent: Some(ProviderName::Codex),
        stage,
        round: None,
        status,
        title: node_id.to_string(),
        summary,
        started_at: "2026-07-11T17:36:02Z".to_string(),
        completed_at: Some("2026-07-11T17:40:10Z".to_string()),
        duration_ms: Some(1),
        artifact_ref: Some("artifact_current".to_string()),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        retry: None,
    }
}
