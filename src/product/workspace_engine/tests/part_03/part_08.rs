async fn prepare_outline_review_decision_without_index(
    scope: WorkItemPlanReviewScope,
) -> (TempDir, LifecycleStore, String, WorkspaceEngine) {
    let (tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate(&format!(
            "sess_outline_policy_{scope:?}"
        ));
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    engine.session.stage = WorkspaceStage::ReviewDecision;
    let source_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::ReviewDecision,
            agent: None,
            stage: WorkspaceStage::ReviewDecision,
            round: Some(1),
            title: "Review 决策".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await;
    lifecycle
        .update_workspace_session_status(
            &engine.session.session_id,
            WorkspaceSessionStatus::WaitingForHuman,
        )
        .expect("set review decision session status");
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "需要重开 Outline".to_string(),
        summary: "需要重开 Outline".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: Some(WorkItemPlanReviewComplete {
            verdict: WorkItemPlanReviewVerdict::PlanReopenRequired,
            review_scope: scope.clone(),
            target_outline_id: (scope == WorkItemPlanReviewScope::Item)
                .then(|| "outline_a".to_string()),
            generation_round_id: if scope == WorkItemPlanReviewScope::Outline {
                "legacy_work_item_plan_candidate".to_string()
            } else {
                "round_0001".to_string()
            },
            draft_id: (scope == WorkItemPlanReviewScope::Item).then(|| "draft_a".to_string()),
            batch_id: (scope == WorkItemPlanReviewScope::Batch).then(|| "batch_a".to_string()),
            review_action: WorkItemPlanReviewAction::ReviseOutline,
            gates: vec![WorkItemPlanReviewGate::RequiresPlanReopen],
            affects_items: Vec::new(),
            warnings: Vec::new(),
        }),
        structured_output_diagnostic: None,
    });
    (tmp, lifecycle, source_node_id, engine)
}

#[tokio::test]
async fn item_and_batch_review_decision_require_active_round() {
    for scope in [WorkItemPlanReviewScope::Item, WorkItemPlanReviewScope::Batch] {
        let (_tmp, lifecycle, source_node_id, mut engine) =
            prepare_outline_review_decision_without_index(scope.clone()).await;
        let original_artifact_versions = engine.artifact_versions.clone();
        let original_timeline_nodes = engine.timeline_nodes.clone();

        let error = engine
            .handle_review_decision("continue".to_string(), None)
            .await
            .expect_err("item/batch plan reopen must require an active round");

        assert!(error.contains("work item plan active index missing"));
        assert_eq!(engine.session().stage, WorkspaceStage::ReviewDecision);
        assert_eq!(engine.active_node_id.as_deref(), Some(source_node_id.as_str()));
        assert_eq!(
            serde_json::to_value(&engine.artifact_versions).expect("artifact versions json"),
            serde_json::to_value(original_artifact_versions).expect("original artifacts json")
        );
        assert_eq!(
            serde_json::to_value(&engine.timeline_nodes).expect("timeline json"),
            serde_json::to_value(original_timeline_nodes).expect("original timeline json")
        );
        assert_eq!(
            lifecycle
                .get_workspace_session(&engine.session.session_id)
                .expect("workspace session")
                .status,
            WorkspaceSessionStatus::WaitingForHuman,
            "{scope:?} failure must not leave lifecycle status Open"
        );
    }
}

#[tokio::test]
async fn generation_mode_generic_request_revision_requires_active_round() {
    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_generation_mode_policy");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    engine
        .complete_active_node(Some("进入 generation mode".to_string()))
        .await;
    engine.session.stage = WorkspaceStage::AuthorConfirm;
    let source_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemGenerationMode,
            agent: None,
            stage: WorkspaceStage::AuthorConfirm,
            round: None,
            title: "选择 Work Item 生成模式".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await;
    engine.pending_revision_context = Some("既有上下文".to_string());
    lifecycle
        .update_workspace_session_status(
            &engine.session.session_id,
            WorkspaceSessionStatus::WaitingForHuman,
        )
        .expect("set generation mode status");
    let original_artifact_versions = engine.artifact_versions.clone();
    let original_timeline_nodes = engine.timeline_nodes.clone();

    let error = engine
        .request_work_item_plan_revision(Some("重新调整 Outline".to_string()))
        .await
        .expect_err("generation mode revision must require active round");

    assert!(error.contains("work item plan active index missing"));
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
    assert_eq!(engine.active_node_id.as_deref(), Some(source_node_id.as_str()));
    assert_eq!(
        engine.pending_revision_context.as_deref(),
        Some("既有上下文")
    );
    assert_eq!(
        serde_json::to_value(&engine.artifact_versions).expect("artifact versions json"),
        serde_json::to_value(original_artifact_versions).expect("original artifacts json")
    );
    assert_eq!(
        serde_json::to_value(&engine.timeline_nodes).expect("timeline json"),
        serde_json::to_value(original_timeline_nodes).expect("original timeline json")
    );
    assert_eq!(
        lifecycle
            .get_workspace_session(&engine.session.session_id)
            .expect("workspace session")
            .status,
        WorkspaceSessionStatus::WaitingForHuman
    );
}

#[tokio::test]
async fn legacy_outline_scope_review_decision_allows_missing_initial_round() {
    let (_tmp, _lifecycle, _source_node_id, mut engine) =
        prepare_outline_review_decision_without_index(WorkItemPlanReviewScope::Outline).await;

    let outcome = engine
        .handle_review_decision("continue".to_string(), None)
        .await
        .expect("legacy outline review keeps initial-round compatibility");

    assert!(matches!(
        outcome,
        ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { .. }
    ));
    assert_eq!(engine.session().stage, WorkspaceStage::Running);
}

fn outline_revision_engine_snapshot(engine: &WorkspaceEngine) -> serde_json::Value {
    serde_json::json!({
        "stage": engine.session.stage.as_str(),
        "active_node_id": engine.active_node_id,
        "pending_revision_context": engine.pending_revision_context,
        "artifact_versions": engine.artifact_versions,
        "timeline_nodes": engine.timeline_nodes,
        "author_retry": engine.work_item_plan_author_retry_count,
        "revision_retry": engine.work_item_plan_revision_retry_count,
    })
}

fn outline_revision_persisted_snapshot(
    lifecycle: &LifecycleStore,
    engine: &WorkspaceEngine,
    plan_id: &str,
    source_node_id: &str,
) -> serde_json::Value {
    let store = engine.work_item_plan_store().expect("work item plan store");
    let mut drafts = store
        .list_draft_records("project_0001", "issue_0001", plan_id)
        .expect("draft records");
    drafts.sort_by(|left, right| left.draft_id.cmp(&right.draft_id));
    let node_detail = match lifecycle.load_node_detail(&engine.session.session_id, source_node_id) {
        Ok(detail) => Some(detail),
        Err(ProductStoreError::NotFound { .. }) => None,
        Err(error) => panic!("load node detail snapshot failed: {error}"),
    };

    serde_json::json!({
        "session_status": lifecycle
            .get_workspace_session(&engine.session.session_id)
            .expect("workspace session")
            .status,
        "artifact_versions": lifecycle
            .list_artifact_versions(&engine.session.session_id)
            .expect("artifact versions"),
        "timeline_nodes": lifecycle
            .load_timeline_nodes(&engine.session.session_id)
            .expect("timeline nodes"),
        "node_detail": node_detail,
        "active_index": store
            .load_active_index("project_0001", "issue_0001", plan_id)
            .expect("active index"),
        "drafts": drafts,
    })
}

fn workspace_timeline_root(
    lifecycle: &LifecycleStore,
    engine: &WorkspaceEngine,
) -> std::path::PathBuf {
    lifecycle
        .app_paths()
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("workspace-timelines")
        .join(&engine.session.session_id)
}

#[cfg(unix)]
#[tokio::test]
async fn outline_revision_index_save_failure_restores_non_empty_drafts_and_session() {
    use std::os::unix::fs::PermissionsExt;

    let (_tmp, lifecycle, plan_id, source_node_id, mut engine) =
        make_atomic_outline_revision_engine("sess_outline_atomic_real_drafts", false).await;
    let original_drafts = save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);
    assert!(original_drafts.len() >= 2, "fixture must contain multiple drafts");
    let engine_before = outline_revision_engine_snapshot(&engine);
    let persisted_before =
        outline_revision_persisted_snapshot(&lifecycle, &engine, &plan_id, &source_node_id);
    let plan_root = active_index_path(&lifecycle, &plan_id)
        .parent()
        .expect("plan root")
        .to_path_buf();
    let original_permissions = std::fs::metadata(&plan_root)
        .expect("plan root metadata")
        .permissions();
    std::fs::set_permissions(&plan_root, std::fs::Permissions::from_mode(0o555))
        .expect("make active index root read-only");

    let result = engine
        .prepare_work_item_plan_outline_revision(
            Some("真实 drafts 原子返修".to_string()),
            WorkItemPlanOutlineRevisionSource::ReviewDecision,
            OutlineRevisionPersistencePolicy::RequireActiveRound,
        )
        .await;

    std::fs::set_permissions(&plan_root, original_permissions)
        .expect("restore active index root permissions");
    let error = result.expect_err("active index save failure must fail the transaction");
    assert!(error.contains("save work item plan active index failed"));
    assert_eq!(outline_revision_engine_snapshot(&engine), engine_before);
    assert_eq!(
        outline_revision_persisted_snapshot(&lifecycle, &engine, &plan_id, &source_node_id),
        persisted_before,
        "drafts, active index, session, artifact and timeline must all roll back"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn outline_revision_artifact_versions_save_failure_rolls_back_everything() {
    use std::os::unix::fs::PermissionsExt;

    let (_tmp, lifecycle, plan_id, source_node_id, mut engine) =
        make_atomic_outline_revision_engine("sess_outline_atomic_artifact_save", false).await;
    let engine_before = outline_revision_engine_snapshot(&engine);
    let persisted_before =
        outline_revision_persisted_snapshot(&lifecycle, &engine, &plan_id, &source_node_id);
    let timeline_root = workspace_timeline_root(&lifecycle, &engine);
    let original_permissions = std::fs::metadata(&timeline_root)
        .expect("timeline root metadata")
        .permissions();
    std::fs::set_permissions(&timeline_root, std::fs::Permissions::from_mode(0o555))
        .expect("make timeline root read-only");

    let result = engine
        .prepare_work_item_plan_outline_revision(
            Some("artifact save failure".to_string()),
            WorkItemPlanOutlineRevisionSource::AuthorConfirm,
            OutlineRevisionPersistencePolicy::AllowMissingInitialRound,
        )
        .await;

    std::fs::set_permissions(&timeline_root, original_permissions)
        .expect("restore timeline root permissions");
    let error = result.expect_err("artifact versions save failure must fail the transaction");
    assert!(error.contains("save outline revision artifact versions failed"));
    assert_eq!(outline_revision_engine_snapshot(&engine), engine_before);
    assert_eq!(
        outline_revision_persisted_snapshot(&lifecycle, &engine, &plan_id, &source_node_id),
        persisted_before
    );
}

#[tokio::test]
async fn outline_revision_timeline_save_failure_rolls_back_everything() {
    let (_tmp, lifecycle, plan_id, source_node_id, mut engine) =
        make_atomic_outline_revision_engine("sess_outline_atomic_timeline_save", false).await;
    let engine_before = outline_revision_engine_snapshot(&engine);
    let persisted_before =
        outline_revision_persisted_snapshot(&lifecycle, &engine, &plan_id, &source_node_id);
    let timeline_path = workspace_timeline_root(&lifecycle, &engine).join("timeline_nodes.json");
    let backup_path = timeline_path.with_extension("json.backup");
    std::fs::rename(&timeline_path, &backup_path).expect("backup timeline file");
    std::fs::create_dir(&timeline_path).expect("block timeline target with directory");

    let result = engine
        .prepare_work_item_plan_outline_revision(
            Some("timeline save failure".to_string()),
            WorkItemPlanOutlineRevisionSource::AuthorConfirm,
            OutlineRevisionPersistencePolicy::AllowMissingInitialRound,
        )
        .await;

    std::fs::remove_dir(&timeline_path).expect("remove timeline blocker");
    std::fs::rename(&backup_path, &timeline_path).expect("restore timeline file");
    let error = result.expect_err("timeline save failure must fail the transaction");
    assert!(error.contains("save outline revision timeline failed"));
    assert_eq!(outline_revision_engine_snapshot(&engine), engine_before);
    assert_eq!(
        outline_revision_persisted_snapshot(&lifecycle, &engine, &plan_id, &source_node_id),
        persisted_before
    );
}

#[cfg(unix)]
#[tokio::test]
async fn outline_revision_node_detail_save_failure_rolls_back_without_success_events() {
    use std::os::unix::fs::PermissionsExt;

    let (_tmp, lifecycle, plan_id, source_node_id, mut engine) =
        make_atomic_outline_revision_engine("sess_outline_atomic_node_detail_save", false).await;
    let engine_before = outline_revision_engine_snapshot(&engine);
    let persisted_before =
        outline_revision_persisted_snapshot(&lifecycle, &engine, &plan_id, &source_node_id);
    let details_root = workspace_timeline_root(&lifecycle, &engine).join("timeline_node_details");
    std::fs::create_dir_all(&details_root).expect("create node details root");
    let original_permissions = std::fs::metadata(&details_root)
        .expect("node details metadata")
        .permissions();
    std::fs::set_permissions(&details_root, std::fs::Permissions::from_mode(0o555))
        .expect("make node details read-only");
    let (event_tx, mut event_rx) = mpsc::channel(16);
    engine.event_tx = event_tx;

    let result = engine
        .prepare_work_item_plan_outline_revision(
            Some("node detail save failure".to_string()),
            WorkItemPlanOutlineRevisionSource::HumanConfirm,
            OutlineRevisionPersistencePolicy::AllowMissingInitialRound,
        )
        .await;

    std::fs::set_permissions(&details_root, original_permissions)
        .expect("restore node details permissions");
    let error = result.expect_err("node detail save failure must fail the transaction");
    assert!(error.contains("save outline revision node detail failed"));
    assert_eq!(outline_revision_engine_snapshot(&engine), engine_before);
    assert_eq!(
        outline_revision_persisted_snapshot(&lifecycle, &engine, &plan_id, &source_node_id),
        persisted_before
    );
    while let Ok(event) = event_rx.try_recv() {
        assert!(
            !matches!(
                event,
                EngineEvent::StageChange { ref stage } if stage == "running"
            ),
            "failed transaction must not emit running StageChange"
        );
        assert!(
            !matches!(event, EngineEvent::TimelineNodeUpdated { .. }),
            "failed transaction must not emit TimelineNodeUpdated"
        );
    }
}
