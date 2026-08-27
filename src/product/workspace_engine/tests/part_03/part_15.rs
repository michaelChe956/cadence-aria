use chrono::DateTime;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::product::models::{
    WorkItemPlanCommitState, WorkItemPlanCompileStatus, WorkItemPlanCompileTransaction,
};
use crate::product::work_item_contract::canonical_contract_hash;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;

/// 3.2 输入抽取前的 legacy compile 三层语义基线。
///
/// 所有时间均在验证 RFC3339 与 transaction 内 `created_at` 恒定之后移除；动态 ID
/// 仅替换为保持引用关系的稳定占位符，不把 ID 字段整体删除。
#[derive(Debug, PartialEq, Eq)]
struct NormalizedInitialCompileObservation {
    plan: Value,
    work_items: Value,
    verification_plans: Value,
    runtime_bindings: Value,
    plan_projection: Value,
    work_item_projections: Value,
    projection_hashes: Value,
    transaction_states: Vec<Value>,
    created_record_counts: Value,
    finalizer: Value,
}

#[test]
fn work_item_plan_initial_compile_pure_prepare_is_deterministic_without_store_handles() {
    let (_tmp, lifecycle, plan_id, engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let input = initial_plan_compile_input_from_fixture(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_pure_prepare",
        "2026-08-27T00:00:00Z",
    );

    let outline_to_work_item_id = input
        .outline_order
        .iter()
        .enumerate()
        .map(|(index, outline_id)| {
            (
                outline_id.clone(),
                format!("work_item_{}_{:03}", input.compile_id, index + 1),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let outline_to_verification_plan_id = input
        .outline_order
        .iter()
        .enumerate()
        .map(|(index, outline_id)| {
            (
                outline_id.clone(),
                format!("verification_plan_{}_{:03}", input.compile_id, index + 1),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let legacy_projection = engine
        .project_work_item_plan_drafts_for_compile(
            &input.previous_plan,
            &input.draft_records,
            WorkItemPlanCompileProjectionContext {
                outline_order: &input.outline_order,
                outline_to_work_item_id: &outline_to_work_item_id,
                outline_to_verification_plan_id: &outline_to_verification_plan_id,
                repository_id: &input.repository_id,
                logical_targets: input.logical_targets.as_ref(),
                now: &input.now,
            },
            &input.change_order,
        )
        .expect("legacy projection accepts the same fixture");
    let first =
        prepare_initial_plan_compile(input.clone(), InitialPlanCompileDurableContext::legacy())
            .expect("pure prepare accepts legacy input");
    let second = prepare_initial_plan_compile(input, InitialPlanCompileDurableContext::legacy())
        .expect("the same pure input prepares deterministically");

    assert_eq!(first, second);
    assert_eq!(
        (
            first.compiled_plan.clone(),
            first.work_items.clone(),
            first.verification_plans.clone(),
        ),
        legacy_projection,
        "legacy adapter and pure core must retain the same projection and validator input"
    );
    assert_eq!(
        first.transaction.status,
        WorkItemPlanCompileStatus::Preparing
    );
    assert_eq!(first.transaction.created_at, "2026-08-27T00:00:00Z");
    assert_eq!(
        first.transaction.effective_flow_kind(),
        WorkItemPlanFlowKind::Legacy
    );
    assert_eq!(
        first.compiled_plan.work_item_ids,
        vec![
            "work_item_compile_pure_prepare_001",
            "work_item_compile_pure_prepare_002"
        ]
    );
}

#[test]
fn work_item_plan_initial_compile_durable_context_is_legacy_compatible_and_fail_closed() {
    let (_tmp, lifecycle, plan_id, engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let input = initial_plan_compile_input_from_fixture(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_durable_context",
        "2026-08-27T00:00:00Z",
    );
    let prepared = prepare_initial_plan_compile(input, InitialPlanCompileDurableContext::legacy())
        .expect("legacy durable context is valid");

    let mut legacy_json =
        serde_json::to_value(&prepared.transaction).expect("serialize transaction");
    let legacy_object = legacy_json
        .as_object_mut()
        .expect("transaction serializes as an object");
    for field in [
        "flow_kind",
        "source_revision_id",
        "source_revision_ref",
        "plan_candidate_ir_ref",
        "mechanical_report_ref",
        "publication_provenance_ref",
        "publication_provenance_content_hash",
    ] {
        assert!(
            legacy_object.remove(field).is_none(),
            "legacy None fields are omitted"
        );
    }
    let legacy: WorkItemPlanCompileTransaction =
        serde_json::from_value(legacy_json).expect("legacy transaction remains readable");
    assert_eq!(legacy.effective_flow_kind(), WorkItemPlanFlowKind::Legacy);
    assert_eq!(legacy.flow_kind, None);
    assert_eq!(legacy.source_revision_id, None);
    assert_eq!(legacy.source_revision_ref, None);
    assert_eq!(legacy.plan_candidate_ir_ref, None);
    assert_eq!(legacy.mechanical_report_ref, None);
    assert_eq!(legacy.publication_provenance_ref, None);
    assert_eq!(legacy.publication_provenance_content_hash, None);

    let incomplete = InitialPlanCompileDurableContext {
        flow_kind: Some(WorkItemPlanFlowKind::SingleCandidate),
        source_revision_id: Some("revision_001".to_string()),
        source_revision_ref: Some("revision://001".to_string()),
        plan_candidate_ir_ref: Some("ir://plan/001".to_string()),
        mechanical_report_ref: Some("report://001".to_string()),
        publication_provenance_ref: Some("provenance://001".to_string()),
        publication_provenance_content_hash: None,
    };
    let error = incomplete
        .validate()
        .expect_err("single candidate context requires every durable ref");
    assert!(error.contains("publication_provenance_content_hash"));
}

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

fn initial_plan_compile_input_from_fixture(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
    plan_id: &str,
    compile_id: &str,
    now: &str,
) -> InitialPlanCompileInput {
    let store = engine.work_item_plan_store().expect("work item plan store");
    let active_index = store
        .load_active_index("project_0001", "issue_0001", plan_id)
        .expect("load active index")
        .expect("active index");
    let outline_candidate = engine
        .latest_work_item_plan_outline_candidate()
        .expect("outline candidate");
    let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)
        .expect("outline order");
    let draft_records = engine
        .accepted_active_draft_records_for_compile(&store, &active_index, &outline_order)
        .expect("accepted drafts");
    let previous_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", plan_id)
        .expect("previous plan");
    let logical_targets = engine
        .logical_work_item_plan_repository_targets(lifecycle, &previous_plan)
        .expect("logical targets");
    let repository_id = if logical_targets.is_none() {
        engine
            .work_item_plan_repository_id(lifecycle, &previous_plan)
            .expect("legacy repository id")
    } else {
        String::new()
    };
    let change_order = draft_batch::compile_support::load_change_order_from_confirmed_design(
        lifecycle,
        &previous_plan,
    )
    .expect("change order");

    InitialPlanCompileInput {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: plan_id.to_string(),
        previous_plan,
        active_index,
        outline_candidate,
        outline_order,
        draft_records,
        logical_targets,
        repository_id,
        change_order,
        compile_id: compile_id.to_string(),
        now: now.to_string(),
    }
}

fn normalized_initial_compile_observation(
    outcome: &InitialPlanCompileOutcome,
    snapshots: &[WorkItemPlanCompileTransaction],
    lifecycle: &LifecycleStore,
    plan_id: &str,
    engine: &WorkspaceEngine,
) -> NormalizedInitialCompileObservation {
    assert_compile_snapshot_timestamps(snapshots);
    assert_projection_hashes(outcome);
    for value in [
        serde_json::to_value(&outcome.plan_revision).expect("serialize plan revision"),
        serde_json::to_value(&outcome.dependency_graph_revision)
            .expect("serialize dependency graph revision"),
        serde_json::to_value(&outcome.validation_report).expect("serialize validation report"),
        serde_json::to_value(&outcome.plan_projection_bundle)
            .expect("serialize plan projection bundle"),
        serde_json::to_value(
            outcome
                .work_items
                .iter()
                .map(|item| {
                    serde_json::json!({
                        "draft_revision": item.draft_revision,
                        "work_item_revision": item.work_item_revision,
                        "verification_plan_revision": item.verification_plan_revision,
                        "projection_bundle": item.projection_bundle,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .expect("serialize compiled work items"),
    ] {
        assert_rfc3339_values(&value);
    }

    let sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list child sessions")
        .into_iter()
        .filter(|session| session.workspace_type == crate::product::models::WorkspaceType::WorkItem)
        .collect::<Vec<_>>();
    assert_eq!(sessions.len(), outcome.work_items.len());
    for session in &sessions {
        assert_rfc3339_values(&serde_json::to_value(session).expect("serialize child session"));
        let binding = session
            .work_item_runtime_binding
            .as_ref()
            .expect("finalizer must bind every child session");
        let item = outcome
            .work_items
            .iter()
            .find(|item| item.work_item_revision.logical_work_item_id == session.entity_id)
            .expect("child session has compiled work item");
        assert_eq!(binding.plan_id, plan_id);
        assert_eq!(binding.plan_revision_id, outcome.plan_revision.id);
        assert_eq!(binding.work_item_revision_id, item.work_item_revision.id);
        assert_eq!(binding.projection_bundle_id, item.projection_bundle.id);
        assert_eq!(
            binding.verification_plan_revision_id,
            item.verification_plan_revision.id
        );
        assert_eq!(
            binding.canonical_contract_hash,
            item.work_item_revision.canonical_contract_hash
        );
        assert_eq!(
            binding.human_projection_hash,
            item.projection_bundle.human_projection_hash
        );
        assert_eq!(
            binding.coder_projection_hash,
            item.projection_bundle.coder_projection_hash
        );
        assert_eq!(
            binding.reviewer_projection_hash,
            item.projection_bundle.reviewer_projection_hash
        );
    }

    let confirmed_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", plan_id)
        .expect("confirmed plan");
    assert_eq!(
        confirmed_plan.work_item_ids,
        outcome
            .plan_projection_bundle
            .coder_group_context
            .ordered_logical_work_item_ids
    );
    let reports = engine
        .artifact_versions
        .iter()
        .filter_map(|version| match &version.payload {
            crate::web::workspace_ws_types::ArtifactPayload::WorkItemPlanCompileReport {
                compile_report,
            } => Some(compile_report.as_ref().clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        reports.len(),
        1,
        "finalizer persists exactly one compile report"
    );
    assert_eq!(reports[0].status, WorkItemPlanCompileStatus::Committed);
    assert_eq!(reports[0].child_session_ids.len(), sessions.len());

    let ids = stable_dynamic_id_map(outcome, snapshots, &sessions);
    let normalize = |value: Value| normalize_value(value, &ids);
    NormalizedInitialCompileObservation {
        plan: normalize(serde_json::to_value(&outcome.plan_revision).expect("serialize plan")),
        work_items: normalize(
            serde_json::to_value(
                outcome
                    .work_items
                    .iter()
                    .map(|item| &item.work_item_revision)
                    .collect::<Vec<_>>(),
            )
            .expect("serialize work items"),
        ),
        verification_plans: normalize(
            serde_json::to_value(
                outcome
                    .work_items
                    .iter()
                    .map(|item| &item.verification_plan_revision)
                    .collect::<Vec<_>>(),
            )
            .expect("serialize verification plans"),
        ),
        runtime_bindings: normalize(
            serde_json::to_value(
                sessions
                    .iter()
                    .map(|session| {
                        (
                            &session.id,
                            &session.entity_id,
                            &session.work_item_runtime_binding,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .expect("serialize runtime bindings"),
        ),
        plan_projection: normalize(
            serde_json::to_value(&outcome.plan_projection_bundle)
                .expect("serialize plan projection"),
        ),
        work_item_projections: normalize(
            serde_json::to_value(
                outcome
                    .work_items
                    .iter()
                    .map(|item| &item.projection_bundle)
                    .collect::<Vec<_>>(),
            )
            .expect("serialize work item projections"),
        ),
        projection_hashes: normalize(serde_json::json!({
            "plan": {
                "human": outcome.plan_projection_bundle.human_group_projection_hash,
                "coder": outcome.plan_projection_bundle.coder_group_context_hash,
                "reviewer": outcome.plan_projection_bundle.reviewer_group_matrix_hash,
            },
            "work_items": outcome.work_items.iter().map(|item| serde_json::json!({
                "logical_work_item_id": item.work_item_revision.logical_work_item_id,
                "canonical_contract": item.work_item_revision.canonical_contract_hash,
                "human": item.projection_bundle.human_projection_hash,
                "coder": item.projection_bundle.coder_projection_hash,
                "reviewer": item.projection_bundle.reviewer_projection_hash,
            })).collect::<Vec<_>>(),
        })),
        transaction_states: snapshots
            .iter()
            .map(|tx| {
                normalize(
                    serde_json::to_value(tx)
                        .expect("serialize complete transaction journal snapshot"),
                )
            })
            .collect(),
        created_record_counts: serde_json::json!({
            "plan_revision": 1,
            "work_item_revisions": outcome.work_items.len(),
            "verification_plan_revisions": outcome.work_items.len(),
            "runtime_bindings": sessions.len(),
            "compile_reports": reports.len(),
        }),
        finalizer: normalize(serde_json::json!({
            "confirmed_plan_status": confirmed_plan.status,
            "confirmed_work_item_ids": confirmed_plan.work_item_ids,
            "report": reports[0],
            "child_session_binding_count": sessions.len(),
            "child_session_ids": sessions.iter().map(|session| &session.id).collect::<Vec<_>>(),
        })),
    }
}

fn provider_ledger_bytes(lifecycle: &LifecycleStore) -> Vec<u8> {
    let root = lifecycle
        .app_paths()
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("provider-runs");
    let mut files = Vec::new();
    collect_provider_ledger_files(&root, &root, &mut files);
    files.sort();
    files.into_iter().fold(Vec::new(), |mut snapshot, path| {
        let relative = path
            .strip_prefix(&root)
            .expect("provider ledger file below root");
        snapshot.extend_from_slice(relative.to_string_lossy().as_bytes());
        snapshot.push(0);
        snapshot.extend_from_slice(&std::fs::read(path).expect("read provider ledger bytes"));
        snapshot.push(0);
        snapshot
    })
}

fn collect_provider_ledger_files(
    root: &std::path::Path,
    current: &std::path::Path,
    files: &mut Vec<std::path::PathBuf>,
) {
    let entries = match std::fs::read_dir(current) {
        Ok(entries) => entries,
        Err(error) if current == root && error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!("read provider ledger {}: {error}", current.display()),
    };
    for entry in entries {
        let path = entry.expect("read provider ledger directory entry").path();
        if path.is_dir() {
            collect_provider_ledger_files(root, &path, files);
        } else {
            files.push(path);
        }
    }
}

fn provider_ledger_started_count(snapshot: &[u8]) -> usize {
    if snapshot.is_empty() {
        return 0;
    }
    let mut count = 0;
    for value in snapshot.split(|byte| *byte == 0) {
        let Ok(value) = serde_json::from_slice::<Value>(value) else {
            continue;
        };
        count += json_started_count(&value);
    }
    count
}

fn json_started_count(value: &Value) -> usize {
    match value {
        Value::Array(values) => values.iter().map(json_started_count).sum(),
        Value::Object(values) => {
            usize::from(values.get("started") == Some(&Value::Bool(true)))
                + values.values().map(json_started_count).sum::<usize>()
        }
        _ => 0,
    }
}

fn drain_compile_events(
    event_rx: &mut tokio::sync::mpsc::Receiver<EngineEvent>,
) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        events.push(event);
    }
    events
}

fn stable_dynamic_id_map(
    outcome: &InitialPlanCompileOutcome,
    snapshots: &[WorkItemPlanCompileTransaction],
    sessions: &[crate::product::models::WorkspaceSessionRecord],
) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    let mut add = |id: &str, placeholder: String| {
        if !id.is_empty() {
            map.entry(id.to_string()).or_insert(placeholder);
        }
    };
    for tx in snapshots {
        add(&tx.compile_id, "<compile-transaction>".to_string());
        for (outline_id, id) in &tx.outline_to_work_item_id {
            add(id, format!("<work-item-{outline_id}>"));
        }
        for (outline_id, id) in &tx.outline_to_verification_plan_id {
            add(id, format!("<verification-plan-{outline_id}>"));
        }
        for (index, id) in tx.created_work_item_ids.iter().enumerate() {
            let logical_id = outcome
                .work_items
                .get(index)
                .map(|item| item.work_item_revision.logical_work_item_id.as_str())
                .unwrap_or("unknown");
            add(id, format!("<work-item-{logical_id}>"));
        }
        for (index, id) in tx.created_verification_plan_ids.iter().enumerate() {
            let logical_id = outcome
                .work_items
                .get(index)
                .map(|item| item.work_item_revision.logical_work_item_id.as_str())
                .unwrap_or("unknown");
            add(id, format!("<verification-plan-{logical_id}>"));
        }
        for (index, id) in tx.child_session_ids.iter().enumerate() {
            let logical_id = outcome
                .work_items
                .get(index)
                .map(|item| item.work_item_revision.logical_work_item_id.as_str())
                .unwrap_or("unknown");
            add(id, format!("<session-{logical_id}>"));
        }
    }
    add(&outcome.plan_revision.id, "<plan-revision>".to_string());
    add(
        &outcome.dependency_graph_revision.id,
        "<dependency-graph>".to_string(),
    );
    add(
        &outcome.validation_report.id,
        "<validation-report>".to_string(),
    );
    add(
        &outcome.plan_projection_bundle.id,
        "<plan-projection>".to_string(),
    );
    for item in &outcome.work_items {
        let logical_id = &item.work_item_revision.logical_work_item_id;
        add(
            &item.draft_revision.id,
            format!("<draft-revision-{logical_id}>"),
        );
        add(
            &item.work_item_revision.id,
            format!("<work-item-revision-{logical_id}>"),
        );
        add(
            &item.verification_plan_revision.id,
            format!("<verification-plan-revision-{logical_id}>"),
        );
        add(
            &item.projection_bundle.id,
            format!("<work-item-projection-{logical_id}>"),
        );
    }
    for session in sessions {
        add(&session.id, format!("<session-{}>", session.entity_id));
    }
    map
}

fn normalize_value(value: Value, ids: &std::collections::BTreeMap<String, String>) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| normalize_value(value, ids))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .filter(|(key, _)| !key.ends_with("_at"))
                .map(|(key, value)| (key, normalize_value(value, ids)))
                .collect(),
        ),
        Value::String(value) => ids
            .get(&value)
            .cloned()
            .map(Value::String)
            .unwrap_or(Value::String(value)),
        value => value,
    }
}

fn assert_compile_snapshot_timestamps(snapshots: &[WorkItemPlanCompileTransaction]) {
    assert!(
        !snapshots.is_empty(),
        "journal must capture every compile transaction write"
    );
    let created_at = &snapshots[0].created_at;
    assert_rfc3339(created_at, "compile transaction created_at");
    for tx in snapshots {
        assert_rfc3339(&tx.created_at, "compile transaction created_at");
        assert_rfc3339(&tx.updated_at, "compile transaction updated_at");
        if let Some(committed_at) = tx.committed_at.as_deref() {
            assert_rfc3339(committed_at, "compile transaction committed_at");
        }
        assert_eq!(
            &tx.created_at, created_at,
            "all journal snapshots for one transaction retain created_at"
        );
        assert_rfc3339_values(&serde_json::to_value(tx).expect("serialize transaction"));
    }
}

fn assert_rfc3339_values(value: &Value) {
    match value {
        Value::Array(values) => values.iter().for_each(assert_rfc3339_values),
        Value::Object(values) => values.iter().for_each(|(key, value)| {
            if key.ends_with("_at")
                && let Some(value) = value.as_str()
            {
                assert_rfc3339(value, key);
            }
            assert_rfc3339_values(value);
        }),
        _ => {}
    }
}

fn assert_rfc3339(value: &str, field: &str) {
    DateTime::parse_from_rfc3339(value)
        .unwrap_or_else(|error| panic!("{field} must be RFC3339: `{value}` ({error})"));
}

fn snapshot_cursors(snapshots: &[WorkItemPlanCompileTransaction]) -> Vec<&str> {
    snapshots.iter().map(|tx| tx.step_cursor.as_str()).collect()
}

fn assert_projection_hashes(outcome: &InitialPlanCompileOutcome) {
    for item in &outcome.work_items {
        assert_eq!(
            item.work_item_revision.canonical_contract_hash,
            canonical_contract_hash(&item.work_item_revision.canonical_contract)
                .expect("hash canonical contract"),
            "canonical contract hash must match the serialized contract"
        );
        assert_eq!(
            item.projection_bundle.canonical_contract_hash,
            item.work_item_revision.canonical_contract_hash
        );
        assert_eq!(
            item.projection_bundle.human_projection_hash,
            serialized_sha256(&item.projection_bundle.human_projection)
        );
        assert_eq!(
            item.projection_bundle.coder_projection_hash,
            serialized_sha256(&item.projection_bundle.coder_projection)
        );
        assert_eq!(
            item.projection_bundle.reviewer_projection_hash,
            serialized_sha256(&item.projection_bundle.reviewer_projection)
        );
    }
    assert_eq!(
        outcome.plan_projection_bundle.human_group_projection_hash,
        serialized_sha256(&outcome.plan_projection_bundle.human_group_projection)
    );
    assert_eq!(
        outcome.plan_projection_bundle.coder_group_context_hash,
        serialized_sha256(&outcome.plan_projection_bundle.coder_group_context)
    );
    assert_eq!(
        outcome.plan_projection_bundle.reviewer_group_matrix_hash,
        serialized_sha256(&outcome.plan_projection_bundle.reviewer_group_matrix)
    );
}

fn serialized_sha256<T: Serialize>(value: &T) -> String {
    hex::encode(Sha256::digest(
        serde_json::to_vec(value).expect("serialize semantic artifact for hash assertion"),
    ))
}
