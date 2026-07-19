use std::path::PathBuf;
use std::sync::{Arc, Barrier};

use super::setup;
use crate::product::coding_attempt_store::{
    FAILED_CODE_REVIEW_RECOVERY_JOURNAL_FILE, FailedCodeReviewRecoveryJournal,
    FailedCodeReviewRecoveryPhase,
};
use crate::product::coding_models::CodingAttemptStatus;
use crate::product::json_store::{ProductStoreError, read_json, write_json};

fn journal(
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    gate_id: &str,
    failed_node_id: &str,
    stale_role_run_id: &str,
    phase: FailedCodeReviewRecoveryPhase,
) -> FailedCodeReviewRecoveryJournal {
    let completed = phase == FailedCodeReviewRecoveryPhase::Completed;
    FailedCodeReviewRecoveryJournal {
        attempt_id: attempt.id.clone(),
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        expected_gate_id: gate_id.to_string(),
        expected_failed_node_id: failed_node_id.to_string(),
        expected_stale_role_run_id: stale_role_run_id.to_string(),
        recovery_key: format!(
            "failed_code_review_recovery:{}:{gate_id}:{stale_role_run_id}",
            attempt.id
        ),
        retry_role_run_id: Some("coding_role_run_0002".to_string()),
        phase,
        runner_started_at: completed.then(|| "2026-07-12T12:53:57Z".to_string()),
        completed_at: completed.then(|| "2026-07-12T12:53:57Z".to_string()),
        created_at: "2026-07-12T12:50:00Z".to_string(),
        updated_at: "2026-07-12T12:53:57Z".to_string(),
    }
}

fn current_path(
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
) -> PathBuf {
    store
        .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .join(FAILED_CODE_REVIEW_RECOVERY_JOURNAL_FILE)
}

fn archived_path(
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    gate_id: &str,
) -> PathBuf {
    store
        .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .join("failed-code-review-recoveries")
        .join("completed")
        .join(format!("{gate_id}.json"))
}

#[test]
fn prepare_archives_completed_journal_before_creating_new_identity() {
    let (_tmp, store, attempt) = setup();
    let old = journal(
        &attempt,
        "coding_blocked_gate_0001",
        "coding_node_0009",
        "coding_role_run_0008",
        FailedCodeReviewRecoveryPhase::Completed,
    );
    write_json(&current_path(&store, &attempt), &old).expect("seed completed journal");

    let current = store
        .prepare_failed_code_review_recovery_journal(
            &attempt,
            "coding_blocked_gate_0007",
            "coding_node_0030",
            "coding_role_run_0029",
        )
        .expect("rotate completed journal");

    assert_eq!(current.expected_gate_id, "coding_blocked_gate_0007");
    assert_eq!(current.phase, FailedCodeReviewRecoveryPhase::Prepared);
    assert_eq!(
        store
            .get_archived_failed_code_review_recovery_journal(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                "coding_blocked_gate_0001",
            )
            .expect("archived journal")
            .expect("completed history"),
        old
    );
}

#[test]
fn prepare_rejects_different_identity_while_current_journal_is_unfinished() {
    let (_tmp, store, attempt) = setup();
    let old = journal(
        &attempt,
        "coding_blocked_gate_0001",
        "coding_node_0009",
        "coding_role_run_0008",
        FailedCodeReviewRecoveryPhase::GateResolved,
    );
    write_json(&current_path(&store, &attempt), &old).expect("seed unfinished journal");

    let rejected = store.prepare_failed_code_review_recovery_journal(
        &attempt,
        "coding_blocked_gate_0007",
        "coding_node_0030",
        "coding_role_run_0029",
    );

    assert!(matches!(
        rejected,
        Err(ProductStoreError::Io(message))
            if message == "coding_failed_review_recovery_state_changed"
    ));
    assert_eq!(
        read_json::<FailedCodeReviewRecoveryJournal>(&current_path(&store, &attempt))
            .expect("unchanged current journal"),
        old
    );
    assert!(!archived_path(&store, &attempt, "coding_blocked_gate_0001").exists());
}

#[test]
fn prepare_rechecks_authoritative_amendment_status_before_writing_journal() {
    for status in [
        CodingAttemptStatus::AwaitingPlanAmendment,
        CodingAttemptStatus::ApplyingPlanAmendment,
        CodingAttemptStatus::AmendmentApplyFailed,
    ] {
        let (_tmp, store, stale_attempt) = setup();
        let mut authoritative = stale_attempt.clone();
        authoritative.status = status.clone();
        store
            .save_coding_attempt(&authoritative)
            .expect("save authoritative amendment status");

        let rejected = store.prepare_failed_code_review_recovery_journal(
            &stale_attempt,
            "coding_blocked_gate_0001",
            "coding_node_0009",
            "coding_role_run_0008",
        );

        assert!(
            matches!(
                rejected,
                Err(ProductStoreError::Io(ref message))
                    if message == "plan_amendment_blocks_provider_run"
            ),
            "{status:?}: unexpected result: {rejected:?}"
        );
        assert!(
            store
                .get_failed_code_review_recovery_journal(
                    &stale_attempt.project_id,
                    &stale_attempt.issue_id,
                    &stale_attempt.id,
                )
                .expect("journal lookup")
                .is_none(),
            "{status:?}: amendment race must not leave a Prepared journal"
        );
    }
}

#[test]
fn concurrent_amendment_pause_and_recovery_prepare_leave_no_prepared_journal() {
    for _ in 0..20 {
        let (_tmp, store, attempt) = setup();
        let running = store
            .update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Running,
            )
            .expect("running attempt");
        let barrier = Arc::new(Barrier::new(3));
        let prepare_store = store.clone();
        let prepare_attempt = running.clone();
        let prepare_barrier = barrier.clone();
        let prepare = std::thread::spawn(move || {
            prepare_barrier.wait();
            prepare_store.prepare_failed_code_review_recovery_journal(
                &prepare_attempt,
                "coding_blocked_gate_0001",
                "coding_node_0009",
                "coding_role_run_0008",
            )
        });
        let pause_store = store.clone();
        let pause_attempt = running.clone();
        let pause_barrier = barrier.clone();
        let pause = std::thread::spawn(move || {
            pause_barrier.wait();
            pause_store.update_attempt_status(
                &pause_attempt.project_id,
                &pause_attempt.issue_id,
                &pause_attempt.id,
                CodingAttemptStatus::AwaitingPlanAmendment,
            )
        });
        barrier.wait();

        let prepared = prepare.join().expect("prepare thread");
        let paused = pause.join().expect("pause thread").expect("pause attempt");

        assert_eq!(paused.status, CodingAttemptStatus::AwaitingPlanAmendment);
        assert!(
            prepared.is_ok()
                || matches!(
                    prepared,
                    Err(ProductStoreError::Io(ref message))
                        if message == "plan_amendment_blocks_provider_run"
                ),
            "unexpected prepare result: {prepared:?}"
        );
        assert!(
            store
                .get_failed_code_review_recovery_journal(
                    &running.project_id,
                    &running.issue_id,
                    &running.id,
                )
                .expect("journal lookup")
                .is_none(),
            "the amendment winner must remove an unadvanced Prepared journal"
        );
    }
}

#[test]
fn prepare_reuses_identical_archive_after_rotation_crash() {
    let (_tmp, store, attempt) = setup();
    let old = journal(
        &attempt,
        "coding_blocked_gate_0001",
        "coding_node_0009",
        "coding_role_run_0008",
        FailedCodeReviewRecoveryPhase::Completed,
    );
    write_json(&current_path(&store, &attempt), &old).expect("seed current journal");
    write_json(
        &archived_path(&store, &attempt, "coding_blocked_gate_0001"),
        &old,
    )
    .expect("seed identical archived journal");

    let current = store
        .prepare_failed_code_review_recovery_journal(
            &attempt,
            "coding_blocked_gate_0007",
            "coding_node_0030",
            "coding_role_run_0029",
        )
        .expect("converge duplicate archive prefix");

    assert_eq!(current.expected_gate_id, "coding_blocked_gate_0007");
    assert_eq!(
        read_json::<FailedCodeReviewRecoveryJournal>(&archived_path(
            &store,
            &attempt,
            "coding_blocked_gate_0001",
        ))
        .expect("preserved archive"),
        old
    );
}

#[test]
fn prepare_rejects_conflicting_archive_without_overwriting_audit_history() {
    let (_tmp, store, attempt) = setup();
    let old = journal(
        &attempt,
        "coding_blocked_gate_0001",
        "coding_node_0009",
        "coding_role_run_0008",
        FailedCodeReviewRecoveryPhase::Completed,
    );
    let conflicting = journal(
        &attempt,
        "coding_blocked_gate_0001",
        "coding_node_0999",
        "coding_role_run_0999",
        FailedCodeReviewRecoveryPhase::Completed,
    );
    write_json(&current_path(&store, &attempt), &old).expect("seed current journal");
    write_json(
        &archived_path(&store, &attempt, "coding_blocked_gate_0001"),
        &conflicting,
    )
    .expect("seed conflicting archive");

    let rejected = store.prepare_failed_code_review_recovery_journal(
        &attempt,
        "coding_blocked_gate_0007",
        "coding_node_0030",
        "coding_role_run_0029",
    );

    assert!(matches!(
        rejected,
        Err(ProductStoreError::Io(message))
            if message == "coding_failed_review_recovery_state_changed"
    ));
    assert_eq!(
        read_json::<FailedCodeReviewRecoveryJournal>(&current_path(&store, &attempt))
            .expect("preserved current journal"),
        old
    );
    assert_eq!(
        read_json::<FailedCodeReviewRecoveryJournal>(&archived_path(
            &store,
            &attempt,
            "coding_blocked_gate_0001",
        ))
        .expect("preserved conflicting archive"),
        conflicting
    );
}

#[test]
fn prepare_recreates_current_journal_after_archive_before_write_crash() {
    let (_tmp, store, attempt) = setup();
    let old = journal(
        &attempt,
        "coding_blocked_gate_0001",
        "coding_node_0009",
        "coding_role_run_0008",
        FailedCodeReviewRecoveryPhase::Completed,
    );
    write_json(
        &archived_path(&store, &attempt, "coding_blocked_gate_0001"),
        &old,
    )
    .expect("seed archive-only crash prefix");

    let current = store
        .prepare_failed_code_review_recovery_journal(
            &attempt,
            "coding_blocked_gate_0007",
            "coding_node_0030",
            "coding_role_run_0029",
        )
        .expect("recreate current journal");

    assert_eq!(current.expected_gate_id, "coding_blocked_gate_0007");
    assert_eq!(current.phase, FailedCodeReviewRecoveryPhase::Prepared);
    assert_eq!(
        store
            .get_archived_failed_code_review_recovery_journal(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                "coding_blocked_gate_0001",
            )
            .expect("archived journal")
            .expect("completed history"),
        old
    );
}

#[test]
fn archived_journal_lookup_rejects_gate_path_escape() {
    let (_tmp, store, attempt) = setup();

    let rejected = store.get_archived_failed_code_review_recovery_journal(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        "../coding_blocked_gate_0001",
    );

    assert!(matches!(
        rejected,
        Err(ProductStoreError::PathEscape(value))
            if value == "../coding_blocked_gate_0001"
    ));
}
