use std::path::PathBuf;
use std::sync::{Arc, Barrier};

use super::setup;
use crate::product::coding_attempt_store::{
    CreateBlockedGateInput, FAILED_CODE_REVIEW_RECOVERY_JOURNAL_FILE,
    FailedCodeReviewRecoveryJournal, FailedCodeReviewRecoveryPhase,
};
use crate::product::coding_models::{
    CodingAgentRole, CodingAttemptStatus, CodingExecutionStage, CodingProviderRole,
    CodingRoleRunRetryMetadata, CodingRoleRunStatus, CodingRoleRunTrigger, CodingTimelineNode,
    CodingTimelineNodeStatus,
};
use crate::product::json_store::{ProductStoreError, read_json, write_json};

#[test]
fn failed_review_retry_identity_accepts_new_legacy_and_normalized_legacy_shapes() {
    let (_tmp, store, attempt, journal) = recovery_boundary_fixture();
    let stale = store
        .get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &journal.expected_stale_role_run_id,
        )
        .expect("stale reviewer run");
    let mut retry = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::RetryReview,
            None,
        )
        .expect("legacy retry run");
    retry.status = CodingRoleRunStatus::Running;
    retry.supersedes_run_id = Some(stale.id.clone());
    retry.retry_metadata = None;
    store
        .save_role_run(&attempt.project_id, &attempt.issue_id, &retry)
        .expect("save legacy retry");
    assert!(crate::product::coding_attempt_store::is_failed_review_manual_retry(&retry, &journal));

    retry.retry_metadata = Some(CodingRoleRunRetryMetadata {
        cycle_id: retry.id.clone(),
        attempt_no: 1,
        prior_run_id: Some(stale.id.clone()),
    });
    store
        .save_role_run(&attempt.project_id, &attempt.issue_id, &retry)
        .expect("normalize legacy retry");
    assert!(crate::product::coding_attempt_store::is_failed_review_manual_retry(&retry, &journal));

    retry.trigger = CodingRoleRunTrigger::ManualRetry;
    assert!(crate::product::coding_attempt_store::is_failed_review_manual_retry(&retry, &journal));
}

#[test]
fn manual_retry_store_write_persists_only_a_fully_linked_run() {
    let (_tmp, store, attempt, journal) = recovery_boundary_fixture();
    let stale = store
        .get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &journal.expected_stale_role_run_id,
        )
        .expect("failed prior reviewer run");

    let retry = store
        .create_manual_retry_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            &stale,
            Some("code_review_provider_interrupted".to_string()),
        )
        .expect("atomic manual retry write");
    let persisted = store
        .get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &retry.id,
        )
        .expect("persisted manual retry");
    let metadata = persisted.retry_metadata.expect("complete retry metadata");

    assert_eq!(persisted.trigger, CodingRoleRunTrigger::ManualRetry);
    assert_eq!(
        persisted.reason_code.as_deref(),
        Some("code_review_provider_interrupted")
    );
    assert_eq!(metadata.cycle_id, persisted.id);
    assert_eq!(metadata.attempt_no, 1);
    assert_eq!(metadata.prior_run_id.as_deref(), Some(stale.id.as_str()));
}

#[test]
fn normalized_legacy_recovery_journal_can_yield_to_plan_repair_after_provider_start() {
    let (_tmp, store, attempt, mut journal) = recovery_boundary_fixture();
    let stale = store
        .get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &journal.expected_stale_role_run_id,
        )
        .expect("stale reviewer run");
    let node = CodingTimelineNode {
        id: "coding_node_0010".to_string(),
        attempt_id: attempt.id.clone(),
        stage: CodingExecutionStage::CodeReview,
        title: "代码审查".to_string(),
        status: CodingTimelineNodeStatus::Running,
        agent_role: Some(CodingAgentRole::Reviewer),
        summary: None,
        started_at: "2026-08-07T00:00:00Z".to_string(),
        completed_at: None,
        artifact_refs: Vec::new(),
    };
    store.save_timeline_node(&attempt, node.clone()).unwrap();
    let mut retry = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::RetryReview,
            None,
        )
        .expect("legacy retry run");
    retry.supersedes_run_id = Some(stale.id.clone());
    retry.retry_metadata = Some(CodingRoleRunRetryMetadata {
        cycle_id: retry.id.clone(),
        attempt_no: 1,
        prior_run_id: Some(stale.id),
    });
    store
        .save_role_run(&attempt.project_id, &attempt.issue_id, &retry)
        .expect("Task 5 normalized legacy retry");
    let retry = store
        .attach_role_run_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &retry.id,
            node.id.clone(),
        )
        .expect("bind recovery retry node");
    store
        .append_role_run_event(
            &attempt,
            &retry,
            crate::product::coding_models::CodingRoleRunEventType::ProviderStart,
            serde_json::json!({"provider": "legacy"}),
        )
        .expect("record ProviderStart");
    journal.retry_role_run_id = Some(retry.id.clone());
    journal.phase = FailedCodeReviewRecoveryPhase::Completed;
    journal.runner_started_at = Some("2026-08-07T00:00:01Z".to_string());
    journal.completed_at = Some("2026-08-07T00:00:01Z".to_string());
    write_json(&current_path(&store, &attempt), &journal).expect("complete legacy journal");

    store
        .ensure_plan_repair_can_win_recovery_arbitration(&attempt)
        .expect("normalized legacy journal yields to plan repair");
    assert!(
        store
            .get_archived_failed_code_review_recovery_journal(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &journal.expected_gate_id,
            )
            .expect("legacy journal archive lookup")
            .is_some()
    );
}

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

pub(super) fn current_path(
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
fn coding_plan_repair_prepare_rechecks_authoritative_amendment_status_before_writing_journal() {
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
fn coding_plan_repair_concurrent_amendment_pause_and_recovery_prepare_leave_no_prepared_journal() {
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

#[test]
fn coding_plan_repair_pause_rolls_back_advanced_failed_review_recovery_prefix() {
    let (_tmp, store, attempt, mut journal) = recovery_boundary_fixture();
    journal = store
        .advance_failed_code_review_recovery_journal(
            &journal,
            FailedCodeReviewRecoveryPhase::AttemptReopened,
            None,
        )
        .unwrap();
    let retry = store
        .ensure_failed_code_review_retry_role_run(&attempt, &journal)
        .unwrap();
    journal = store
        .advance_failed_code_review_recovery_journal(
            &journal,
            FailedCodeReviewRecoveryPhase::RetryRunCreated,
            Some(&retry.id),
        )
        .unwrap();
    journal = store
        .advance_failed_code_review_recovery_journal(
            &journal,
            FailedCodeReviewRecoveryPhase::AttemptRunning,
            Some(&retry.id),
        )
        .unwrap();
    store
        .resolve_failed_code_review_gate_idempotent(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &journal.expected_gate_id,
        )
        .unwrap();
    store
        .advance_failed_code_review_recovery_journal(
            &journal,
            FailedCodeReviewRecoveryPhase::GateResolved,
            Some(&retry.id),
        )
        .unwrap();

    let paused = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::AwaitingPlanAmendment,
        )
        .expect("Plan Repair pause must win over unfinished recovery");

    assert_eq!(paused.status, CodingAttemptStatus::AwaitingPlanAmendment);
    assert!(
        store
            .get_failed_code_review_recovery_journal(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap()
            .iter()
            .all(|run| run.id != retry.id)
    );
    let stale = store
        .get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &journal.expected_stale_role_run_id,
        )
        .unwrap();
    assert_eq!(stale.status, CodingRoleRunStatus::Failed);
    assert_eq!(stale.superseded_by_run_id, None);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap()
            .iter()
            .any(|gate| gate.gate_id == journal.expected_gate_id)
    );
}

#[test]
fn coding_plan_repair_amendment_status_blocks_every_recovery_write_boundary() {
    for boundary in [
        "advance",
        "retry_role",
        "attempt_running",
        "gate_resolve",
        "complete",
    ] {
        let (_tmp, store, attempt, mut journal) = recovery_boundary_fixture();
        if boundary != "advance" {
            journal = store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::AttemptReopened,
                    None,
                )
                .unwrap();
        }
        let retry = if matches!(boundary, "gate_resolve" | "complete") {
            let retry = store
                .ensure_failed_code_review_retry_role_run(&attempt, &journal)
                .unwrap();
            journal = store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::AttemptRunning,
                    Some(&retry.id),
                )
                .unwrap();
            Some(retry)
        } else {
            None
        };
        if boundary == "complete" {
            store
                .resolve_failed_code_review_gate_idempotent(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &journal.expected_gate_id,
                )
                .unwrap();
            journal = store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::GateResolved,
                    Some(&retry.as_ref().unwrap().id),
                )
                .unwrap();
        }
        let mut amendment_attempt = attempt.clone();
        amendment_attempt.status = CodingAttemptStatus::AwaitingPlanAmendment;
        store.save_coding_attempt(&amendment_attempt).unwrap();
        let journal_before = store
            .get_failed_code_review_recovery_journal(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )
            .unwrap();
        let roles_before = store
            .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap();
        let gates_before = store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap();

        let result = match boundary {
            "advance" => store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::AttemptReopened,
                    None,
                )
                .map(|_| ()),
            "retry_role" => store
                .ensure_failed_code_review_retry_role_run(&amendment_attempt, &journal)
                .map(|_| ()),
            "attempt_running" => store
                .reopen_failed_review_attempt_running(&amendment_attempt)
                .map(|_| ()),
            "gate_resolve" => store.resolve_failed_code_review_gate_idempotent(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &journal.expected_gate_id,
            ),
            "complete" => store
                .complete_failed_code_review_recovery_journal(
                    &amendment_attempt,
                    &journal.expected_gate_id,
                )
                .map(|_| ()),
            _ => unreachable!(),
        };

        assert!(
            matches!(result, Err(ProductStoreError::Io(ref message)) if message == "plan_amendment_blocks_provider_run"),
            "{boundary}: unexpected result {result:?}"
        );
        assert_eq!(
            store
                .get_failed_code_review_recovery_journal(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                )
                .unwrap(),
            journal_before,
            "{boundary}: journal changed"
        );
        assert_eq!(
            store
                .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap(),
            roles_before,
            "{boundary}: roles changed"
        );
        assert_eq!(
            store
                .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap(),
            gates_before,
            "{boundary}: gates changed"
        );
    }
}

pub(super) fn recovery_boundary_fixture() -> (
    tempfile::TempDir,
    crate::product::coding_attempt_store::CodingAttemptStore,
    crate::product::coding_models::CodingExecutionAttempt,
    FailedCodeReviewRecoveryJournal,
) {
    let (tmp, store, attempt) = setup();
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .unwrap();
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .unwrap();
    let stale = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0009".to_string()),
        )
        .unwrap();
    let stale = store
        .update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &stale.id,
            CodingRoleRunStatus::Failed,
            Some("code_review_provider_interrupted".to_string()),
        )
        .unwrap();
    let gate = store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: Some("coding_node_0009".to_string()),
                role: Some(CodingProviderRole::CodeReviewer),
                title: "review interrupted".to_string(),
                description: "retry".to_string(),
                reason_code: Some("code_review_provider_interrupted".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: Vec::new(),
            },
        )
        .unwrap();
    let journal = store
        .prepare_failed_code_review_recovery_journal(
            &attempt,
            &gate.gate_id,
            "coding_node_0009",
            &stale.id,
        )
        .unwrap();
    (tmp, store, attempt, journal)
}
