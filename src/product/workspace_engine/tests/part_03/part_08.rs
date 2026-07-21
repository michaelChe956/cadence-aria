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

fn make_work_item_plan_engine_with_accepted_contract_drafts()
-> (TempDir, LifecycleStore, String, WorkspaceEngine) {
    let (tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_initial_plan_revision_compile");
    let mut outline = test_work_item_plan_outline(vec![WorkItemOutlineDependencyEdge {
        from_outline_id: "outline_a".to_string(),
        to_outline_id: "outline_b".to_string(),
    }]);
    outline.work_item_outlines.truncate(2);
    outline.work_item_outlines[1].depends_on = vec!["outline_a".to_string()];
    engine.session.artifact = Some(ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline,
            design_context_gaps: vec![],
            validator_findings: vec![],
            context_blockers: vec![],
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: Some(WorkItemGenerationModeDto::Serial),
        }),
    });

    let store = engine.work_item_plan_store().expect("work item plan store");
    let draft_a = test_work_item_draft_record(
        &plan_id,
        "outline_a",
        "draft_outline_a",
        WorkItemDraftStatus::Accepted,
        WorkItemGenerationMode::Serial,
        None,
    );
    let mut draft_b = test_work_item_draft_record(
        &plan_id,
        "outline_b",
        "draft_outline_b",
        WorkItemDraftStatus::Accepted,
        WorkItemGenerationMode::Serial,
        None,
    );
    let mut required = crate::product::work_item_contract::canonical_contract_fixture("unused")
        .input_contracts
        .remove(0);
    required.provider_logical_work_item_id = "wi_a".to_string();
    required.contract_id = "contract.canonical".to_string();
    required.required_capabilities = vec!["stable_hash".to_string()];
    draft_b
        .candidate
        .canonical_contract_candidate
        .input_contracts
        .push(required);
    draft_b
        .candidate
        .canonical_contract_candidate
        .handoff_contract
        .provided_contract_refs
        .clear();

    for draft in [&draft_a, &draft_b] {
        store.put_draft_record(draft).expect("put accepted draft");
    }
    store
        .save_active_index(&WorkItemPlanDraftActiveIndex {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: plan_id.clone(),
            current_generation_round_id: "round_0001".to_string(),
            outline_state: "confirmed".to_string(),
            active_outline_id: None,
            outline_to_current_draft_id: BTreeMap::from([
                ("outline_a".to_string(), "draft_outline_a".to_string()),
                ("outline_b".to_string(), "draft_outline_b".to_string()),
            ]),
            draft_statuses: BTreeMap::from([
                (
                    "draft_outline_a".to_string(),
                    WorkItemDraftStatus::Accepted,
                ),
                (
                    "draft_outline_b".to_string(),
                    WorkItemDraftStatus::Accepted,
                ),
            ]),
            batches: vec![],
            updated_at: chrono::Utc::now().to_rfc3339(),
        })
        .expect("save accepted draft index");

    (tmp, lifecycle, plan_id, engine)
}

#[tokio::test]
async fn work_item_plan_initial_compile_publishes_revision_and_projection_bundles() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let lifecycle_work_item_count = lifecycle
        .count_work_items("project_0001", "issue_0001")
        .unwrap();
    let outcome = engine.run_work_item_plan_compile().await.unwrap();
    let revision_store = engine.revision_store();

    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    let revision_id = plan.active_revision_id.as_deref().unwrap();
    let revision = revision_store
        .get_plan_revision(
            "project_0001",
            "issue_0001",
            &plan.id,
            revision_id,
        )
        .unwrap();

    assert_eq!(revision.revision_no, 1);
    assert_eq!(revision.work_item_bindings.len(), 2);
    assert!(outcome.projection_validation.is_valid());
    assert_eq!(
        outcome
            .plan_projection_bundle
            .coder_group_context
            .ordered_logical_work_item_ids,
        vec!["wi_a".to_string(), "wi_b".to_string()]
    );
    assert_eq!(
        revision_store
            .get_dependency_graph_revision(&plan, &revision.dependency_graph_revision_id)
            .unwrap(),
        outcome.dependency_graph_revision
    );
    assert_eq!(
        revision_store
            .get_plan_validation_report(&plan, &revision.validation_report_ref)
            .unwrap(),
        outcome.validation_report
    );
    assert_eq!(
        revision_store
            .get_plan_projection_bundle(&plan, &revision.plan_projection_bundle_id)
            .unwrap(),
        outcome.plan_projection_bundle
    );
    for (logical_work_item_id, work_item_revision_id) in &revision.work_item_bindings {
        let work_item_revision = revision_store
            .get_work_item_revision(
                &plan,
                logical_work_item_id,
                work_item_revision_id,
            )
            .unwrap();
        revision_store
            .get_verification_plan_revision(
                &plan,
                &work_item_revision.verification_plan_revision_id,
            )
            .unwrap();
        revision_store
            .get_work_item_projection_bundle(
                &plan,
                &work_item_revision.work_item_projection_bundle_id,
            )
            .unwrap();
    }
    assert_eq!(
        lifecycle
            .count_work_items("project_0001", "issue_0001")
            .unwrap(),
        lifecycle_work_item_count,
        "initial revision compile must not create legacy LifecycleWorkItemRecord facts"
    );
}

#[tokio::test]
async fn work_item_plan_compile_canonical_validation_failure_writes_no_revision_artifacts() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let lifecycle_work_item_count = lifecycle
        .count_work_items("project_0001", "issue_0001")
        .unwrap();
    let store = engine.work_item_plan_store().unwrap();
    let mut draft_b = store
        .get_draft_record(
            "project_0001",
            "issue_0001",
            &plan_id,
            "round_0001",
            "draft_outline_b",
        )
        .unwrap();
    draft_b
        .candidate
        .canonical_contract_candidate
        .input_contracts[0]
        .required_capabilities = vec!["missing_capability".to_string()];
    store.put_draft_record(&draft_b).unwrap();

    let error = engine.run_work_item_plan_compile().await.unwrap_err();

    assert!(error.contains("canonical work item plan validation failed"));
    assert!(matches!(
        engine.revision_store().get_plan_lineage(
            "project_0001",
            "issue_0001",
            &plan_id,
        ),
        Err(ProductStoreError::NotFound { .. })
    ));
    assert_eq!(
        lifecycle
            .count_work_items("project_0001", "issue_0001")
            .unwrap(),
        lifecycle_work_item_count
    );
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

fn assert_no_outline_revision_success_events(event_rx: &mut mpsc::Receiver<EngineEvent>) {
    while let Ok(event) = event_rx.try_recv() {
        assert!(
            !matches!(event, EngineEvent::TimelineNodeUpdated { .. }),
            "failed transaction must not emit TimelineNodeUpdated"
        );
        assert!(
            !matches!(event, EngineEvent::TimelineNodeCreated { .. }),
            "failed transaction must not emit TimelineNodeCreated"
        );
        assert!(
            !matches!(
                event,
                EngineEvent::StageChange { ref stage } if stage == "running"
            ),
            "failed transaction must not emit running StageChange"
        );
    }
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
    let run_node_id = format!("timeline_node_{:03}", engine.timeline_nodes.len() + 1);
    let plan_root = active_index_path(&lifecycle, &plan_id)
        .parent()
        .expect("plan root")
        .to_path_buf();
    let original_permissions = std::fs::metadata(&plan_root)
        .expect("plan root metadata")
        .permissions();
    std::fs::set_permissions(&plan_root, std::fs::Permissions::from_mode(0o555))
        .expect("make active index root read-only");
    let (event_tx, mut event_rx) = mpsc::channel(16);
    engine.event_tx = event_tx;

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
    assert!(matches!(
        lifecycle.load_node_detail(&engine.session.session_id, &run_node_id),
        Err(ProductStoreError::NotFound { .. })
    ));
    assert_no_outline_revision_success_events(&mut event_rx);
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

#[tokio::test]
async fn outline_revision_node_detail_save_failure_rolls_back_without_success_events() {
    let (_tmp, lifecycle, plan_id, source_node_id, mut engine) =
        make_atomic_outline_revision_engine("sess_outline_atomic_node_detail_save", false).await;
    let engine_before = outline_revision_engine_snapshot(&engine);
    let persisted_before =
        outline_revision_persisted_snapshot(&lifecycle, &engine, &plan_id, &source_node_id);
    let run_node_id = format!("timeline_node_{:03}", engine.timeline_nodes.len() + 1);
    let details_root = workspace_timeline_root(&lifecycle, &engine).join("timeline_node_details");
    std::fs::create_dir_all(&details_root).expect("create node details root");
    let run_detail_blocker = details_root.join(format!("{run_node_id}.json"));
    std::fs::create_dir(&run_detail_blocker).expect("block only the new run detail target");
    let (event_tx, mut event_rx) = mpsc::channel(16);
    engine.event_tx = event_tx;

    let result = engine
        .prepare_work_item_plan_outline_revision(
            Some("node detail save failure".to_string()),
            WorkItemPlanOutlineRevisionSource::HumanConfirm,
            OutlineRevisionPersistencePolicy::AllowMissingInitialRound,
        )
        .await;

    std::fs::remove_dir(&run_detail_blocker).expect("remove run detail blocker");
    let error = result.expect_err("node detail save failure must fail the transaction");
    assert!(error.contains("save outline revision run node detail failed"));
    assert_eq!(outline_revision_engine_snapshot(&engine), engine_before);
    assert_eq!(
        outline_revision_persisted_snapshot(&lifecycle, &engine, &plan_id, &source_node_id),
        persisted_before
    );
    assert!(matches!(
        lifecycle.load_node_detail(&engine.session.session_id, &run_node_id),
        Err(ProductStoreError::NotFound { .. })
    ));
    assert_no_outline_revision_success_events(&mut event_rx);
}

#[tokio::test]
async fn outline_revision_prepared_journal_recovers_each_persistence_crash_point() {
    for crash_point in [
        OutlineRevisionCrashPoint::Status,
        OutlineRevisionCrashPoint::ArtifactVersions,
        OutlineRevisionCrashPoint::Timeline,
        OutlineRevisionCrashPoint::SourceNodeDetail,
        OutlineRevisionCrashPoint::RunNodeDetail,
        OutlineRevisionCrashPoint::PlanDrafts,
        OutlineRevisionCrashPoint::ActiveIndex,
    ] {
        let (_tmp, lifecycle, plan_id, source_node_id, mut engine) =
            make_atomic_outline_revision_engine(
                &format!("sess_outline_crash_{crash_point:?}"),
                true,
            )
            .await;
        save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);
        let persisted_before =
            outline_revision_persisted_snapshot(&lifecycle, &engine, &plan_id, &source_node_id);
        engine.outline_revision_crash_after = Some(crash_point);

        let error = engine
            .prepare_work_item_plan_outline_revision(
                Some("crash-safe feedback".to_string()),
                WorkItemPlanOutlineRevisionSource::AuthorConfirm,
                OutlineRevisionPersistencePolicy::RequireActiveRound,
            )
            .await
            .expect_err("test crash must interrupt before in-memory commit");
        assert!(error.contains("simulated outline revision crash"));

        let session_record = lifecycle
            .get_workspace_session(&engine.session.session_id)
            .expect("persisted session after simulated crash");
        let checkpoint_store = Arc::new(CheckpointStore::new(
            lifecycle
                .app_paths()
                .issue_lifecycle_root("project_0001", "issue_0001"),
        ));
        let (event_tx, _event_rx) = mpsc::channel(8);
        let recovered = WorkspaceEngine::new_persistent(
            checkpoint_store,
            lifecycle.clone(),
            event_tx,
            WorkspaceSession::from_record(session_record),
        );

        assert!(
            recovered.outline_revision_recovery_error().is_none(),
            "{crash_point:?}"
        );
        assert_eq!(
            outline_revision_persisted_snapshot(
                &lifecycle,
                &recovered,
                &plan_id,
                &source_node_id,
            ),
            persisted_before,
            "{crash_point:?} must roll back from the persisted journal"
        );
    }
}

#[tokio::test]
async fn outline_revision_active_index_crash_happens_after_revising_index_is_persisted() {
    let (_tmp, _lifecycle, plan_id, _source_node_id, mut engine) =
        make_atomic_outline_revision_engine("sess_outline_crash_after_active_index", true).await;
    save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);
    engine.outline_revision_crash_after = Some(OutlineRevisionCrashPoint::ActiveIndex);

    engine
        .prepare_work_item_plan_outline_revision(
            Some("crash after active index".to_string()),
            WorkItemPlanOutlineRevisionSource::AuthorConfirm,
            OutlineRevisionPersistencePolicy::RequireActiveRound,
        )
        .await
        .expect_err("active-index crash hook must interrupt the transaction");

    let index = engine
        .work_item_plan_store()
        .expect("work item plan store")
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load persisted active index")
        .expect("persisted active index");
    assert_eq!(
        index.outline_state, "revising",
        "active-index crash hook must run after the revised index reaches disk"
    );
}

#[tokio::test]
async fn outline_revision_committed_journal_keeps_revision_state_on_restart() {
    let (_tmp, lifecycle, plan_id, _source_node_id, mut engine) =
        make_atomic_outline_revision_engine("sess_outline_committed_crash", true).await;
    engine.outline_revision_crash_after = Some(OutlineRevisionCrashPoint::Committed);

    let error = engine
        .prepare_work_item_plan_outline_revision(
            Some("committed feedback".to_string()),
            WorkItemPlanOutlineRevisionSource::AuthorConfirm,
            OutlineRevisionPersistencePolicy::RequireActiveRound,
        )
        .await
        .expect_err("committed crash hook must stop before memory commit");
    assert!(error.contains("simulated outline revision crash"));

    let session_record = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .expect("committed persisted session");
    let checkpoint_store = Arc::new(CheckpointStore::new(
        lifecycle
            .app_paths()
            .issue_lifecycle_root("project_0001", "issue_0001"),
    ));
    let (event_tx, _event_rx) = mpsc::channel(8);
    let recovered = WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(session_record),
    );

    assert!(recovered.outline_revision_recovery_error().is_none());
    assert_eq!(recovered.current_stage(), WorkspaceStage::Running);
    assert_eq!(
        recovered.active_node_type(),
        Some(TimelineNodeType::WorkItemPlanOutlineRun)
    );
    let run_node_id = recovered
        .active_timeline_node_id()
        .expect("committed active outline revision run");
    let detail = lifecycle
        .load_node_detail(&recovered.session.session_id, &run_node_id)
        .expect("committed run detail");
    assert!(detail.is_revision);
    assert!(detail
        .revision_feedback
        .as_deref()
        .expect("committed feedback")
        .contains("committed feedback"));
    assert_eq!(
        recovered
            .work_item_plan_store()
            .expect("work item plan store")
            .load_active_index("project_0001", "issue_0001", &plan_id)
            .expect("active index")
            .expect("active index record")
            .outline_state,
        "revising"
    );
}
