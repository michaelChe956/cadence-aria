use super::*;

#[derive(Debug, Clone, Copy)]
enum NonTransportReviewerFailure {
    Generic,
    InvalidStructuredOutput,
    Permission,
    Choice,
}

struct NonTransportReviewerFailureProvider {
    failure: NonTransportReviewerFailure,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for NonTransportReviewerFailureProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel(1);
        let failure = self.failure;
        tokio::spawn(async move {
            match failure {
                NonTransportReviewerFailure::Generic => {
                    let _ = event_tx
                        .send(ProviderEvent::Failed {
                            message: "reviewer rejected the invocation".to_string(),
                        })
                        .await;
                }
                NonTransportReviewerFailure::InvalidStructuredOutput => {
                    let _ = event_tx
                        .send(ProviderEvent::Completed(
                            crate::cross_cutting::streaming_provider::ProviderCompletion::from_output(
                                "not structured review output".to_string(),
                                input.structured_output_contract.as_ref(),
                                None,
                            ),
                        ))
                        .await;
                }
                NonTransportReviewerFailure::Permission => {
                    let _ = event_tx
                        .send(ProviderEvent::PermissionTimeout {
                            permission_id: "permission_1".to_string(),
                        })
                        .await;
                }
                NonTransportReviewerFailure::Choice => {
                    let _ = event_tx
                        .send(ProviderEvent::ChoiceRequest(
                            crate::cross_cutting::streaming_provider::ChoiceRequestData {
                                id: "reviewer_choice_0001".to_string(),
                                prompt: "Choose reviewer action".to_string(),
                                options: vec![
                                    crate::cross_cutting::streaming_provider::ChoiceOptionData {
                                        id: "continue".to_string(),
                                        label: "Continue".to_string(),
                                        description: None,
                                    },
                                ],
                                allow_multiple: false,
                                allow_free_text: false,
                                questions: Vec::new(),
                                source: crate::cross_cutting::streaming_provider::ChoiceRequestSource::ProviderChoice,
                            },
                        ))
                        .await;
                    let _ = event_tx
                        .send(ProviderEvent::TextDelta {
                            content: "must wait for reviewer choice".to_string(),
                        })
                        .await;
                }
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

async fn assert_non_transport_reviewer_failure_cannot_start_recovery(
    failure: NonTransportReviewerFailure,
) {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = NonTransportReviewerFailureProvider { failure };
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let _ = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await;

    let runs_before = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reviewer role runs");
    let terminal = runs_before.last().expect("terminal reviewer run");
    match failure {
        NonTransportReviewerFailure::Generic | NonTransportReviewerFailure::Permission => {
            assert_eq!(terminal.status, CodingRoleRunStatus::Failed);
            assert_eq!(
                terminal.reason_code.as_deref(),
                Some("code_review_provider_interrupted")
            );
            assert!(
                !terminal.raw_provider_output_refs.is_empty(),
                "modern provider failures retain raw invocation evidence"
            );
        }
        NonTransportReviewerFailure::InvalidStructuredOutput => {
            assert_eq!(terminal.status, CodingRoleRunStatus::Blocked);
            assert_eq!(terminal.reason_code.as_deref(), Some("code_review_blocked"));
        }
        NonTransportReviewerFailure::Choice => {
            assert_eq!(terminal.status, CodingRoleRunStatus::Blocked);
            assert_eq!(
                terminal.reason_code.as_deref(),
                Some("provider_choice_unresolved")
            );
        }
    }
    let recovery_id = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("blocked reviewer gates")
        .into_iter()
        .next()
        .map(|gate| gate.gate_id)
        .or_else(|| {
            store
                .list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .expect("reviewer choice gates")
                .into_iter()
                .next()
                .map(|gate| gate.choice_id)
        })
        .expect("terminal reviewer interaction id");

    let recovery = engine.recover_failed_code_review(&recovery_id).await;
    assert!(matches!(
        recovery,
        Err(CodingWorkspaceEngineError::ProviderStream(message))
            if message == "coding_failed_review_recovery_state_changed"
    ));
    assert!(
        store
            .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("role runs after rejected recovery")
            .iter()
            .all(|run| run.trigger != CodingRoleRunTrigger::ManualRetry)
    );
}

#[tokio::test]
async fn generic_reviewer_failure_cannot_start_failed_review_recovery() {
    assert_non_transport_reviewer_failure_cannot_start_recovery(
        NonTransportReviewerFailure::Generic,
    )
    .await;
}

#[tokio::test]
async fn invalid_structured_reviewer_output_cannot_start_failed_review_recovery() {
    assert_non_transport_reviewer_failure_cannot_start_recovery(
        NonTransportReviewerFailure::InvalidStructuredOutput,
    )
    .await;
}

#[tokio::test]
async fn reviewer_permission_timeout_cannot_start_failed_review_recovery() {
    assert_non_transport_reviewer_failure_cannot_start_recovery(
        NonTransportReviewerFailure::Permission,
    )
    .await;
}

#[tokio::test]
async fn unresolved_reviewer_choice_cannot_start_failed_review_recovery() {
    assert_non_transport_reviewer_failure_cannot_start_recovery(
        NonTransportReviewerFailure::Choice,
    )
    .await;
}

#[tokio::test]
async fn exhausted_reviewer_transport_cycle_can_start_a_linked_manual_cycle_with_fresh_budget() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let exhausted_provider = TransportFailuresThenSuccessProvider::new(usize::MAX, "unused");
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let first_cycle_error = engine
        .execute_code_review_with_commands(&attempt, &exhausted_provider, &mut command_rx)
        .await
        .expect_err("third transport failure opens reviewer gate");
    assert!(matches!(
        first_cycle_error,
        CodingWorkspaceEngineError::ProviderStream(ref message)
            if message == "connection reset by peer"
    ));
    let gate = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reviewer gates")
        .into_iter()
        .find(|gate| gate.reason_code.as_deref() == Some("code_review_provider_interrupted"))
        .expect("reviewer recovery gate");
    let exhausted_runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("first cycle role runs");
    assert_eq!(exhausted_runs.len(), 3);
    let exhausted_last = exhausted_runs.last().expect("third failed run");
    assert_eq!(exhausted_last.trigger, CodingRoleRunTrigger::AutomaticRetry);
    assert_eq!(
        exhausted_last
            .retry_metadata
            .as_ref()
            .map(|metadata| metadata.attempt_no),
        Some(3)
    );
    assert_eq!(
        exhausted_last.reason_code.as_deref(),
        Some("provider_connection_interrupted")
    );

    let running = engine
        .recover_failed_code_review(&gate.gate_id)
        .await
        .expect("manual retry starts a new cycle");
    assert_eq!(running.status, CodingAttemptStatus::Running);
    let manual_run = store
        .latest_role_run(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
        )
        .expect("manual role run")
        .expect("manual role run exists");
    assert_eq!(manual_run.trigger, CodingRoleRunTrigger::ManualRetry);
    let manual_metadata = manual_run.retry_metadata.as_ref().expect("manual metadata");
    assert_eq!(manual_metadata.attempt_no, 1);
    assert_eq!(
        manual_metadata.prior_run_id.as_deref(),
        Some(exhausted_last.id.as_str())
    );
    assert_ne!(
        manual_metadata.cycle_id,
        exhausted_last.retry_metadata.as_ref().unwrap().cycle_id
    );
    assert_eq!(
        store
            .get_role_run(
                &running.project_id,
                &running.issue_id,
                &running.id,
                &exhausted_last.id,
            )
            .expect("preserved exhausted run")
            .status,
        CodingRoleRunStatus::Failed
    );

    let recovery_provider = TransportFailuresThenSuccessProvider::new(
        2,
        r#"{"verdict":"approve","summary":"manual retry completed","findings":[]}"#,
    );
    let report = engine
        .execute_code_review_with_commands(&running, &recovery_provider, &mut command_rx)
        .await
        .expect("manual cycle succeeds after two automatic retries");
    assert_eq!(report.verdict, ReviewVerdict::Approve);
    let all_runs = store
        .list_role_runs(&running.project_id, &running.issue_id, &running.id)
        .expect("all reviewer role runs");
    assert_eq!(all_runs.len(), 6);
    let manual_cycle_runs = all_runs
        .iter()
        .filter(|run| {
            run.retry_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.cycle_id == manual_metadata.cycle_id)
        })
        .collect::<Vec<_>>();
    assert_eq!(manual_cycle_runs.len(), 3);
    assert_eq!(
        manual_cycle_runs[0].trigger,
        CodingRoleRunTrigger::ManualRetry
    );
    assert_eq!(manual_cycle_runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(
        manual_cycle_runs[1].trigger,
        CodingRoleRunTrigger::AutomaticRetry
    );
    assert_eq!(manual_cycle_runs[1].status, CodingRoleRunStatus::Failed);
    assert_eq!(
        manual_cycle_runs[2].trigger,
        CodingRoleRunTrigger::AutomaticRetry
    );
    assert_eq!(manual_cycle_runs[2].status, CodingRoleRunStatus::Completed);
    assert_eq!(
        manual_cycle_runs
            .iter()
            .map(|run| run.retry_metadata.as_ref().unwrap().attempt_no)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(
        manual_cycle_runs
            .iter()
            .all(|run| !run.raw_provider_output_refs.is_empty())
    );
    assert!(
        store
            .list_open_blocked_gates(&running.project_id, &running.issue_id, &running.id)
            .expect("gates after successful manual cycle")
            .is_empty()
    );
}

#[tokio::test]
async fn exhausted_reviewer_recovery_rejects_non_transport_terminal_reason() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = TransportFailuresThenSuccessProvider::new(usize::MAX, "unused");
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    let _ = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("transport failures open reviewer gate");
    let failed = store
        .latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
        )
        .expect("latest reviewer run")
        .expect("third reviewer run");
    store
        .update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &failed.id,
            CodingRoleRunStatus::Failed,
            Some("provider_structured_output".to_string()),
        )
        .expect("rewrite terminal reason");
    let gate = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reviewer gates")
        .into_iter()
        .find(|gate| gate.reason_code.as_deref() == Some("code_review_provider_interrupted"))
        .expect("reviewer gate");
    let recovery = engine.recover_failed_code_review(&gate.gate_id).await;
    assert!(matches!(
        recovery,
        Err(CodingWorkspaceEngineError::ProviderStream(message))
            if message == "coding_failed_review_recovery_state_changed"
    ));
}
