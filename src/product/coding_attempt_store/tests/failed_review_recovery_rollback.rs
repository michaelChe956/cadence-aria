use std::fs;

use super::failed_review_recovery::{current_path, recovery_boundary_fixture};
use crate::product::coding_attempt_store::{
    FailedCodeReviewRecoveryPhase, path_is_regular_file, remove_file_if_exists,
};
use crate::product::coding_models::CodingRoleRunStatus;
use crate::product::json_store::write_json;

#[test]
fn coding_plan_repair_rollback_converges_role_run_crash_prefixes() {
    for prefix in ["retry_created", "stale_cleared", "retry_deleted"] {
        let (_tmp, store, attempt, mut journal) = recovery_boundary_fixture();
        journal = store
            .advance_failed_code_review_recovery_journal(
                &journal,
                FailedCodeReviewRecoveryPhase::AttemptReopened,
                None,
            )
            .expect("advance recovery before retry creation");
        let retry = store
            .ensure_failed_code_review_retry_role_run(&attempt, &journal)
            .expect("create retry role run");
        if prefix != "retry_created" {
            journal = store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::RetryRunCreated,
                    Some(&retry.id),
                )
                .expect("record retry role run");
        }

        let mut stale = store
            .get_role_run(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &journal.expected_stale_role_run_id,
            )
            .expect("stale role run");
        stale.status = CodingRoleRunStatus::Failed;
        stale.superseded_by_run_id = None;
        store
            .save_role_run(&attempt.project_id, &attempt.issue_id, &stale)
            .expect("seed rollback crash prefix");
        if prefix == "retry_deleted" {
            remove_file_if_exists(&store.role_run_path(
                &attempt.project_id,
                &attempt.issue_id,
                &retry,
            ))
            .expect("seed deleted retry prefix");
        }

        store
            .rollback_failed_code_review_recovery_for_plan_amendment_locked(&attempt)
            .unwrap_or_else(|error| panic!("{prefix}: first rollback failed: {error}"));
        store
            .rollback_failed_code_review_recovery_for_plan_amendment_locked(&attempt)
            .unwrap_or_else(|error| panic!("{prefix}: repeated rollback failed: {error}"));

        assert!(
            store
                .get_failed_code_review_recovery_journal(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                )
                .expect("journal lookup")
                .is_none(),
            "{prefix}: journal remains"
        );
        assert!(
            store
                .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .expect("role runs")
                .iter()
                .all(|run| run.id != retry.id),
            "{prefix}: retry remains"
        );
        let stale = store
            .get_role_run(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &journal.expected_stale_role_run_id,
            )
            .expect("stale role run after rollback");
        assert_eq!(stale.status, CodingRoleRunStatus::Failed, "{prefix}");
        assert_eq!(stale.superseded_by_run_id, None, "{prefix}");
    }
}

#[test]
fn coding_plan_repair_rollback_converges_gate_crash_prefixes() {
    for prefix in ["resolved_only", "open_and_resolved", "open_only"] {
        let (_tmp, store, attempt, mut journal) = recovery_boundary_fixture();
        journal.phase = FailedCodeReviewRecoveryPhase::GateResolved;
        write_json(&current_path(&store, &attempt), &journal).expect("seed gate journal phase");

        let gates_root =
            store.blocked_gates_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let open_path = gates_root.join(format!("{}.json", journal.expected_gate_id));
        let resolved_path = gates_root
            .join("resolved")
            .join(format!("{}.json", journal.expected_gate_id));
        if prefix != "open_only" {
            store
                .resolve_failed_code_review_gate_idempotent(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &journal.expected_gate_id,
                )
                .expect("resolve failed review gate");
        }
        if prefix == "open_and_resolved" {
            fs::copy(&resolved_path, &open_path).expect("seed duplicate open gate");
        }

        store
            .rollback_failed_code_review_recovery_for_plan_amendment_locked(&attempt)
            .unwrap_or_else(|error| panic!("{prefix}: first rollback failed: {error}"));
        store
            .rollback_failed_code_review_recovery_for_plan_amendment_locked(&attempt)
            .unwrap_or_else(|error| panic!("{prefix}: repeated rollback failed: {error}"));

        assert!(
            path_is_regular_file(&open_path).expect("open gate metadata"),
            "{prefix}"
        );
        assert!(
            !path_is_regular_file(&resolved_path).expect("resolved gate metadata"),
            "{prefix}: resolved duplicate remains"
        );
    }
}
