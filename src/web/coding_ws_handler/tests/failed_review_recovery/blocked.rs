use super::*;

#[tokio::test]
async fn blocked_provider_interrupted_review_retry_enters_the_same_recovery_journal() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interrupted gate");
    let recovery = recoverable_failed_code_review(&fixture.store, &fixture.attempt)
        .expect("inspect blocked recovery")
        .expect("blocked review must be recoverable");

    assert_eq!(recovery.gate_id, gate.gate_id);
    assert_eq!(recovery.failed_node_id, support::FAILED_NODE_ID);
    assert_eq!(recovery.stale_role_run_id, fixture.stale_role_run_id);

    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let updated = engine
        .recover_failed_code_review_for_attempt(&fixture.attempt, &gate.gate_id)
        .await
        .expect("recover blocked provider interruption");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    assert_eq!(updated.completed_at, None);
    let journal = fixture
        .store
        .get_failed_code_review_recovery_journal(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
        )
        .expect("load blocked recovery journal")
        .expect("blocked recovery journal");
    assert_eq!(journal.expected_gate_id, gate.gate_id);
    assert_eq!(journal.expected_failed_node_id, support::FAILED_NODE_ID);
    assert_eq!(
        journal.expected_stale_role_run_id,
        fixture.stale_role_run_id
    );
    assert_eq!(journal.phase, FailedCodeReviewRecoveryPhase::GateResolved);

    let runs = fixture
        .store
        .list_role_runs(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("role runs after blocked recovery");
    assert_eq!(runs.len(), 2);
    let stale = runs
        .iter()
        .find(|run| run.id == fixture.stale_role_run_id)
        .expect("failed reviewer run");
    assert_eq!(stale.status, CodingRoleRunStatus::Failed);
    let retry = runs
        .iter()
        .find(|run| run.trigger == CodingRoleRunTrigger::RetryReview)
        .expect("single retry reviewer run");
    assert_eq!(
        retry.supersedes_run_id.as_deref(),
        Some(fixture.stale_role_run_id.as_str())
    );
}

#[tokio::test]
async fn blocked_provider_interrupted_retry_cannot_use_the_ordinary_gate_path() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interrupted gate");
    let attempt_before = fixture.attempt.clone();
    let runs_before = fixture
        .store
        .list_role_runs(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("role runs before ordinary retry");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);

    let error = engine
        .handle_blocked_gate_response(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &gate.gate_id,
            "retry_review",
            None,
        )
        .await
        .expect_err("ordinary retry path must require a reservation");

    assert!(
        error
            .to_string()
            .contains("coding_failed_review_recovery_requires_reservation"),
        "{error}"
    );
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("attempt after ordinary retry rejection"),
        attempt_before
    );
    assert_eq!(
        fixture
            .store
            .list_role_runs(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("role runs after ordinary retry rejection"),
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
            .expect("journal after ordinary retry rejection")
            .is_none()
    );
}

#[tokio::test]
async fn blocked_provider_interrupted_recovery_prefixes_converge_idempotently() {
    for prefix in [
        RecoveryPrefix::Prepared,
        RecoveryPrefix::AttemptReopened,
        RecoveryPrefix::RetryRunCreated,
        RecoveryPrefix::AttemptRunning,
        RecoveryPrefix::GateResolved,
    ] {
        let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
        let recovery = recoverable_failed_code_review(&fixture.store, &fixture.attempt)
            .expect("inspect blocked recovery")
            .expect("blocked recovery identity");
        let mut journal = fixture
            .store
            .prepare_failed_code_review_recovery_journal(
                &fixture.attempt,
                &recovery.gate_id,
                &recovery.failed_node_id,
                &recovery.stale_role_run_id,
            )
            .expect("prepare blocked recovery journal");

        if matches!(
            prefix,
            RecoveryPrefix::AttemptReopened
                | RecoveryPrefix::RetryRunCreated
                | RecoveryPrefix::AttemptRunning
                | RecoveryPrefix::GateResolved
        ) {
            journal = fixture
                .store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::AttemptReopened,
                    None,
                )
                .expect("persist blocked attempt-reopened prefix");
        }
        if matches!(
            prefix,
            RecoveryPrefix::RetryRunCreated
                | RecoveryPrefix::AttemptRunning
                | RecoveryPrefix::GateResolved
        ) {
            let retry = fixture
                .store
                .ensure_failed_code_review_retry_role_run(&fixture.attempt, &journal)
                .expect("persist blocked retry run prefix");
            journal = fixture
                .store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::RetryRunCreated,
                    Some(&retry.id),
                )
                .expect("record blocked retry run prefix");
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
                .expect("persist blocked attempt-running prefix");
            journal = fixture
                .store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::AttemptRunning,
                    journal.retry_role_run_id.as_deref(),
                )
                .expect("record blocked attempt-running prefix");
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
                .expect("persist blocked gate-resolved prefix");
            let retry_role_run_id = journal.retry_role_run_id.clone();
            fixture
                .store
                .advance_failed_code_review_recovery_journal(
                    &journal,
                    FailedCodeReviewRecoveryPhase::GateResolved,
                    retry_role_run_id.as_deref(),
                )
                .expect("record blocked gate-resolved prefix");
        }

        let prefixed_attempt = fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("attempt at blocked recovery prefix");
        assert_eq!(
            crate::web::coding_ws_handler::socket::unfinished_failed_code_review_recovery_message_allowed(
                &fixture.store,
                &prefixed_attempt,
                &CodingWsInMessage::GateResponse {
                    gate_id: recovery.gate_id.clone(),
                    action_id: "retry_review".to_string(),
                    extra_context: None,
                },
            ),
            Some(true),
            "{prefix:?}: exact retry must remain allowed"
        );
        assert_eq!(
            crate::web::coding_ws_handler::socket::unfinished_failed_code_review_recovery_message_allowed(
                &fixture.store,
                &prefixed_attempt,
                &CodingWsInMessage::AbortAttempt,
            ),
            Some(false),
            "{prefix:?}: abort must not bypass unfinished recovery"
        );

        let (event_tx, _event_rx) = mpsc::channel(8);
        let engine =
            CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
        let first = engine
            .recover_failed_code_review_for_attempt(&fixture.attempt, &recovery.gate_id)
            .await
            .unwrap_or_else(|error| panic!("{prefix:?}: first recovery: {error}"));
        let second = engine
            .recover_failed_code_review_for_attempt(&fixture.attempt, &recovery.gate_id)
            .await
            .unwrap_or_else(|error| panic!("{prefix:?}: second recovery: {error}"));

        assert_eq!(first.id, fixture.attempt.id, "{prefix:?}");
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
            .expect("role runs after blocked prefix convergence");
        assert_eq!(runs.len(), 2, "{prefix:?}: {runs:?}");
        assert_eq!(
            runs.iter()
                .filter(|run| run.trigger == CodingRoleRunTrigger::RetryReview)
                .count(),
            1,
            "{prefix:?}"
        );
        let stale = runs
            .iter()
            .find(|run| run.id == fixture.stale_role_run_id)
            .expect("failed reviewer run after convergence");
        assert_eq!(stale.status, CodingRoleRunStatus::Failed, "{prefix:?}");
    }
}
