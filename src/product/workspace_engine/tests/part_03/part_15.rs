use chrono::DateTime;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::product::models::{
    WorkItemPlanCommitState, WorkItemPlanCompileStatus, WorkItemPlanCompileTransaction,
};
use crate::product::work_item_contract::canonical_contract_hash;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;

#[tokio::test(flavor = "current_thread")]
async fn work_item_plan_initial_compile_phase2_normal_records_transaction_journal() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outline_payload = engine.session.artifact.clone().expect("outline artifact");
    engine.update_artifact(outline_payload).await;
    let session_record = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .expect("persisted plan session");
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new_persistent(
        engine.checkpoint_store.clone(),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let provider_ledger_before = provider_ledger_bytes(&lifecycle);
    let started_before = provider_ledger_started_count(&provider_ledger_before);
    let journal = crate::product::work_item_plan_store::observe_compile_transaction_writes();

    let outcome = engine
        .run_work_item_plan_compile()
        .await
        .expect("legacy initial compile succeeds");
    let snapshots = journal.snapshots();
    let provider_ledger_after = provider_ledger_bytes(&lifecycle);
    let started_after = provider_ledger_started_count(&provider_ledger_after);
    assert_eq!(
        provider_ledger_after, provider_ledger_before,
        "compile must preserve provider ledger bytes rather than merely preserving a zero count"
    );
    let newly_started = started_after
        .checked_sub(started_before)
        .expect("compile cannot remove provider ledger started records");
    assert_eq!(newly_started, 0, "compile cannot start a provider run");
    assert!(
        !drain_compile_events(&mut event_rx)
            .iter()
            .any(|event| matches!(event, EngineEvent::ProviderRunRequested { .. })),
        "initial compile must not request a provider run"
    );

    assert_compile_snapshot_timestamps(&snapshots);
    assert_eq!(
        snapshot_cursors(&snapshots),
        vec![
            "preparing",
            "validating",
            "committing",
            "plan_summary_prepared",
            "child_session_001_ensured",
            "child_session_001_binding_ensured",
            "child_session_001_context_prepared",
            "child_session_002_ensured",
            "child_session_002_binding_ensured",
            "child_session_002_context_prepared",
            "child_workspaces_prepared",
            "plan_confirmed",
            "compile_report_persisted",
            "committed",
        ]
    );
    assert_eq!(
        snapshots
            .iter()
            .map(|tx| tx.status.clone())
            .collect::<Vec<_>>(),
        vec![
            WorkItemPlanCompileStatus::Preparing,
            WorkItemPlanCompileStatus::Validating,
            WorkItemPlanCompileStatus::Committing,
            WorkItemPlanCompileStatus::Committing,
            WorkItemPlanCompileStatus::Committing,
            WorkItemPlanCompileStatus::Committing,
            WorkItemPlanCompileStatus::Committing,
            WorkItemPlanCompileStatus::Committing,
            WorkItemPlanCompileStatus::Committing,
            WorkItemPlanCompileStatus::Committing,
            WorkItemPlanCompileStatus::Committing,
            WorkItemPlanCompileStatus::Committing,
            WorkItemPlanCompileStatus::Committing,
            WorkItemPlanCompileStatus::Committed,
        ]
    );
    assert_eq!(
        snapshots
            .last()
            .expect("committed snapshot")
            .plan_commit_state,
        WorkItemPlanCommitState::Committed
    );

    let observation =
        normalized_initial_compile_observation(&outcome, &snapshots, &lifecycle, &plan_id, &engine);
    assert_eq!(
        observation.created_record_counts,
        serde_json::json!({
            "plan_revision": 1,
            "work_item_revisions": 2,
            "verification_plan_revisions": 2,
            "runtime_bindings": 2,
            "compile_reports": 1,
        })
    );
    assert_eq!(
        observation
            .finalizer
            .get("confirmed_plan_status")
            .and_then(Value::as_str),
        Some("confirmed")
    );
    assert_eq!(
        observation
            .finalizer
            .get("child_session_binding_count")
            .and_then(Value::as_u64),
        Some(2)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn work_item_plan_initial_compile_phase2_validation_failure_records_failed_journal() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let store = engine.work_item_plan_store().expect("work item plan store");
    let mut draft_b = store
        .get_draft_record(
            "project_0001",
            "issue_0001",
            &plan_id,
            "round_0001",
            "draft_outline_b",
        )
        .expect("accepted draft B");
    draft_b.candidate.verification_plan.checks[0].command = Some("rm -rf /".to_string());
    store
        .put_draft_record(&draft_b)
        .expect("save invalid draft");

    let journal = crate::product::work_item_plan_store::observe_compile_transaction_writes();
    let error = engine
        .run_work_item_plan_compile()
        .await
        .expect_err("strict validator must reject incompatible input contract");
    let snapshots = journal.snapshots();

    assert!(
        error.contains("Final Compile strict validator failed"),
        "strict validation failure must retain its legacy diagnostic: {error}"
    );
    assert_compile_snapshot_timestamps(&snapshots);
    assert_eq!(
        snapshots
            .iter()
            .map(|tx| tx.status.clone())
            .collect::<Vec<_>>(),
        vec![
            WorkItemPlanCompileStatus::Preparing,
            WorkItemPlanCompileStatus::Validating,
            WorkItemPlanCompileStatus::Failed,
        ]
    );
    assert_eq!(
        snapshot_cursors(&snapshots),
        vec!["preparing", "validating", "validating"]
    );
    let failed = snapshots.last().expect("failed snapshot");
    assert_eq!(
        failed.plan_commit_state,
        WorkItemPlanCommitState::NotStarted
    );
    assert!(
        failed
            .failure_reason
            .as_deref()
            .is_some_and(|reason| !reason.is_empty())
    );
    assert!(
        failed
            .validator_findings
            .iter()
            .any(|finding| finding.code == "verification_command_unsafe")
    );
    assert!(matches!(
        engine
            .revision_store()
            .get_plan_lineage("project_0001", "issue_0001", &plan_id),
        Err(crate::product::json_store::ProductStoreError::NotFound { .. })
    ));
    assert_eq!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("list sessions")
            .into_iter()
            .filter(|session| {
                session.workspace_type == crate::product::models::WorkspaceType::WorkItem
            })
            .count(),
        0,
        "validator failure cannot create child runtime bindings"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn work_item_plan_initial_compile_phase2_recovery_reuses_compile_id_and_marks_publication_resumed()
 {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let (compile_tx, accepted_drafts) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_phase2_recovery",
        "2026-08-27T00:00:00Z",
    );
    let store = engine.work_item_plan_store().expect("work item plan store");
    let journal = crate::product::work_item_plan_store::observe_compile_transaction_writes();
    store
        .put_compile_transaction(&compile_tx)
        .expect("seed transaction");
    let failpoint = engine
        .revision_store()
        .register_initial_plan_publication_failpoint(
        "project_0001",
        "issue_0001",
        &plan_id,
        &compile_tx.compile_id,
        crate::product::work_item_revision_store::InitialPlanPublicationCheckpoint::LineageWritten,
    );
    let error = engine
        .compile_initial_plan_revision(&accepted_drafts)
        .expect_err("publication failpoint interrupts initial publish");
    drop(failpoint);
    assert!(engine.mark_latest_compile_transaction_recovery_required(&error.to_string()));
    engine
        .enter_work_item_plan_compile_recovery(Some(error.to_string()))
        .await;

    let outcome = engine
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .expect("Continue resumes the original transaction");
    assert_eq!(outcome, WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
    let snapshots = journal.snapshots();

    assert_compile_snapshot_timestamps(&snapshots);
    let resumed_at = snapshot_cursors(&snapshots)
        .iter()
        .position(|cursor| *cursor == "publication_resumed")
        .expect("recovery must record the publication_resumed cursor");
    let first_finalizer = snapshot_cursors(&snapshots)
        .iter()
        .position(|cursor| *cursor == "plan_summary_prepared")
        .expect("recovery must continue into finalizer");
    assert!(
        resumed_at < first_finalizer,
        "publication_resumed must sit between initial publication recovery and finalizer cursors"
    );
    let transactions = store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .expect("list transactions");
    assert_eq!(
        transactions.len(),
        1,
        "recovery must not allocate a new transaction"
    );
    assert_eq!(transactions[0].compile_id, compile_tx.compile_id);
    assert_eq!(transactions[0].status, WorkItemPlanCompileStatus::Committed);
    assert_eq!(transactions[0].step_cursor, "committed");

    let reloaded = engine
        .load_initial_plan_compile_outcome(&transactions[0])
        .expect("load persisted outcome")
        .expect("committed outcome");
    let observation = normalized_initial_compile_observation(
        &reloaded, &snapshots, &lifecycle, &plan_id, &engine,
    );
    assert_eq!(
        observation.created_record_counts,
        serde_json::json!({
            "plan_revision": 1,
            "work_item_revisions": 2,
            "verification_plan_revisions": 2,
            "runtime_bindings": 2,
            "compile_reports": 1,
        })
    );
    let final_transaction = observation
        .transaction_states
        .last()
        .expect("final committed transaction state");
    assert_eq!(
        final_transaction.get("compile_id").and_then(Value::as_str),
        Some("<compile-transaction>")
    );
    assert_eq!(
        final_transaction.get("status").and_then(Value::as_str),
        Some("committed")
    );
    assert_eq!(
        final_transaction
            .get("plan_commit_state")
            .and_then(Value::as_str),
        Some("committed")
    );
    assert_eq!(
        final_transaction.get("step_cursor").and_then(Value::as_str),
        Some("committed")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn work_item_plan_initial_compile_phase2_updated_at_order_is_transient_for_selection_and_continue()
 {
    let (first_selected, first_observation) =
        updated_at_recovery_case("2026-08-27T00:00:30Z", "2026-08-27T00:00:10Z").await;
    let (second_selected, second_observation) =
        updated_at_recovery_case("2026-08-27T00:00:10Z", "2026-08-27T00:00:30Z").await;

    assert_eq!(first_selected, "compile_time_new");
    assert_eq!(second_selected, "compile_time_new");
    assert_eq!(
        first_observation, second_observation,
        "reversing only valid updated_at values must not change matching, Continue cursors, or final artifacts"
    );

    let selected_with_original_created_at =
        select_recovery_transaction_by_created_at("2026-08-27T00:00:00Z", "2026-08-27T00:01:00Z");
    let selected_with_reversed_created_at =
        select_recovery_transaction_by_created_at("2026-08-27T00:01:00Z", "2026-08-27T00:00:00Z");
    assert_eq!(selected_with_original_created_at, "compile_created_new");
    assert_eq!(selected_with_reversed_created_at, "compile_created_old");
}

async fn updated_at_recovery_case(
    old_updated_at: &str,
    new_updated_at: &str,
) -> (String, NormalizedInitialCompileObservation) {
    assert_rfc3339(old_updated_at, "old updated_at");
    assert_rfc3339(new_updated_at, "new updated_at");
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let (mut old_tx, _) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_time_old",
        "2026-08-27T00:00:00Z",
    );
    let (mut new_tx, _) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_time_new",
        "2026-08-27T00:01:00Z",
    );
    old_tx.updated_at = old_updated_at.to_string();
    new_tx.updated_at = new_updated_at.to_string();
    let store = engine.work_item_plan_store().expect("work item plan store");
    store
        .put_compile_transaction(&old_tx)
        .expect("seed old transaction");
    store
        .put_compile_transaction(&new_tx)
        .expect("seed new transaction");

    assert!(engine.mark_latest_compile_transaction_recovery_required("phase2 transient timestamp"));
    let selected = engine
        .latest_work_item_plan_recovery_transaction(&store)
        .expect("latest recovery transaction");
    assert_eq!(selected.compile_id, "compile_time_new");

    old_tx.status = WorkItemPlanCompileStatus::Failed;
    store
        .put_compile_transaction(&old_tx)
        .expect("remove older candidate from recovery matching set");
    let journal = crate::product::work_item_plan_store::observe_compile_transaction_writes();
    engine
        .enter_work_item_plan_compile_recovery(Some("phase2 transient timestamp".to_string()))
        .await;
    engine
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .expect("Continue must use selected transaction despite updated_at order");
    let snapshots = journal.snapshots();
    assert_compile_snapshot_timestamps(&snapshots);
    assert_eq!(
        snapshots.first().map(|tx| tx.step_cursor.as_str()),
        Some("publication_resumed")
    );
    let committed = store
        .get_compile_transaction("project_0001", "issue_0001", &plan_id, &selected.compile_id)
        .expect("selected transaction after Continue");
    assert_eq!(committed.status, WorkItemPlanCompileStatus::Committed);
    let outcome = engine
        .load_initial_plan_compile_outcome(&committed)
        .expect("load completed outcome")
        .expect("completed initial outcome");
    let observation =
        normalized_initial_compile_observation(&outcome, &snapshots, &lifecycle, &plan_id, &engine);
    (selected.compile_id, observation)
}

#[tokio::test(flavor = "current_thread")]
async fn work_item_plan_initial_compile_phase2_abort_and_human_triage_updated_at_are_rfc3339() {
    for action in [
        WorkItemPlanCompileRecoveryActionDto::AbortAndRollback,
        WorkItemPlanCompileRecoveryActionDto::HumanTriage,
    ] {
        let (_tmp, lifecycle, plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        let compile_id = format!("compile_phase2_realtime_{action:?}").to_lowercase();
        let (mut tx, _) = prepare_initial_compile_transaction(
            &engine,
            &lifecycle,
            &plan_id,
            &compile_id,
            "2026-08-27T00:00:00Z",
        );
        tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
        tx.failure_reason = Some("phase2 real-time action fixture".to_string());
        tx.updated_at = "2026-08-27T00:00:01Z".to_string();
        let store = engine.work_item_plan_store().expect("work item plan store");
        store
            .put_compile_transaction(&tx)
            .expect("seed recovery transaction");
        engine
            .enter_work_item_plan_compile_recovery(Some(
                "phase2 real-time action fixture".to_string(),
            ))
            .await;

        let outcome = engine
            .handle_work_item_plan_compile_recovery_action(
                action,
                Some("phase2 operator action".to_string()),
            )
            .await
            .expect("recovery action succeeds");
        assert_eq!(outcome, WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
        let updated = store
            .get_compile_transaction("project_0001", "issue_0001", &plan_id, &tx.compile_id)
            .expect("recovery transaction after action");
        assert_rfc3339(
            &updated.updated_at,
            "Abort/HumanTriage real-time updated_at",
        );
    }
}

fn select_recovery_transaction_by_created_at(old_created_at: &str, new_created_at: &str) -> String {
    assert_rfc3339(old_created_at, "old created_at");
    assert_rfc3339(new_created_at, "new created_at");
    let (_tmp, lifecycle, plan_id, engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let (mut old_tx, _) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_created_old",
        old_created_at,
    );
    let (mut new_tx, _) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_created_new",
        new_created_at,
    );
    old_tx.updated_at = "2026-08-27T00:00:30Z".to_string();
    new_tx.updated_at = "2026-08-27T00:00:10Z".to_string();
    let store = engine.work_item_plan_store().expect("work item plan store");
    store
        .put_compile_transaction(&old_tx)
        .expect("seed old transaction");
    store
        .put_compile_transaction(&new_tx)
        .expect("seed new transaction");
    assert!(engine.mark_latest_compile_transaction_recovery_required("created_at baseline"));
    engine
        .latest_work_item_plan_recovery_transaction(&store)
        .expect("latest created_at transaction")
        .compile_id
}
