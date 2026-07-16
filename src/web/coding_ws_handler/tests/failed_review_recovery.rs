use tokio::sync::mpsc;

use crate::product::coding_attempt_store::FailedCodeReviewRecoveryPhase;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage, CodingProviderRole,
    CodingRoleRunStatus, CodingRoleRunTrigger,
};
use crate::product::coding_workspace_engine::{
    CodingWorkspaceEngine, recoverable_failed_code_review,
};
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::web::coding_ws_handler::CodingWsInMessage;
use crate::web::coding_ws_handler::socket::failed_code_review_recovery_request;
use crate::web::state::{CodingAttemptRunKey, CodingRunRegistry};

use super::{CodingWsOutMessage, build_coding_session_state};

mod blocked;
mod repeated;
mod runner;
mod support;

use support::{
    FixtureCase, assert_recovery_gate, attempt_data_fingerprint, failed_review_fixture,
    git_status_porcelain,
};

#[derive(Debug, Clone, Copy)]
enum RecoveryPrefix {
    Prepared,
    AttemptReopened,
    RetryRunCreated,
    AttemptRunning,
    GateResolved,
}

#[test]
fn group_failed_review_recovery_gate_reuses_dirty_gate_without_persisting_changes() {
    assert_recovery_gate(CodingAttemptScope::WorkItemGroup);
}

#[test]
fn work_item_failed_review_recovery_gate_reuses_dirty_gate_without_persisting_changes() {
    assert_recovery_gate(CodingAttemptScope::WorkItem);
}

#[test]
fn failed_review_recovery_requires_the_complete_historical_shape() {
    let cases = [
        ("completed attempt", FixtureCase::CompletedAttempt),
        ("aborted attempt", FixtureCase::AbortedAttempt),
        ("testing stage", FixtureCase::TestingStage),
        ("missing completed_at", FixtureCase::MissingCompletedAt),
        (
            "group without active unit",
            FixtureCase::GroupWithoutActiveUnit,
        ),
        (
            "group active unit id mismatch",
            FixtureCase::GroupActiveUnitIdMismatch,
        ),
        ("group unit not running", FixtureCase::GroupUnitNotRunning),
        (
            "latest review completed",
            FixtureCase::LatestReviewCompleted,
        ),
        ("role run node mismatch", FixtureCase::RoleRunNodeMismatch),
        ("role run not running", FixtureCase::RoleRunNotRunning),
        ("missing dirty gate", FixtureCase::MissingDirtyGate),
        ("missing worktree path", FixtureCase::MissingWorktreePath),
        ("missing worktree", FixtureCase::MissingWorktree),
    ];

    for (name, case) in cases {
        let fixture = failed_review_fixture(CodingAttemptScope::WorkItemGroup, case);
        let state = build_coding_session_state(&fixture.store, fixture.attempt)
            .unwrap_or_else(|error| panic!("{name}: build session state: {error}"));
        let CodingWsOutMessage::CodingSessionState { pending_gates, .. } = state else {
            panic!("{name}: expected coding session state");
        };
        assert!(
            pending_gates
                .iter()
                .all(|gate| gate.reason_code.as_deref() != Some("failed_code_review_recoverable")),
            "{name}: unexpected recovery gate: {pending_gates:?}"
        );
    }

    let fixture = failed_review_fixture(
        CodingAttemptScope::WorkItem,
        FixtureCase::WorkItemWithActiveUnitId,
    );
    let state = build_coding_session_state(&fixture.store, fixture.attempt)
        .expect("work item with active unit id session state");
    let CodingWsOutMessage::CodingSessionState { pending_gates, .. } = state else {
        panic!("expected coding session state");
    };
    assert!(
        pending_gates
            .iter()
            .all(|gate| gate.reason_code.as_deref() != Some("failed_code_review_recoverable"))
    );
}

#[tokio::test]
async fn failed_code_review_is_recovered_in_place_without_changing_execution_fingerprints() {
    let fixture =
        failed_review_fixture(CodingAttemptScope::WorkItemGroup, FixtureCase::Recoverable);
    let dirty_gate = fixture.dirty_gate.clone().expect("dirty gate");
    let attempt_before = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("attempt before recovery");
    let units_before = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units before recovery");
    let worktree_status_before = git_status_porcelain(
        fixture
            .attempt
            .worktree_path
            .as_deref()
            .expect("worktree path"),
    );
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);

    let updated = engine
        .recover_failed_code_review(&dirty_gate.gate_id)
        .await
        .expect("recover failed review");

    assert_eq!(updated.id, attempt_before.id);
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    assert_eq!(updated.completed_at, None);
    assert_eq!(
        attempt_data_fingerprint(&updated),
        attempt_data_fingerprint(&attempt_before)
    );
    assert_eq!(updated.active_unit_id, attempt_before.active_unit_id);
    assert_eq!(
        fixture
            .store
            .list_coding_units(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("units after recovery"),
        units_before
    );
    assert_eq!(
        git_status_porcelain(
            fixture
                .attempt
                .worktree_path
                .as_deref()
                .expect("worktree path")
        ),
        worktree_status_before
    );
    assert!(
        fixture
            .store
            .list_open_blocked_gates(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("open gates after recovery")
            .iter()
            .all(|gate| gate.gate_id != dirty_gate.gate_id)
    );

    let runs = fixture
        .store
        .list_role_runs(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("role runs after recovery");
    assert_eq!(runs.len(), 2);
    let stale = runs
        .iter()
        .find(|run| run.id == fixture.stale_role_run_id)
        .expect("stale reviewer run");
    let retry = runs
        .iter()
        .find(|run| run.id != fixture.stale_role_run_id)
        .expect("retry reviewer run");
    assert_eq!(stale.status, CodingRoleRunStatus::Superseded);
    assert_eq!(
        stale.superseded_by_run_id.as_deref(),
        Some(retry.id.as_str())
    );
    assert_eq!(retry.status, CodingRoleRunStatus::Running);
    assert_eq!(retry.trigger, CodingRoleRunTrigger::RetryReview);
    assert_eq!(retry.node_id, None);
    assert_eq!(
        retry.supersedes_run_id.as_deref(),
        Some(fixture.stale_role_run_id.as_str())
    );
}

#[tokio::test]
async fn failed_code_review_recovery_rejects_stale_identity_without_writes() {
    for (name, case, gate_id) in [
        (
            "stale gate",
            FixtureCase::Recoverable,
            "coding_blocked_gate_9999",
        ),
        (
            "node and run mismatch",
            FixtureCase::RoleRunNodeMismatch,
            "coding_blocked_gate_0001",
        ),
    ] {
        let fixture = failed_review_fixture(CodingAttemptScope::WorkItemGroup, case);
        let runs_before = fixture
            .store
            .list_role_runs(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("role runs before rejected recovery");
        let (event_tx, _event_rx) = mpsc::channel(8);
        let engine =
            CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);

        let error = engine
            .recover_failed_code_review(gate_id)
            .await
            .expect_err(name);

        assert!(
            error
                .to_string()
                .contains("coding_failed_review_recovery_state_changed"),
            "{name}: {error}"
        );
        assert_eq!(
            fixture
                .store
                .list_role_runs(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                )
                .expect("role runs after rejected recovery"),
            runs_before,
            "{name}"
        );
        assert!(
            fixture
                .store
                .get_failed_code_review_recovery_journal(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                )
                .expect("journal after rejected recovery")
                .is_none(),
            "{name}"
        );
    }
}

#[tokio::test]
async fn failed_code_review_recovery_reloads_attempt_and_rejects_status_change() {
    let fixture =
        failed_review_fixture(CodingAttemptScope::WorkItemGroup, FixtureCase::Recoverable);
    let dirty_gate = fixture.dirty_gate.clone().expect("dirty gate");
    let mut changed = fixture.attempt.clone();
    changed.status = CodingAttemptStatus::Blocked;
    changed.completed_at = None;
    fixture
        .store
        .save_coding_attempt(&changed)
        .expect("persist changed attempt status");
    let runs_before = fixture
        .store
        .list_role_runs(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("role runs before status change rejection");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);

    let error = engine
        .recover_failed_code_review(&dirty_gate.gate_id)
        .await
        .expect_err("status change must reject recovery");

    assert!(
        error
            .to_string()
            .contains("coding_failed_review_recovery_state_changed"),
        "{error}"
    );
    assert_eq!(
        fixture
            .store
            .list_role_runs(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("role runs after status change rejection"),
        runs_before
    );
    assert!(
        fixture
            .store
            .get_failed_code_review_recovery_journal(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("journal after status change rejection")
            .is_none()
    );
}

#[tokio::test]
async fn failed_code_review_recovery_journal_prefixes_converge_idempotently() {
    for prefix in [
        RecoveryPrefix::Prepared,
        RecoveryPrefix::AttemptReopened,
        RecoveryPrefix::RetryRunCreated,
        RecoveryPrefix::AttemptRunning,
        RecoveryPrefix::GateResolved,
    ] {
        let fixture =
            failed_review_fixture(CodingAttemptScope::WorkItemGroup, FixtureCase::Recoverable);
        let recovery = recoverable_failed_code_review(&fixture.store, &fixture.attempt)
            .expect("recoverable failed review")
            .expect("recovery identity");
        let journal = fixture
            .store
            .prepare_failed_code_review_recovery_journal(
                &fixture.attempt,
                &recovery.gate_id,
                &recovery.failed_node_id,
                &recovery.stale_role_run_id,
            )
            .expect("prepare recovery journal");

        if matches!(
            prefix,
            RecoveryPrefix::AttemptReopened
                | RecoveryPrefix::RetryRunCreated
                | RecoveryPrefix::AttemptRunning
                | RecoveryPrefix::GateResolved
        ) {
            fixture
                .store
                .reopen_failed_code_review_attempt(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                )
                .expect("persist reopened attempt prefix");
        }
        if matches!(
            prefix,
            RecoveryPrefix::RetryRunCreated
                | RecoveryPrefix::AttemptRunning
                | RecoveryPrefix::GateResolved
        ) {
            let reopened = fixture
                .store
                .get_attempt(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                )
                .expect("reopened attempt");
            fixture
                .store
                .ensure_failed_code_review_retry_role_run(&reopened, &journal)
                .expect("persist retry role run prefix");
        }
        if matches!(
            prefix,
            RecoveryPrefix::AttemptRunning | RecoveryPrefix::GateResolved
        ) {
            fixture
                .store
                .update_attempt_status(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                    CodingAttemptStatus::Running,
                )
                .expect("persist running attempt prefix");
        }
        if matches!(prefix, RecoveryPrefix::GateResolved) {
            fixture
                .store
                .resolve_blocked_gate(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                    &recovery.gate_id,
                )
                .expect("persist resolved gate prefix");
        }

        let (event_tx, _event_rx) = mpsc::channel(8);
        let engine =
            CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
        let first = engine
            .recover_failed_code_review(&recovery.gate_id)
            .await
            .unwrap_or_else(|error| panic!("{prefix:?}: first recovery: {error}"));
        let second = engine
            .recover_failed_code_review(&recovery.gate_id)
            .await
            .unwrap_or_else(|error| panic!("{prefix:?}: second recovery: {error}"));

        assert_eq!(first.id, fixture.attempt.id, "{prefix:?}");
        assert_eq!(second.id, fixture.attempt.id, "{prefix:?}");
        assert_eq!(second.status, CodingAttemptStatus::Running, "{prefix:?}");
        assert_eq!(second.stage, CodingExecutionStage::CodeReview, "{prefix:?}");
        assert_eq!(second.completed_at, None, "{prefix:?}");
        let runs = fixture
            .store
            .list_role_runs(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("role runs after prefix convergence");
        assert_eq!(runs.len(), 2, "{prefix:?}: {runs:?}");
        let retry = runs
            .iter()
            .find(|run| run.trigger == CodingRoleRunTrigger::RetryReview)
            .expect("stable retry role run");
        assert_eq!(
            retry.supersedes_run_id.as_deref(),
            Some(recovery.stale_role_run_id.as_str()),
            "{prefix:?}"
        );
        let converged = fixture
            .store
            .get_failed_code_review_recovery_journal(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("converged journal")
            .expect("journal remains until runner activation");
        assert_eq!(
            converged.phase,
            FailedCodeReviewRecoveryPhase::GateResolved,
            "{prefix:?}"
        );
        assert_eq!(
            converged.retry_role_run_id.as_deref(),
            Some(retry.id.as_str()),
            "{prefix:?}"
        );
        let state = build_coding_session_state(&fixture.store, second)
            .expect("session state while journal incomplete");
        let CodingWsOutMessage::CodingSessionState { pending_gates, .. } = state else {
            panic!("expected coding session state");
        };
        assert!(
            pending_gates
                .iter()
                .any(|gate| gate.gate_id == recovery.gate_id),
            "{prefix:?}: incomplete journal must keep recovery gate"
        );
    }
}

#[tokio::test]
async fn failed_review_recovery_journal_records_activation_before_retry_node_exists() {
    let fixture =
        failed_review_fixture(CodingAttemptScope::WorkItemGroup, FixtureCase::Recoverable);
    let gate_id = fixture
        .dirty_gate
        .as_ref()
        .expect("dirty gate")
        .gate_id
        .clone();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let updated = engine
        .recover_failed_code_review(&gate_id)
        .await
        .expect("recover failed review");
    let registry = CodingRunRegistry::default();
    let attempt_key = CodingAttemptRunKey::from_attempt(&updated);
    let reservation = registry
        .try_reserve_attempt(&attempt_key)
        .expect("reserve recovered attempt");
    let before_activation = fixture
        .store
        .get_failed_code_review_recovery_journal(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
        )
        .expect("journal before activation")
        .expect("incomplete recovery journal");
    assert_eq!(
        before_activation.phase,
        FailedCodeReviewRecoveryPhase::GateResolved
    );
    assert!(before_activation.runner_started_at.is_none());
    assert!(before_activation.completed_at.is_none());

    let (command_tx, _command_rx) = mpsc::channel(1);
    let run_id = reservation
        .activate(command_tx)
        .expect("activate reserved runner");
    fixture
        .store
        .complete_failed_code_review_recovery_journal(&updated, &gate_id)
        .expect("complete recovery journal after activation");

    let completed = fixture
        .store
        .get_failed_code_review_recovery_journal(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
        )
        .expect("completed journal")
        .expect("recovery journal");
    assert_eq!(completed.phase, FailedCodeReviewRecoveryPhase::Completed);
    assert!(completed.runner_started_at.is_some());
    assert!(completed.completed_at.is_some());
    let state = build_coding_session_state(&fixture.store, updated)
        .expect("session state after journal completion");
    let CodingWsOutMessage::CodingSessionState { pending_gates, .. } = state else {
        panic!("expected coding session state");
    };
    assert!(pending_gates.iter().any(|gate| gate.gate_id == gate_id));
    registry.remove(&attempt_key, run_id);
}

#[tokio::test]
async fn completed_journal_keeps_recovery_gate_until_retry_run_binds_a_node() {
    let fixture =
        failed_review_fixture(CodingAttemptScope::WorkItemGroup, FixtureCase::Recoverable);
    let gate_id = fixture
        .dirty_gate
        .as_ref()
        .expect("dirty gate")
        .gate_id
        .clone();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let updated = engine
        .recover_failed_code_review(&gate_id)
        .await
        .expect("recover failed review");
    let completed = fixture
        .store
        .complete_failed_code_review_recovery_journal(&updated, &gate_id)
        .expect("complete recovery journal before runner node");
    let retry_role_run_id = completed
        .retry_role_run_id
        .as_deref()
        .expect("retry role run id")
        .to_string();

    let state_before_node = build_coding_session_state(&fixture.store, updated.clone())
        .expect("session state before retry node");
    let CodingWsOutMessage::CodingSessionState { pending_gates, .. } = state_before_node else {
        panic!("expected coding session state");
    };
    assert!(pending_gates.iter().any(|gate| gate.gate_id == gate_id));
    assert!(failed_code_review_recovery_request(
        &fixture.store,
        &updated,
        &CodingWsInMessage::GateResponse {
            gate_id: gate_id.clone(),
            action_id: "retry_review".to_string(),
            extra_context: None,
        },
    ));
    let restarted = engine
        .recover_failed_code_review(&gate_id)
        .await
        .expect("restart completed recovery before retry node");
    assert_eq!(restarted.id, updated.id);
    assert_eq!(
        fixture
            .store
            .list_role_runs(&updated.project_id, &updated.issue_id, &updated.id)
            .expect("role runs after restart")
            .len(),
        2
    );

    fixture
        .store
        .attach_role_run_node(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            &retry_role_run_id,
            "coding_node_0010".to_string(),
        )
        .expect("bind retry role run node");
    let state_after_node = build_coding_session_state(&fixture.store, updated.clone())
        .expect("session state after retry node");
    let CodingWsOutMessage::CodingSessionState { pending_gates, .. } = state_after_node else {
        panic!("expected coding session state");
    };
    assert!(pending_gates.iter().all(|gate| gate.gate_id != gate_id));
    assert!(!failed_code_review_recovery_request(
        &fixture.store,
        &updated,
        &CodingWsInMessage::GateResponse {
            gate_id,
            action_id: "retry_review".to_string(),
            extra_context: None,
        },
    ));
}

#[test]
fn failed_review_recovery_supersedes_the_journal_stale_run_not_the_latest_run() {
    let fixture =
        failed_review_fixture(CodingAttemptScope::WorkItemGroup, FixtureCase::Recoverable);
    let recovery = recoverable_failed_code_review(&fixture.store, &fixture.attempt)
        .expect("recoverable failed review")
        .expect("recovery identity");
    let journal = fixture
        .store
        .prepare_failed_code_review_recovery_journal(
            &fixture.attempt,
            &recovery.gate_id,
            &recovery.failed_node_id,
            &recovery.stale_role_run_id,
        )
        .expect("prepare recovery journal");
    let reopened = fixture
        .store
        .reopen_failed_code_review_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("reopen failed review attempt");
    let unrelated_latest = fixture
        .store
        .create_role_run(
            &reopened,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0008".to_string()),
        )
        .expect("newer unrelated reviewer run");

    let retry = fixture
        .store
        .ensure_failed_code_review_retry_role_run(&reopened, &journal)
        .expect("ensure stable retry reviewer run");

    let runs = fixture
        .store
        .list_role_runs(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("role runs after precise supersede");
    let stale = runs
        .iter()
        .find(|run| run.id == recovery.stale_role_run_id)
        .expect("journal stale run");
    let unrelated = runs
        .iter()
        .find(|run| run.id == unrelated_latest.id)
        .expect("unrelated latest run");
    assert_eq!(stale.status, CodingRoleRunStatus::Superseded);
    assert_eq!(
        stale.superseded_by_run_id.as_deref(),
        Some(retry.id.as_str())
    );
    assert_eq!(unrelated.status, CodingRoleRunStatus::Running);
    assert_eq!(retry.supersedes_run_id.as_deref(), Some(stale.id.as_str()));
}
