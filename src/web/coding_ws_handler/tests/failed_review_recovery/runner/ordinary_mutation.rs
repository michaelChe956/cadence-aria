use std::fs;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::product::coding_attempt_store::FailedCodeReviewRecoveryPhase;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage, CodingRoleRunTrigger,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::web::coding_ws_handler::socket::{CodingMessagePreparation, prepare_coding_message};
use crate::web::coding_ws_handler::{
    CodingRunnerStartProbe, CodingWsInMessage, context_note_chat_entry,
    spawn_coding_runner_reserved_with_probe,
};
use crate::web::runtime::WebRuntime;
use crate::web::state::{CodingAttemptRunKey, WebAppState};

use super::super::support::{FixtureCase, failed_review_fixture};
use super::wait_for_runner_count;

#[derive(Debug, Clone, Copy)]
enum OrdinaryMutationCase {
    AbortAttempt,
    ContextNote,
}

#[tokio::test]
async fn ordinary_allowed_mutation_finishes_before_retry_reloads_state() {
    for case in [
        OrdinaryMutationCase::AbortAttempt,
        OrdinaryMutationCase::ContextNote,
    ] {
        let fixture = failed_review_fixture(
            CodingAttemptScope::WorkItem,
            FixtureCase::BlockedProviderInterrupted,
        );
        if matches!(case, OrdinaryMutationCase::AbortAttempt) {
            fs::remove_file(
                fixture
                    .attempt
                    .worktree_path
                    .as_ref()
                    .expect("fixture worktree")
                    .join("dirty-review.txt"),
            )
            .expect("clean abort fixture worktree");
        }
        let state = WebAppState::new(
            fixture._tmp.path().to_path_buf(),
            WebRuntime::new_fake(fixture._tmp.path().to_path_buf()),
        );
        let attempt_id = fixture.attempt.id.clone();
        let attempt_key = CodingAttemptRunKey::from_attempt(&fixture.attempt);
        let gate_id = fixture
            .dirty_gate
            .as_ref()
            .expect("provider interrupted gate")
            .gate_id
            .clone();
        let ordinary_message = match case {
            OrdinaryMutationCase::AbortAttempt => CodingWsInMessage::AbortAttempt,
            OrdinaryMutationCase::ContextNote => CodingWsInMessage::ContextNote {
                content: "ordinary mutation wins".to_string(),
            },
        };
        let (ordinary_event_tx, _ordinary_event_rx) = mpsc::channel(8);
        let ordinary_preparation = prepare_coding_message(
            &fixture.store,
            &state.coding_runs,
            &ordinary_event_tx,
            ("project_0001", "issue_0001", &attempt_id),
            &ordinary_message,
        )
        .await
        .expect("ordinary production preparation");
        let CodingMessagePreparation::Allowed {
            attempt: current_attempt,
            mutation_lease,
        } = ordinary_preparation
        else {
            panic!("{case:?}: expected ordinary message to be allowed");
        };

        let recovery_store = fixture.store.clone();
        let recovery_registry = state.coding_runs.clone();
        let recovery_attempt_id = attempt_id.clone();
        let retry_message = CodingWsInMessage::GateResponse {
            gate_id: gate_id.clone(),
            action_id: "retry_review".to_string(),
            extra_context: None,
        };
        let (recovery_started_tx, recovery_started_rx) = oneshot::channel();
        let mut recovery = tokio::spawn(async move {
            let (event_tx, _event_rx) = mpsc::channel(8);
            recovery_started_tx
                .send(())
                .expect("signal retry preparation start");
            prepare_coding_message(
                &recovery_store,
                &recovery_registry,
                &event_tx,
                ("project_0001", "issue_0001", &recovery_attempt_id),
                &retry_message,
            )
            .await
            .expect("retry production preparation")
        });
        recovery_started_rx
            .await
            .expect("retry preparation task started");

        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut recovery)
                .await
                .is_err(),
            "{case:?}: retry crossed Allowed→mutation window"
        );

        match case {
            OrdinaryMutationCase::AbortAttempt => {
                let engine = CodingWorkspaceEngine::new(
                    fixture.store.clone(),
                    GitWorkspaceService::new(),
                    ordinary_event_tx.clone(),
                );
                engine
                    .handle_abort(
                        &current_attempt.project_id,
                        &current_attempt.issue_id,
                        &current_attempt.id,
                    )
                    .await
                    .expect("ordinary abort mutation");
            }
            OrdinaryMutationCase::ContextNote => {
                let note = fixture
                    .store
                    .create_context_note(&current_attempt, "ordinary mutation wins".to_string())
                    .expect("ordinary context note mutation");
                let entry = context_note_chat_entry(&fixture.store, &current_attempt, note)
                    .expect("ordinary context note chat entry");
                fixture
                    .store
                    .save_chat_entry(&current_attempt, &entry)
                    .expect("persist ordinary context note chat entry");
            }
        }
        drop(mutation_lease);

        let recovery_preparation = recovery.await.expect("retry preparation task");
        match case {
            OrdinaryMutationCase::AbortAttempt => {
                assert!(matches!(
                    recovery_preparation,
                    CodingMessagePreparation::Rejected
                ));
                assert!(
                    !state
                        .coding_runs
                        .has_active_recovery_reservation(&attempt_key)
                );
                assert!(
                    fixture
                        .store
                        .get_failed_code_review_recovery_journal(
                            &current_attempt.project_id,
                            &current_attempt.issue_id,
                            &current_attempt.id,
                        )
                        .expect("abort recovery journal")
                        .is_none()
                );
                let persisted = fixture
                    .store
                    .get_attempt_by_id(&attempt_id)
                    .expect("aborted attempt");
                assert_eq!(persisted.status, CodingAttemptStatus::Aborted);
                assert_eq!(persisted.stage, CodingExecutionStage::CodeReview);
                assert_eq!(state.coding_runs.runner_count(&attempt_key), 0);
                assert_eq!(
                    fixture
                        .store
                        .list_role_runs(&persisted.project_id, &persisted.issue_id, &persisted.id,)
                        .expect("abort role runs")
                        .len(),
                    1
                );
            }
            OrdinaryMutationCase::ContextNote => {
                let CodingMessagePreparation::FailedReviewRecovery {
                    attempt: updated,
                    gate_id,
                    reservation,
                } = recovery_preparation
                else {
                    panic!("context note must linearize before retry recovery");
                };
                let events = Arc::new(std::sync::Mutex::new(Vec::new()));
                let (provider_entry_tx, provider_entry_rx) = oneshot::channel();
                let (runner_continue_tx, runner_continue_rx) = oneshot::channel();
                spawn_coding_runner_reserved_with_probe(
                    state.clone(),
                    fixture.store.clone(),
                    ordinary_event_tx,
                    updated.clone(),
                    &gate_id,
                    reservation,
                    CodingRunnerStartProbe {
                        events,
                        provider_entry_tx,
                        continue_rx: runner_continue_rx,
                    },
                )
                .expect("spawn retry after context note mutation");
                provider_entry_rx.await.expect("retry provider entry");
                assert!(
                    !state
                        .coding_runs
                        .has_active_recovery_reservation(&attempt_key)
                );
                assert_eq!(state.coding_runs.runner_count(&attempt_key), 1);
                let persisted = fixture
                    .store
                    .get_attempt_by_id(&attempt_id)
                    .expect("running attempt after context note recovery");
                assert_eq!(persisted.status, CodingAttemptStatus::Running);
                assert_eq!(persisted.stage, CodingExecutionStage::CodeReview);
                assert_eq!(
                    fixture
                        .store
                        .get_failed_code_review_recovery_journal(
                            &updated.project_id,
                            &updated.issue_id,
                            &updated.id,
                        )
                        .expect("context note recovery journal")
                        .expect("completed context note recovery journal")
                        .phase,
                    FailedCodeReviewRecoveryPhase::Completed
                );
                assert_eq!(
                    fixture
                        .store
                        .list_context_notes(&updated.project_id, &updated.issue_id, &updated.id)
                        .expect("context notes after retry")
                        .len(),
                    1
                );
                assert_eq!(
                    fixture
                        .store
                        .list_chat_entries(&updated.project_id, &updated.issue_id, &updated.id)
                        .expect("context note chat entries after retry")
                        .len(),
                    1
                );
                assert_eq!(
                    fixture
                        .store
                        .list_role_runs(&updated.project_id, &updated.issue_id, &updated.id)
                        .expect("context note recovery role runs")
                        .iter()
                        .filter(|run| run.trigger == CodingRoleRunTrigger::RetryReview)
                        .count(),
                    1
                );
                drop(runner_continue_tx);
                wait_for_runner_count(&state.coding_runs, &attempt_key, 0).await;
            }
        }
    }
}
