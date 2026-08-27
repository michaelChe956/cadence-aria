use std::collections::BTreeSet;

use crate::product::models::WorkItemDraftRevision;

#[tokio::test]
async fn work_item_plan_initial_compile_uses_tx_scoped_publication_journal() {
    let (_tmp, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();

    let outcome = engine.run_work_item_plan_compile().await.unwrap();
    let compile_tx = engine
        .work_item_plan_store()
        .unwrap()
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .unwrap()
        .into_iter()
        .max_by(|left, right| left.created_at.cmp(&right.created_at))
        .expect("initial compile transaction");
    let logical_ids = outcome
        .work_items
        .iter()
        .map(|item| item.work_item_revision.logical_work_item_id.clone())
        .collect::<Vec<_>>();
    let revision_store = engine.revision_store();
    let allocated = revision_store
        .allocate_initial_plan_publication_ids(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_tx.compile_id,
            &logical_ids,
        )
        .unwrap();
    let journal = revision_store
        .get_initial_plan_publication_journal(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_tx.compile_id,
        )
        .unwrap();

    assert_eq!(
        journal.phase,
        crate::product::work_item_revision_store::InitialPlanPublicationPhase::PlanActivated
    );
    assert_eq!(journal.outline_version_ref, compile_tx.outline_version_ref);
    assert_eq!(journal.allocated_ids, allocated);
    assert_eq!(journal.created_at, compile_tx.created_at);
    assert_eq!(journal.updated_at, compile_tx.created_at);
    assert_eq!(journal.artifacts.lineage.created_at, compile_tx.created_at);
    assert_eq!(journal.artifacts.lineage.updated_at, compile_tx.created_at);
    assert_eq!(
        journal.artifacts.plan_revision.created_at,
        compile_tx.created_at
    );
    assert_eq!(
        journal.artifacts.dependency_graph_revision.created_at,
        compile_tx.created_at
    );
    assert_eq!(
        journal.artifacts.validation_report.created_at,
        compile_tx.created_at
    );
    assert_eq!(
        journal.artifacts.plan_projection_bundle.created_at,
        compile_tx.created_at
    );
    assert_eq!(outcome.plan_revision.id, allocated.plan_revision_id);
    for (item, publication_item) in outcome.work_items.iter().zip(&journal.artifacts.work_items) {
        let ids = allocated
            .work_items
            .get(&item.work_item_revision.logical_work_item_id)
            .unwrap();
        assert_eq!(item.work_item_revision.id, ids.work_item_revision_id);
        assert_eq!(
            item.verification_plan_revision.id,
            ids.verification_plan_revision_id
        );
        assert_eq!(
            item.projection_bundle.id,
            ids.work_item_projection_bundle_id
        );
        assert_eq!(
            publication_item.logical_work_item.created_at,
            compile_tx.created_at
        );
        assert_eq!(
            publication_item.logical_work_item.updated_at,
            compile_tx.created_at
        );
        assert_eq!(item.work_item_revision.created_at, compile_tx.created_at);
        assert_eq!(
            item.verification_plan_revision.created_at,
            compile_tx.created_at
        );
        assert_eq!(item.projection_bundle.created_at, compile_tx.created_at);
    }
    let replayed = revision_store
        .publish_or_resume_initial_plan_revision(&journal)
        .unwrap();
    assert_eq!(
        replayed, journal,
        "replay must preserve journal fingerprint"
    );
}

#[tokio::test]
async fn active_initial_plan_outcome_reloads_from_tx_scoped_revision_store_facts() {
    let (_tmp, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let published = engine.run_work_item_plan_compile().await.unwrap();
    let compile_tx = engine
        .work_item_plan_store()
        .unwrap()
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .unwrap()
        .into_iter()
        .max_by(|left, right| left.created_at.cmp(&right.created_at))
        .expect("initial compile transaction");

    let reloaded = engine
        .load_initial_plan_compile_outcome(&compile_tx)
        .unwrap()
        .expect("active initial plan outcome");

    assert_eq!(reloaded, published);
}

#[tokio::test]
async fn active_initial_plan_outcome_reload_finishes_journal_marker_after_restart() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let plan_store = engine.work_item_plan_store().unwrap();
    let index = plan_store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .unwrap()
        .unwrap();
    let outline = engine.latest_work_item_plan_outline_candidate().unwrap();
    let outline_order = work_item_plan_outline_topological_order(&outline.outline).unwrap();
    let draft_records = engine
        .accepted_active_draft_records_for_compile(&plan_store, &index, &outline_order)
        .unwrap();
    let accepted_drafts = draft_records
        .iter()
        .map(work_item_draft_revision_from_record)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let previous_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .unwrap();
    let compile_tx = WorkItemPlanCompileTransaction {
        compile_id: "compile_restart_after_plan_active".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: plan_id.clone(),
        flow_kind: None,
        source_revision_id: None,
        source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        publication_provenance_ref: None,
        publication_provenance_content_hash: None,
        generation_round_id: index.current_generation_round_id,
        outline_version_ref: outline.outline.id,
        active_draft_ids: draft_records
            .iter()
            .map(|record| record.draft_id.clone())
            .collect(),
        status: WorkItemPlanCompileStatus::Committing,
        plan_commit_state: WorkItemPlanCommitState::NotStarted,
        step_cursor: "committing".to_string(),
        outline_to_work_item_id: BTreeMap::new(),
        outline_to_verification_plan_id: BTreeMap::new(),
        created_work_item_ids: Vec::new(),
        created_verification_plan_ids: Vec::new(),
        child_session_ids: Vec::new(),
        validator_findings: Vec::new(),
        abort_requested_at: None,
        failure_reason: None,
        previous_plan_snapshot: previous_plan,
        created_at: "2026-07-17T00:00:40Z".to_string(),
        updated_at: "2026-07-17T00:00:40Z".to_string(),
        committed_at: None,
    };
    plan_store.put_compile_transaction(&compile_tx).unwrap();
    let revision_store = engine.revision_store();
    let _failpoint = revision_store.register_initial_plan_publication_failpoint(
        "project_0001",
        "issue_0001",
        &plan_id,
        &compile_tx.compile_id,
        crate::product::work_item_revision_store::InitialPlanPublicationCheckpoint::PlanActivated,
    );

    let error = engine
        .compile_initial_plan_revision(&accepted_drafts)
        .unwrap_err();
    assert!(error.to_string().contains("PlanActivated"));
    let active_lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert!(active_lineage.active_revision_id.is_some());
    let interrupted_journal = revision_store
        .get_initial_plan_publication_journal(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_tx.compile_id,
        )
        .unwrap();
    assert_eq!(
        interrupted_journal.phase,
        crate::product::work_item_revision_store::InitialPlanPublicationPhase::Prepared
    );

    let session_record = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .unwrap();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let recovered = WorkspaceEngine::new_persistent(
        engine.checkpoint_store.clone(),
        lifecycle,
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let outcome = recovered
        .load_initial_plan_compile_outcome(&compile_tx)
        .unwrap()
        .expect("active outcome after restart");
    let completed_journal = recovered
        .revision_store()
        .get_initial_plan_publication_journal(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_tx.compile_id,
        )
        .unwrap();

    assert_eq!(
        completed_journal.phase,
        crate::product::work_item_revision_store::InitialPlanPublicationPhase::PlanActivated
    );
    assert_eq!(completed_journal.error, None);
    assert_eq!(
        active_lineage.active_revision_id.as_deref(),
        Some(outcome.plan_revision.id.as_str())
    );
}

#[tokio::test]
async fn compile_recovery_continue_finalizes_active_plan_without_new_compile_transaction() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let (mut compile_tx, accepted_drafts) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_continue_active_plan",
        "2026-07-17T00:01:00Z",
    );
    let plan_store = engine.work_item_plan_store().unwrap();
    plan_store.put_compile_transaction(&compile_tx).unwrap();
    let published = engine
        .compile_initial_plan_revision(&accepted_drafts)
        .unwrap();
    compile_tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
    compile_tx.failure_reason = Some("simulated crash after active plan".to_string());
    plan_store.put_compile_transaction(&compile_tx).unwrap();
    engine
        .enter_work_item_plan_compile_recovery(Some(
            "simulated crash after active plan".to_string(),
        ))
        .await;

    let recovery = engine
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .unwrap();

    assert_eq!(recovery, WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
    let transactions = plan_store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(transactions.len(), 1, "Continue must not create a new tx");
    let committed_tx = plan_store
        .get_compile_transaction(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_tx.compile_id,
        )
        .unwrap();
    assert_eq!(committed_tx.status, WorkItemPlanCompileStatus::Committed);
    assert_eq!(
        committed_tx.plan_commit_state,
        WorkItemPlanCommitState::Committed
    );
    assert_eq!(committed_tx.step_cursor, "committed");

    let lifecycle_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(
        lifecycle_plan.work_item_ids,
        published
            .plan_projection_bundle
            .coder_group_context
            .ordered_logical_work_item_ids
    );
    let child_sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .unwrap()
        .into_iter()
        .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
        .collect::<Vec<_>>();
    assert_eq!(child_sessions.len(), published.work_items.len());
    assert_eq!(
        committed_tx
            .child_session_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
        child_sessions
            .iter()
            .map(|session| session.id.clone())
            .collect::<BTreeSet<_>>()
    );
    let reports = engine
        .artifact_versions
        .iter()
        .filter_map(|version| match &version.payload {
            ArtifactPayload::WorkItemPlanCompileReport { compile_report }
                if compile_report.compile_id == compile_tx.compile_id =>
            {
                Some(compile_report.as_ref())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, WorkItemPlanCompileStatus::Committed);
    assert_eq!(reports[0].child_session_ids, committed_tx.child_session_ids);
}

#[tokio::test]
async fn compile_recovery_continue_replays_each_partial_finalizer_checkpoint_after_restart() {
    for (
        checkpoint,
        expected_cursor,
        expected_sessions,
        expected_reports,
        first_session_bound,
        first_context_prepared,
        expected_plan_status,
    ) in [
        (
            WorkItemPlanCompileFinalizerCheckpoint::PlanSummaryPrepared,
            "plan_summary_prepared",
            0,
            0,
            false,
            false,
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        ),
        (
            WorkItemPlanCompileFinalizerCheckpoint::FirstChildSessionEnsured,
            "child_session_001_ensured",
            1,
            0,
            false,
            false,
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        ),
        (
            WorkItemPlanCompileFinalizerCheckpoint::FirstChildBindingEnsured,
            "child_session_001_binding_ensured",
            1,
            0,
            true,
            false,
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        ),
        (
            WorkItemPlanCompileFinalizerCheckpoint::FirstChildContextPrepared,
            "child_session_001_context_prepared",
            1,
            0,
            true,
            true,
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        ),
        (
            WorkItemPlanCompileFinalizerCheckpoint::CompileReportPersisted,
            "compile_report_persisted",
            2,
            1,
            true,
            true,
            crate::product::models::IssueWorkItemPlanStatus::Confirmed,
        ),
    ] {
        let (_tmp, lifecycle, plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        let compile_id = format!("compile_finalizer_{checkpoint:?}").to_lowercase();
        let (mut compile_tx, accepted_drafts) = prepare_initial_compile_transaction(
            &engine,
            &lifecycle,
            &plan_id,
            &compile_id,
            "2026-07-17T00:01:20Z",
        );
        let plan_store = engine.work_item_plan_store().unwrap();
        plan_store.put_compile_transaction(&compile_tx).unwrap();
        let published = engine
            .compile_initial_plan_revision(&accepted_drafts)
            .unwrap();
        compile_tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
        compile_tx.failure_reason = Some(format!("simulated {checkpoint:?} crash"));
        plan_store.put_compile_transaction(&compile_tx).unwrap();
        engine
            .enter_work_item_plan_compile_recovery(Some(format!("simulated {checkpoint:?} crash")))
            .await;
        let _failpoint = engine.register_work_item_plan_compile_finalizer_failpoint(
            &compile_tx.compile_id,
            checkpoint,
        );

        let error = engine
            .handle_work_item_plan_compile_recovery_action(
                WorkItemPlanCompileRecoveryActionDto::Continue,
                None,
            )
            .await
            .unwrap_err();

        assert!(error.contains(&format!("{checkpoint:?}")));
        let interrupted_tx = plan_store
            .get_compile_transaction(
                "project_0001",
                "issue_0001",
                &plan_id,
                &compile_tx.compile_id,
            )
            .unwrap();
        assert_eq!(
            interrupted_tx.plan_commit_state,
            WorkItemPlanCommitState::NotStarted
        );
        assert_eq!(
            interrupted_tx.status,
            WorkItemPlanCompileStatus::RecoveryRequired
        );
        assert_eq!(interrupted_tx.step_cursor, expected_cursor);
        assert_eq!(interrupted_tx.child_session_ids.len(), expected_sessions);
        assert_eq!(
            lifecycle
                .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
                .unwrap()
                .status,
            expected_plan_status,
            "Plan 只能在所有子 Workspace RuntimeBinding 与 Context 已准备后确认"
        );
        let child_sessions = lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .unwrap()
            .into_iter()
            .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
            .collect::<Vec<_>>();
        assert_eq!(child_sessions.len(), expected_sessions);
        if let Some(first_session) = child_sessions.first() {
            assert_eq!(
                first_session.work_item_runtime_binding.is_some(),
                first_session_bound
            );
            assert_eq!(
                first_session.messages.first().is_some_and(|message| {
                    message.role == "system" && message.content.contains("[work_item_context]")
                }),
                first_context_prepared
            );
        }
        assert_eq!(
            engine
                .artifact_versions
                .iter()
                .filter(|version| {
                    matches!(
                        &version.payload,
                        ArtifactPayload::WorkItemPlanCompileReport { compile_report }
                            if compile_report.compile_id == compile_tx.compile_id
                    )
                })
                .count(),
            expected_reports
        );

        let session_record = lifecycle
            .get_workspace_session(&engine.session.session_id)
            .unwrap();
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut recovered = WorkspaceEngine::new_persistent(
            engine.checkpoint_store.clone(),
            lifecycle.clone(),
            event_tx,
            WorkspaceSession::from_record(session_record),
        );
        let recovery = recovered
            .handle_work_item_plan_compile_recovery_action(
                WorkItemPlanCompileRecoveryActionDto::Continue,
                None,
            )
            .await
            .unwrap();

        assert_eq!(recovery, WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
        let committed_tx = plan_store
            .get_compile_transaction(
                "project_0001",
                "issue_0001",
                &plan_id,
                &compile_tx.compile_id,
            )
            .unwrap();
        assert_eq!(committed_tx.status, WorkItemPlanCompileStatus::Committed);
        assert_eq!(
            committed_tx.plan_commit_state,
            WorkItemPlanCommitState::Committed
        );
        assert_eq!(committed_tx.step_cursor, "committed");
        assert_eq!(
            plan_store
                .list_compile_transactions("project_0001", "issue_0001", &plan_id)
                .unwrap()
                .len(),
            1
        );
        let child_sessions = lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .unwrap()
            .into_iter()
            .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
            .collect::<Vec<_>>();
        assert_eq!(child_sessions.len(), published.work_items.len());
        assert_eq!(
            child_sessions
                .iter()
                .map(|session| session.entity_id.clone())
                .collect::<BTreeSet<_>>(),
            published
                .work_items
                .iter()
                .map(|item| item.work_item_revision.logical_work_item_id.clone())
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(
            recovered
                .artifact_versions
                .iter()
                .filter(|version| {
                    matches!(
                        &version.payload,
                        ArtifactPayload::WorkItemPlanCompileReport { compile_report }
                            if compile_report.compile_id == compile_tx.compile_id
                    )
                })
                .count(),
            1,
            "report replay must be idempotent"
        );
    }
}

#[tokio::test]
async fn compile_recovery_continue_replays_pre_active_publication_with_same_tx_after_restart() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outline_payload = engine.session.artifact.clone().unwrap();
    engine.update_artifact(outline_payload).await;
    let (compile_tx, accepted_drafts) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_continue_pre_active",
        "2026-07-17T00:01:40Z",
    );
    let plan_store = engine.work_item_plan_store().unwrap();
    plan_store.put_compile_transaction(&compile_tx).unwrap();
    let revision_store = engine.revision_store();
    let _failpoint = revision_store.register_initial_plan_publication_failpoint(
        "project_0001",
        "issue_0001",
        &plan_id,
        &compile_tx.compile_id,
        crate::product::work_item_revision_store::InitialPlanPublicationCheckpoint::LineageWritten,
    );

    let error = engine
        .compile_initial_plan_revision(&accepted_drafts)
        .unwrap_err();
    assert!(error.to_string().contains("LineageWritten"));
    assert!(engine.mark_latest_compile_transaction_recovery_required(&error.to_string()));
    let inactive_lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(inactive_lineage.active_revision_id, None);
    let prepared_journal = revision_store
        .get_initial_plan_publication_journal(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_tx.compile_id,
        )
        .unwrap();
    assert_eq!(
        prepared_journal.phase,
        crate::product::work_item_revision_store::InitialPlanPublicationPhase::Prepared
    );
    let recovery_tx = plan_store
        .get_compile_transaction(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_tx.compile_id,
        )
        .unwrap();
    assert_eq!(
        recovery_tx.status,
        WorkItemPlanCompileStatus::RecoveryRequired
    );
    engine
        .enter_work_item_plan_compile_recovery(Some(error.to_string()))
        .await;

    let session_record = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .unwrap();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut recovered = WorkspaceEngine::new_persistent(
        engine.checkpoint_store.clone(),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let recovery = recovered
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .unwrap();

    assert_eq!(recovery, WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
    let transactions = plan_store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(transactions.len(), 1, "pre-active replay must reuse the tx");
    assert_eq!(transactions[0].compile_id, compile_tx.compile_id);
    assert_eq!(transactions[0].status, WorkItemPlanCompileStatus::Committed);
    assert_eq!(
        transactions[0].plan_commit_state,
        WorkItemPlanCommitState::Committed
    );
    let active_lineage = recovered
        .revision_store()
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert!(active_lineage.active_revision_id.is_some());
    let completed_journal = recovered
        .revision_store()
        .get_initial_plan_publication_journal(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_tx.compile_id,
        )
        .unwrap();
    assert_eq!(
        completed_journal.phase,
        crate::product::work_item_revision_store::InitialPlanPublicationPhase::PlanActivated
    );
    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(plan.work_item_ids.len(), 2);
    assert_eq!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .unwrap()
            .into_iter()
            .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
            .count(),
        2
    );
    assert_eq!(
        recovered
            .artifact_versions
            .iter()
            .filter(|version| {
                matches!(
                    &version.payload,
                    ArtifactPayload::WorkItemPlanCompileReport { compile_report }
                        if compile_report.compile_id == compile_tx.compile_id
                )
            })
            .count(),
        1
    );
}

fn prepare_initial_compile_transaction(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
    plan_id: &str,
    compile_id: &str,
    created_at: &str,
) -> (WorkItemPlanCompileTransaction, Vec<WorkItemDraftRevision>) {
    let plan_store = engine.work_item_plan_store().unwrap();
    let index = plan_store
        .load_active_index("project_0001", "issue_0001", plan_id)
        .unwrap()
        .unwrap();
    let outline = engine.latest_work_item_plan_outline_candidate().unwrap();
    let outline_order = work_item_plan_outline_topological_order(&outline.outline).unwrap();
    let draft_records = engine
        .accepted_active_draft_records_for_compile(&plan_store, &index, &outline_order)
        .unwrap();
    let accepted_drafts = draft_records
        .iter()
        .map(work_item_draft_revision_from_record)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let previous_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", plan_id)
        .unwrap();
    (
        WorkItemPlanCompileTransaction {
            compile_id: compile_id.to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: plan_id.to_string(),
            flow_kind: None,
            source_revision_id: None,
            source_revision_ref: None,
            plan_candidate_ir_ref: None,
            mechanical_report_ref: None,
            publication_provenance_ref: None,
            publication_provenance_content_hash: None,
            generation_round_id: index.current_generation_round_id,
            outline_version_ref: outline.outline.id,
            active_draft_ids: draft_records
                .iter()
                .map(|record| record.draft_id.clone())
                .collect(),
            status: WorkItemPlanCompileStatus::Committing,
            plan_commit_state: WorkItemPlanCommitState::NotStarted,
            step_cursor: "committing".to_string(),
            outline_to_work_item_id: BTreeMap::new(),
            outline_to_verification_plan_id: BTreeMap::new(),
            created_work_item_ids: Vec::new(),
            created_verification_plan_ids: Vec::new(),
            child_session_ids: Vec::new(),
            validator_findings: Vec::new(),
            abort_requested_at: None,
            failure_reason: None,
            previous_plan_snapshot: previous_plan,
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
            committed_at: None,
        },
        accepted_drafts,
    )
}
