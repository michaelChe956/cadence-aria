use std::sync::{Arc, Barrier};
use std::thread;

use tokio::sync::{mpsc, oneshot};

use crate::product::coding_attempt_store::FailedCodeReviewRecoveryPhase;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::web::coding_ws_handler::socket::{
    CodingMessagePreparation, CodingRecoveryPreparationProbe, failed_code_review_recovery_request,
    prepare_coding_message, prepare_coding_message_with_probe,
    unfinished_failed_code_review_recovery_message_allowed,
};
use crate::web::coding_ws_handler::{
    CodingRunnerStartProbe, CodingWsInMessage, is_coding_ws_message_allowed,
    spawn_coding_runner_reserved_with_probe,
};
use crate::web::runtime::WebRuntime;
use crate::web::state::{CodingAttemptRunKey, CodingRunRegistry, WebAppState};

use super::support::{provider_interrupted_review_fixture, seed_repeated_interrupted_review};

mod ordinary_mutation;

#[tokio::test]
async fn reserved_spawn_creates_task_then_completes_journal_before_provider_entry() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate_id = fixture
        .dirty_gate
        .as_ref()
        .expect("dirty gate")
        .gate_id
        .clone();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        event_tx.clone(),
    );
    let updated = engine
        .recover_failed_code_review(&gate_id)
        .await
        .expect("recover failed review");
    let state = WebAppState::new(
        fixture._tmp.path().to_path_buf(),
        WebRuntime::new_fake(fixture._tmp.path().to_path_buf()),
    );
    let attempt_key = CodingAttemptRunKey::from_attempt(&updated);
    let reservation = state
        .coding_runs
        .try_reserve_attempt(&attempt_key)
        .expect("reserve recovered attempt");
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (provider_entry_tx, provider_entry_rx) = oneshot::channel();
    let (continue_tx, continue_rx) = oneshot::channel();

    spawn_coding_runner_reserved_with_probe(
        state.clone(),
        fixture.store.clone(),
        event_tx,
        updated.clone(),
        &gate_id,
        reservation,
        CodingRunnerStartProbe {
            events: Arc::clone(&events),
            provider_entry_tx,
            continue_rx,
        },
    )
    .expect("spawn reserved runner with probe");
    provider_entry_rx.await.expect("provider entry probe");

    assert_eq!(
        *events.lock().expect("runner start events"),
        vec!["task_created", "journal_completed", "provider_entry"]
    );
    let journal = fixture
        .store
        .get_failed_code_review_recovery_journal(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
        )
        .expect("journal after reserved spawn")
        .expect("recovery journal");
    assert_eq!(journal.phase, FailedCodeReviewRecoveryPhase::Completed);
    drop(continue_tx);
    wait_for_runner_count(&state.coding_runs, &attempt_key, 0).await;
}

#[tokio::test]
async fn journal_completion_failure_stops_task_before_provider_entry_and_cleans_registry() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate_id = fixture
        .dirty_gate
        .as_ref()
        .expect("dirty gate")
        .gate_id
        .clone();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        event_tx.clone(),
    );
    let updated = engine
        .recover_failed_code_review(&gate_id)
        .await
        .expect("recover failed review");
    let state = WebAppState::new(
        fixture._tmp.path().to_path_buf(),
        WebRuntime::new_fake(fixture._tmp.path().to_path_buf()),
    );
    let attempt_key = CodingAttemptRunKey::from_attempt(&updated);
    let reservation = state
        .coding_runs
        .try_reserve_attempt(&attempt_key)
        .expect("reserve recovered attempt");
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (provider_entry_tx, provider_entry_rx) = oneshot::channel();
    let (_continue_tx, continue_rx) = oneshot::channel();

    let error = spawn_coding_runner_reserved_with_probe(
        state.clone(),
        fixture.store.clone(),
        event_tx,
        updated.clone(),
        "coding_blocked_gate_9999",
        reservation,
        CodingRunnerStartProbe {
            events: Arc::clone(&events),
            provider_entry_tx,
            continue_rx,
        },
    )
    .expect_err("journal completion must fail for stale gate");

    assert!(
        error
            .to_string()
            .contains("coding_failed_review_recovery_state_changed"),
        "{error}"
    );
    assert!(provider_entry_rx.await.is_err());
    wait_for_runner_count(&state.coding_runs, &attempt_key, 0).await;
    assert_eq!(
        *events.lock().expect("runner start events"),
        vec!["task_created"]
    );
}

#[tokio::test]
async fn failed_review_websocket_guard_allows_only_the_exact_retry_gate_request() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let dirty_gate = fixture.dirty_gate.as_ref().expect("dirty gate");
    let retry = CodingWsInMessage::GateResponse {
        gate_id: dirty_gate.gate_id.clone(),
        action_id: "retry_review".to_string(),
        extra_context: None,
    };

    assert!(failed_code_review_recovery_request(
        &fixture.store,
        &fixture.attempt,
        &retry,
    ));
    assert!(!failed_code_review_recovery_request(
        &fixture.store,
        &fixture.attempt,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_9999".to_string(),
            action_id: "retry_review".to_string(),
            extra_context: None,
        },
    ));
    assert!(!failed_code_review_recovery_request(
        &fixture.store,
        &fixture.attempt,
        &CodingWsInMessage::GateResponse {
            gate_id: dirty_gate.gate_id.clone(),
            action_id: "manual_continue".to_string(),
            extra_context: None,
        },
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::Failed,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::ContextNote {
            content: "continue".to_string(),
        },
    ));
}

#[tokio::test]
async fn unfinished_blocked_review_journal_allows_only_its_exact_retry_message() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interrupted gate");
    let recovery = crate::product::coding_workspace_engine::recoverable_failed_code_review(
        &fixture.store,
        &fixture.attempt,
    )
    .expect("inspect recovery")
    .expect("blocked recovery identity");
    fixture
        .store
        .prepare_failed_code_review_recovery_journal(
            &fixture.attempt,
            &recovery.gate_id,
            &recovery.failed_node_id,
            &recovery.stale_role_run_id,
        )
        .expect("prepare blocked recovery journal");

    let allowed = CodingWsInMessage::GateResponse {
        gate_id: gate.gate_id.clone(),
        action_id: "retry_review".to_string(),
        extra_context: None,
    };
    assert_eq!(
        unfinished_failed_code_review_recovery_message_allowed(
            &fixture.store,
            &fixture.attempt,
            &allowed,
        ),
        Some(true)
    );

    for rejected in [
        CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_9999".to_string(),
            action_id: "retry_review".to_string(),
            extra_context: None,
        },
        CodingWsInMessage::GateResponse {
            gate_id: gate.gate_id.clone(),
            action_id: "manual_continue".to_string(),
            extra_context: None,
        },
        CodingWsInMessage::GateResponse {
            gate_id: gate.gate_id.clone(),
            action_id: "send_to_coder".to_string(),
            extra_context: None,
        },
        CodingWsInMessage::GateResponse {
            gate_id: gate.gate_id.clone(),
            action_id: "abort".to_string(),
            extra_context: None,
        },
        CodingWsInMessage::ContextNote {
            content: "continue".to_string(),
        },
        CodingWsInMessage::AbortAttempt,
    ] {
        assert_eq!(
            unfinished_failed_code_review_recovery_message_allowed(
                &fixture.store,
                &fixture.attempt,
                &rejected,
            ),
            Some(false),
            "unfinished recovery must reject {rejected:?}"
        );
    }
}

#[tokio::test]
async fn blocked_review_retry_reservation_rejects_an_old_runner_before_persistence() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interrupted gate");
    let retry = CodingWsInMessage::GateResponse {
        gate_id: gate.gate_id.clone(),
        action_id: "retry_review".to_string(),
        extra_context: None,
    };
    assert!(failed_code_review_recovery_request(
        &fixture.store,
        &fixture.attempt,
        &retry,
    ));

    let registry = CodingRunRegistry::default();
    let attempt_key = CodingAttemptRunKey::from_attempt(&fixture.attempt);
    let (old_runner_tx, _old_runner_rx) = mpsc::channel(1);
    let old_runner_id = registry
        .insert_cancellable(&attempt_key, old_runner_tx)
        .expect("register old runner")
        .run_id;
    assert!(registry.try_reserve_attempt(&attempt_key).is_none());
    assert!(
        fixture
            .store
            .get_failed_code_review_recovery_journal(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("journal before reservation")
            .is_none()
    );
    assert_eq!(
        fixture
            .store
            .list_role_runs(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("role runs before reservation")
            .len(),
        1
    );
    registry.remove(&attempt_key, old_runner_id);
}

#[tokio::test]
async fn two_blocked_review_retry_sockets_converge_to_one_retry_run_and_runner() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate_id = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interrupted gate")
        .gate_id
        .clone();
    let registry = Arc::new(CodingRunRegistry::default());
    let barrier = Arc::new(Barrier::new(3));
    let attempts = (0..2)
        .map(|_| {
            let registry = Arc::clone(&registry);
            let barrier = Arc::clone(&barrier);
            let attempt_key = CodingAttemptRunKey::from_attempt(&fixture.attempt);
            thread::spawn(move || {
                barrier.wait();
                registry.try_reserve_attempt(&attempt_key)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let mut reservations = attempts
        .into_iter()
        .filter_map(|attempt| attempt.join().expect("socket reservation"))
        .collect::<Vec<_>>();
    assert_eq!(reservations.len(), 1);

    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let updated = engine
        .recover_failed_code_review_for_attempt(&fixture.attempt, &gate_id)
        .await
        .expect("winning socket recovers review");
    let (command_tx, _command_rx) = mpsc::channel(1);
    let run_id = reservations
        .pop()
        .expect("winning reservation")
        .activate_cancellable(command_tx)
        .expect("activate winning runner")
        .run_id;
    fixture
        .store
        .complete_failed_code_review_recovery_journal(&updated, &gate_id)
        .expect("complete winning recovery journal");

    let attempt_key = CodingAttemptRunKey::from_attempt(&updated);
    assert_eq!(registry.runner_count(&attempt_key), 1);
    let runs = fixture
        .store
        .list_role_runs(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("role runs after concurrent retries");
    assert_eq!(
        runs.iter()
            .filter(|run| run.trigger
                == crate::product::coding_models::CodingRoleRunTrigger::RetryReview)
            .count(),
        1
    );
    registry.remove(&attempt_key, run_id);
}

#[tokio::test]
async fn two_repeated_review_retry_sockets_converge_to_one_current_run_and_runner() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let repeated = seed_repeated_interrupted_review(&fixture).await;
    let gate_id = repeated.second_gate.gate_id.clone();
    let retry = CodingWsInMessage::GateResponse {
        gate_id: gate_id.clone(),
        action_id: "retry_review".to_string(),
        extra_context: None,
    };
    assert!(failed_code_review_recovery_request(
        &fixture.store,
        &repeated.blocked_attempt,
        &retry,
    ));

    let registry = Arc::new(CodingRunRegistry::default());
    let barrier = Arc::new(Barrier::new(3));
    let attempts = (0..2)
        .map(|_| {
            let registry = Arc::clone(&registry);
            let barrier = Arc::clone(&barrier);
            let attempt_key = CodingAttemptRunKey::from_attempt(&repeated.blocked_attempt);
            thread::spawn(move || {
                barrier.wait();
                registry.try_reserve_attempt(&attempt_key)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let mut reservations = attempts
        .into_iter()
        .filter_map(|attempt| attempt.join().expect("socket reservation"))
        .collect::<Vec<_>>();
    assert_eq!(reservations.len(), 1);

    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let updated = engine
        .recover_failed_code_review_for_attempt(&repeated.blocked_attempt, &gate_id)
        .await
        .expect("winning socket recovers second interrupted review");
    let (command_tx, _command_rx) = mpsc::channel(1);
    let run_id = reservations
        .pop()
        .expect("winning reservation")
        .activate_cancellable(command_tx)
        .expect("activate winning runner")
        .run_id;
    let current = fixture
        .store
        .complete_failed_code_review_recovery_journal(&updated, &gate_id)
        .expect("complete second recovery journal");

    let attempt_key = CodingAttemptRunKey::from_attempt(&updated);
    assert_eq!(registry.runner_count(&attempt_key), 1);
    assert_eq!(current.expected_gate_id, gate_id);
    assert_eq!(
        fixture
            .store
            .get_archived_failed_code_review_recovery_journal(
                &updated.project_id,
                &updated.issue_id,
                &updated.id,
                &repeated.first_journal.expected_gate_id,
            )
            .expect("archived first journal")
            .expect("first recovery history"),
        repeated.first_journal
    );
    let runs = fixture
        .store
        .list_role_runs(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("role runs after concurrent second retry");
    assert_eq!(
        runs.iter()
            .filter(|run| run.trigger
                == crate::product::coding_models::CodingRoleRunTrigger::RetryReview)
            .count(),
        2
    );
    assert_eq!(
        runs.iter()
            .filter(|run| run.reason_code.as_deref() == Some(current.recovery_key.as_str()))
            .count(),
        1
    );
    registry.remove(&attempt_key, run_id);
}

#[tokio::test]
async fn production_recovery_lifecycle_rejects_competing_abort_and_context_note() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let attempt_id = fixture.attempt.id.clone();
    let attempt_key = CodingAttemptRunKey::from_attempt(&fixture.attempt);
    let gate_id = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interrupted gate")
        .gate_id
        .clone();
    let state = WebAppState::new(
        fixture._tmp.path().to_path_buf(),
        WebRuntime::new_fake(fixture._tmp.path().to_path_buf()),
    );
    let registry = state.coding_runs.clone();
    let (reserved_tx, reserved_rx) = oneshot::channel();
    let (continue_tx, continue_rx) = oneshot::channel();
    let (prepared_tx, prepared_rx) = oneshot::channel();
    let (spawn_tx, spawn_rx) = oneshot::channel();
    let winner_state = state.clone();
    let winner_registry = registry.clone();
    let winner_store = fixture.store.clone();
    let winner_attempt_id = attempt_id.clone();
    let winner_gate_id = gate_id.clone();
    let winner = tokio::spawn(async move {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let inbound = CodingWsInMessage::GateResponse {
            gate_id: winner_gate_id.clone(),
            action_id: "retry_review".to_string(),
            extra_context: None,
        };
        let preparation = prepare_coding_message_with_probe(
            &winner_store,
            &winner_registry,
            &event_tx,
            ("project_0001", "issue_0001", &winner_attempt_id),
            &inbound,
            CodingRecoveryPreparationProbe {
                reserved_tx,
                continue_rx,
            },
        )
        .await
        .expect("winning recovery preparation");
        prepared_tx
            .send(())
            .expect("signal released recovery guard");
        spawn_rx.await.expect("continue reserved spawn");
        let CodingMessagePreparation::FailedReviewRecovery {
            attempt: updated,
            gate_id,
            reservation,
        } = preparation
        else {
            panic!("expected failed review recovery preparation");
        };
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (provider_entry_tx, provider_entry_rx) = oneshot::channel();
        let (runner_continue_tx, runner_continue_rx) = oneshot::channel();
        spawn_coding_runner_reserved_with_probe(
            winner_state,
            winner_store,
            event_tx,
            updated.clone(),
            &gate_id,
            reservation,
            CodingRunnerStartProbe {
                events: Arc::clone(&events),
                provider_entry_tx,
                continue_rx: runner_continue_rx,
            },
        )
        .expect("winning recovery activates reserved runner");
        (updated, events, provider_entry_rx, runner_continue_tx)
    });

    reserved_rx.await.expect("winner reserved attempt");
    assert!(registry.has_active_recovery_reservation(&attempt_key));
    assert!(
        fixture
            .store
            .get_failed_code_review_recovery_journal(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &attempt_id,
            )
            .expect("journal before winner continues")
            .is_none()
    );

    let competing_messages = [
        CodingWsInMessage::AbortAttempt,
        CodingWsInMessage::ContextNote {
            content: "competing note".to_string(),
        },
    ];
    let mut competitors = competing_messages
        .into_iter()
        .map(|message| {
            let registry = registry.clone();
            let store = fixture.store.clone();
            let attempt_id = attempt_id.clone();
            tokio::spawn(async move {
                let (event_tx, _event_rx) = mpsc::channel(8);
                prepare_coding_message(
                    &store,
                    &registry,
                    &event_tx,
                    ("project_0001", "issue_0001", &attempt_id),
                    &message,
                )
                .await
                .expect("competing socket preparation")
            })
        })
        .collect::<Vec<_>>();

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(
        competitors
            .iter()
            .all(|competitor| !competitor.is_finished())
    );
    assert!(registry.has_active_recovery_reservation(&attempt_key));
    assert_eq!(
        fixture
            .store
            .get_attempt_by_id(&attempt_id)
            .expect("attempt while winner paused")
            .status,
        CodingAttemptStatus::Blocked
    );
    assert_eq!(
        fixture
            .store
            .list_role_runs(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &attempt_id,
            )
            .expect("role runs while winner paused")
            .len(),
        1
    );

    continue_tx.send(()).expect("continue recovery preparation");
    prepared_rx.await.expect("recovery guard released");
    for competitor in competitors.drain(..) {
        assert!(matches!(
            competitor.await.expect("competing preparation task"),
            CodingMessagePreparation::Rejected
        ));
    }

    assert!(registry.has_active_recovery_reservation(&attempt_key));
    let journal = fixture.store.get_failed_code_review_recovery_journal(
        &fixture.attempt.project_id,
        &fixture.attempt.issue_id,
        &attempt_id,
    );
    assert_eq!(
        journal
            .expect("journal after guard release")
            .expect("unfinished recovery journal")
            .phase,
        FailedCodeReviewRecoveryPhase::GateResolved
    );
    spawn_tx.send(()).expect("continue reserved spawn");
    let (updated, events, provider_entry_rx, runner_continue_tx) =
        winner.await.expect("winning recovery task");

    assert!(!registry.has_active_recovery_reservation(&attempt_key));
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    assert_eq!(
        fixture
            .store
            .list_role_runs(&updated.project_id, &updated.issue_id, &updated.id)
            .expect("role runs after winner recovery")
            .iter()
            .filter(|run| run.trigger
                == crate::product::coding_models::CodingRoleRunTrigger::RetryReview)
            .count(),
        1
    );
    assert!(
        fixture
            .store
            .list_context_notes(&updated.project_id, &updated.issue_id, &updated.id)
            .expect("context notes after competitors")
            .is_empty()
    );
    let journal = fixture
        .store
        .get_failed_code_review_recovery_journal(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
        )
        .expect("journal after winner recovery")
        .expect("winner recovery journal");
    assert_eq!(journal.phase, FailedCodeReviewRecoveryPhase::Completed);
    provider_entry_rx.await.expect("provider entry after spawn");
    assert_eq!(
        *events.lock().expect("runner start events"),
        vec!["task_created", "journal_completed", "provider_entry"]
    );
    drop(runner_continue_tx);
    wait_for_runner_count(&registry, &attempt_key, 0).await;
}

#[test]
fn failed_review_recovery_reservation_is_atomic_and_releasable() {
    let registry = Arc::new(CodingRunRegistry::default());
    let barrier = Arc::new(Barrier::new(3));
    let attempt_key = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
    let attempts = (0..2)
        .map(|_| {
            let registry = Arc::clone(&registry);
            let barrier = Arc::clone(&barrier);
            let attempt_key = attempt_key.clone();
            thread::spawn(move || {
                barrier.wait();
                registry.try_reserve_attempt(&attempt_key)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let reservations = attempts
        .into_iter()
        .map(|attempt| attempt.join().expect("reservation thread"))
        .collect::<Vec<_>>();

    assert_eq!(
        reservations
            .iter()
            .filter(|reservation| reservation.is_some())
            .count(),
        1
    );
    assert!(registry.attempt_is_reserved_or_running(&attempt_key));
    drop(reservations);
    assert!(!registry.attempt_is_reserved_or_running(&attempt_key));

    let reservation = registry
        .try_reserve_attempt(&attempt_key)
        .expect("reservation after release");
    let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
    assert!(
        registry
            .insert_cancellable(&attempt_key, ordinary_tx)
            .is_none()
    );
    assert_eq!(registry.runner_count(&attempt_key), 0);
    let (command_tx, _command_rx) = mpsc::channel(1);
    let run_id = reservation
        .activate_cancellable(command_tx)
        .expect("activate reserved runner")
        .run_id;
    assert_eq!(registry.runner_count(&attempt_key), 1);
    let (late_ordinary_tx, _late_ordinary_rx) = mpsc::channel(1);
    assert!(
        registry
            .insert_cancellable(&attempt_key, late_ordinary_tx)
            .is_none()
    );
    assert_eq!(registry.runner_count(&attempt_key), 1);
    assert!(registry.try_reserve_attempt(&attempt_key).is_none());
    registry.remove(&attempt_key, run_id);
    assert!(registry.try_reserve_attempt(&attempt_key).is_some());
}

async fn wait_for_runner_count(
    registry: &CodingRunRegistry,
    attempt_key: &CodingAttemptRunKey,
    expected: usize,
) {
    for _ in 0..20 {
        if registry.runner_count(attempt_key) == expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(registry.runner_count(attempt_key), expected);
}
