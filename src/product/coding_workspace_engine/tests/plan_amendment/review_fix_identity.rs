use super::*;
use std::time::Duration;

#[tokio::test]
async fn coding_amendment_existing_journal_identity_mismatch_is_zero_write_when_dirty() {
    let fixture = amendment_fixture().await;
    prepare_application_phase(&fixture, CodingAmendmentApplicationPhase::Started);
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let mut snapshot = lifecycle
        .load_plan_repair_session_state(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.child_session_id,
        )
        .unwrap()
        .unwrap();
    snapshot.request.trigger_attempt_id = "coding_attempt_forged".to_string();
    lifecycle
        .save_plan_repair_session_state(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.child_session_id,
            &snapshot,
        )
        .unwrap();
    std::fs::write(
        fixture
            .attempt
            .worktree_path
            .as_ref()
            .unwrap()
            .join("dirty.txt"),
        "identity mismatch must win before dirty gate",
    )
    .unwrap();

    let attempt_before = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let journal_before = fixture
        .store
        .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
        .unwrap();
    let gates_before = fixture
        .store
        .list_open_blocked_gates(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let snapshot_before = lifecycle
        .load_plan_repair_session_state(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.child_session_id,
        )
        .unwrap();

    let error = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect_err("identity mismatch must fail before dirty-worktree handling");

    assert!(error.to_string().contains("identity_mismatch"));
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap(),
        attempt_before
    );
    assert_eq!(
        fixture
            .store
            .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
            .unwrap(),
        journal_before
    );
    assert_eq!(
        fixture
            .store
            .list_open_blocked_gates(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap(),
        gates_before
    );
    assert_eq!(
        lifecycle
            .load_plan_repair_session_state(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.child_session_id,
            )
            .unwrap(),
        snapshot_before
    );
}

#[tokio::test]
async fn coding_amendment_existing_journal_identity_mismatch_is_zero_write_at_every_phase() {
    for phase in [
        CodingAmendmentApplicationPhase::Started,
        CodingAmendmentApplicationPhase::PlanBindingWritten,
        CodingAmendmentApplicationPhase::UnitRunsWritten,
        CodingAmendmentApplicationPhase::ResumeTargetWritten,
    ] {
        for dirty in [false, true] {
            assert_existing_journal_identity_mismatch_zero_write(phase.clone(), dirty).await;
        }
    }
}

async fn assert_existing_journal_identity_mismatch_zero_write(
    phase: CodingAmendmentApplicationPhase,
    dirty: bool,
) {
    let fixture = amendment_fixture().await;
    prepare_application_phase(&fixture, phase.clone());
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let mut snapshot = lifecycle
        .load_plan_repair_session_state(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.child_session_id,
        )
        .unwrap()
        .unwrap();
    snapshot.request.trigger_attempt_id = "coding_attempt_forged".to_string();
    lifecycle
        .save_plan_repair_session_state(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.child_session_id,
            &snapshot,
        )
        .unwrap();
    if dirty {
        std::fs::write(
            fixture
                .attempt
                .worktree_path
                .as_ref()
                .unwrap()
                .join("dirty.txt"),
            "identity mismatch must win before dirty gate",
        )
        .unwrap();
    }
    let attempt_before = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let journal_before = fixture
        .store
        .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
        .unwrap();
    let gates_before = fixture
        .store
        .list_open_blocked_gates(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let snapshot_before = lifecycle
        .load_plan_repair_session_state(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.child_session_id,
        )
        .unwrap();

    let error = fixture
        .engine
        .apply_plan_amendment(&attempt_before, &fixture.manifest)
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("identity_mismatch"),
        "{phase:?} {dirty}"
    );
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap(),
        attempt_before,
        "{phase:?} {dirty}"
    );
    assert_eq!(
        fixture
            .store
            .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
            .unwrap(),
        journal_before,
        "{phase:?} {dirty}"
    );
    assert_eq!(
        fixture
            .store
            .list_open_blocked_gates(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap(),
        gates_before,
        "{phase:?} {dirty}"
    );
    assert_eq!(
        lifecycle
            .load_plan_repair_session_state(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.child_session_id,
            )
            .unwrap(),
        snapshot_before,
        "{phase:?} {dirty}"
    );
}

#[tokio::test]
async fn coding_amendment_recovery_does_not_select_historical_completed_journal() {
    let fixture = amendment_fixture().await;
    let applied = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .unwrap();
    let plan = fixture
        .revision_store
        .get_plan_lineage(&applied.project_id, &applied.issue_id, &fixture.plan.id)
        .unwrap();
    let previous_request = fixture
        .revision_store
        .get_repair_request(&plan, &fixture.manifest.repair_request_id)
        .unwrap();
    let mut current_request = previous_request;
    current_request.id = "plan_repair_request_0002".to_string();
    current_request.base_plan_revision_id = fixture.manifest.new_plan_revision_id.clone();
    current_request.amendment_id = Some("plan_amendment_current_missing_lock".to_string());
    current_request.fingerprint = "plan_repair_fingerprint_0002".to_string();
    current_request.status = PlanRepairRequestStatus::Published;
    current_request.created_at = "2026-07-19T00:01:00Z".to_string();
    current_request.updated_at = "2026-07-19T00:01:00Z".to_string();
    fixture
        .revision_store
        .put_repair_request(&plan, &current_request)
        .unwrap();
    let awaiting = fixture
        .store
        .update_attempt_status(
            &applied.project_id,
            &applied.issue_id,
            &applied.id,
            CodingAttemptStatus::AwaitingPlanAmendment,
        )
        .unwrap();
    let historical_journal_before = fixture
        .store
        .get_amendment_application_journal(&awaiting, &fixture.manifest.id)
        .unwrap();

    let error = fixture
        .engine
        .recover_plan_amendment(&awaiting)
        .await
        .expect_err("missing authoritative current amendment identity must fail closed");

    assert!(error.to_string().contains("identity_mismatch"));
    assert_eq!(
        fixture
            .store
            .get_attempt(&awaiting.project_id, &awaiting.issue_id, &awaiting.id)
            .unwrap()
            .status,
        CodingAttemptStatus::AwaitingPlanAmendment
    );
    assert_eq!(
        fixture
            .store
            .get_amendment_application_journal(&awaiting, &fixture.manifest.id)
            .unwrap(),
        historical_journal_before
    );
}

#[tokio::test]
async fn coding_amendment_journal_rejects_non_adjacent_phase_advance() {
    let fixture = amendment_fixture().await;
    let started = fixture
        .store
        .load_or_prepare_amendment_application(&fixture.attempt, &fixture.manifest)
        .unwrap();

    let error = fixture
        .store
        .advance_amendment_application_journal(
            &fixture.attempt,
            &fixture.manifest.id,
            CodingAmendmentApplicationPhase::UnitRunsWritten,
            None,
            "2026-07-19T00:02:00Z".to_string(),
        )
        .expect_err("journal phase must advance one durable boundary at a time");

    assert!(error.to_string().contains("identity_mismatch"));
    assert_eq!(
        fixture
            .store
            .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
            .unwrap(),
        started
    );
}

#[tokio::test]
async fn coding_amendment_journal_rejects_forged_deterministic_id() {
    let fixture = amendment_fixture().await;
    fixture
        .store
        .load_or_prepare_amendment_application(&fixture.attempt, &fixture.manifest)
        .unwrap();
    let path = fixture.store.amendment_application_path(
        &fixture.attempt.project_id,
        &fixture.attempt.issue_id,
        &fixture.attempt.id,
        &fixture.manifest.id,
    );
    let mut journal: crate::product::coding_models::CodingAmendmentApplicationJournal =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    journal.id = "coding_amendment_application_forged".to_string();
    std::fs::write(&path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();

    let error = fixture
        .store
        .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
        .expect_err("journal ID must be deterministic for the amendment");

    assert!(error.to_string().contains("identity_mismatch"));
}

#[tokio::test]
async fn coding_amendment_recovery_rejects_corrupt_skipped_binding_prefix_without_writes() {
    let fixture = amendment_fixture().await;
    prepare_application_phase(
        &fixture,
        CodingAmendmentApplicationPhase::PlanBindingWritten,
    );
    let binding_path = fixture.store.plan_binding_path(
        &fixture.attempt.project_id,
        &fixture.attempt.issue_id,
        &fixture.attempt.id,
    );
    let mut binding: crate::product::coding_models::CodingAttemptPlanBinding =
        serde_json::from_slice(&std::fs::read(&binding_path).unwrap()).unwrap();
    binding.bound_plan_revision_id = fixture.manifest.previous_plan_revision_id.clone();
    binding.applied_amendment_ids.clear();
    std::fs::write(&binding_path, serde_json::to_vec_pretty(&binding).unwrap()).unwrap();
    let attempt_before = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let journal_before = fixture
        .store
        .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
        .unwrap();

    let error = fixture
        .engine
        .recover_plan_amendment(&attempt_before)
        .await
        .expect_err("recovery must verify a skipped binding durable prefix");

    assert!(error.to_string().contains("identity_mismatch"));
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap(),
        attempt_before
    );
    assert_eq!(
        fixture
            .store
            .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
            .unwrap(),
        journal_before
    );
}

#[tokio::test]
async fn coding_amendment_recovery_rejects_corrupt_resume_target_prefix_without_writes() {
    let fixture = amendment_fixture().await;
    prepare_application_phase(
        &fixture,
        CodingAmendmentApplicationPhase::ResumeTargetWritten,
    );
    let mut forged_attempt = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    forged_attempt.active_unit_id = Some("coding_unit_0002".to_string());
    forged_attempt.current_work_item_id = Some("work_item_0002".to_string());
    fixture
        .store
        .write_coding_attempt_for_test(&forged_attempt)
        .unwrap();
    let attempt_before = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let journal_before = fixture
        .store
        .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
        .unwrap();

    let error = fixture
        .engine
        .recover_plan_amendment(&attempt_before)
        .await
        .expect_err("recovery must verify the persisted resume target");

    assert!(error.to_string().contains("identity_mismatch"));
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap(),
        attempt_before
    );
    assert_eq!(
        fixture
            .store
            .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
            .unwrap(),
        journal_before
    );
}

#[tokio::test]
async fn coding_amendment_completed_prefix_rejects_corrupt_session_identity_without_writes() {
    let fixture = amendment_fixture().await;
    prepare_application_phase(
        &fixture,
        CodingAmendmentApplicationPhase::ResumeTargetWritten,
    );
    fixture
        .store
        .advance_amendment_application_journal(
            &fixture.attempt,
            &fixture.manifest.id,
            CodingAmendmentApplicationPhase::Completed,
            None,
            "2026-07-19T00:03:00Z".to_string(),
        )
        .unwrap();
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let mut snapshot = lifecycle
        .load_plan_repair_session_state(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.child_session_id,
        )
        .unwrap()
        .unwrap();
    snapshot.amendment = None;
    lifecycle
        .save_plan_repair_session_state(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.child_session_id,
            &snapshot,
        )
        .unwrap();
    let attempt_before = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let request_before = fixture
        .revision_store
        .get_repair_request(&fixture.plan, &fixture.manifest.repair_request_id)
        .unwrap();
    let journal_before = fixture
        .store
        .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
        .unwrap();

    let error = fixture
        .engine
        .recover_plan_amendment(&attempt_before)
        .await
        .expect_err("Completed must prove session identity before finalization writes");

    assert!(error.to_string().contains("identity_mismatch"));
    assert_eq!(
        fixture
            .revision_store
            .get_repair_request(&fixture.plan, &fixture.manifest.repair_request_id)
            .unwrap(),
        request_before
    );
    assert_eq!(
        fixture
            .store
            .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
            .unwrap(),
        journal_before
    );
}

#[tokio::test]
async fn coding_amendment_binding_write_rejects_replaced_lineage_lock() {
    let fixture = amendment_fixture().await;
    fixture
        .store
        .load_or_prepare_amendment_application(&fixture.attempt, &fixture.manifest)
        .unwrap();
    let binding_before = fixture.store.get_plan_binding(&fixture.attempt).unwrap();
    fixture
        .revision_store
        .release_active_amendment(&fixture.plan, &fixture.manifest.id)
        .unwrap();
    let plan = fixture
        .revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.plan.id,
        )
        .unwrap();
    fixture
        .revision_store
        .acquire_active_amendment(&plan, "plan_amendment_replacement")
        .unwrap();

    let error = fixture
        .store
        .update_plan_binding_from_manifest(&fixture.attempt, &fixture.manifest)
        .expect_err("binding CAS must reject a replaced amendment lineage");

    assert!(error.to_string().contains("identity_mismatch"));
    assert_eq!(
        fixture.store.get_plan_binding(&fixture.attempt).unwrap(),
        binding_before
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coding_amendment_arbitration_rechecks_lineage_before_first_write() {
    let fixture = amendment_fixture().await;
    let binding_before = fixture.store.get_plan_binding(&fixture.attempt).unwrap();
    let guard = fixture
        .store
        .acquire_amendment_application_arbitration(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .await
        .unwrap();
    let attempt = fixture.attempt.clone();
    let manifest = fixture.manifest.clone();
    let store = fixture.store.clone();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut task = tokio::spawn(async move {
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
            .apply_plan_amendment(&attempt, &manifest)
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut task)
            .await
            .is_err(),
        "application must wait at the per-attempt arbitration boundary"
    );
    fixture
        .revision_store
        .release_active_amendment(&fixture.plan, &fixture.manifest.id)
        .unwrap();
    let plan = fixture
        .revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.plan.id,
        )
        .unwrap();
    fixture
        .revision_store
        .acquire_active_amendment(&plan, "plan_amendment_replacement")
        .unwrap();
    drop(guard);

    let error = task
        .await
        .unwrap()
        .expect_err("worker must revalidate lineage after arbitration wait");

    assert!(error.to_string().contains("identity_mismatch"));
    assert_eq!(
        fixture.store.get_plan_binding(&fixture.attempt).unwrap(),
        binding_before
    );
    assert!(
        fixture
            .store
            .list_amendment_application_journals(&fixture.attempt)
            .unwrap()
            .is_empty()
    );
}
