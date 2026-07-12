use std::sync::{Arc, Barrier};
use std::thread;

use tokio::sync::{mpsc, oneshot};

use crate::product::coding_attempt_store::FailedCodeReviewRecoveryPhase;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::web::coding_ws_handler::socket::failed_code_review_recovery_request;
use crate::web::coding_ws_handler::{
    CodingRunnerStartProbe, CodingWsInMessage, is_coding_ws_message_allowed,
    spawn_coding_runner_reserved_with_probe,
};
use crate::web::runtime::WebRuntime;
use crate::web::state::{CodingRunRegistry, WebAppState};

use super::support::{FixtureCase, failed_review_fixture};

#[tokio::test]
async fn reserved_spawn_creates_task_then_completes_journal_before_provider_entry() {
    let fixture =
        failed_review_fixture(CodingAttemptScope::WorkItemGroup, FixtureCase::Recoverable);
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
    let reservation = state
        .coding_runs
        .try_reserve_attempt(&updated.id)
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
    wait_for_runner_count(&state.coding_runs, &updated.id, 0).await;
}

#[tokio::test]
async fn journal_completion_failure_stops_task_before_provider_entry_and_cleans_registry() {
    let fixture =
        failed_review_fixture(CodingAttemptScope::WorkItemGroup, FixtureCase::Recoverable);
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
    let reservation = state
        .coding_runs
        .try_reserve_attempt(&updated.id)
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
    wait_for_runner_count(&state.coding_runs, &updated.id, 0).await;
    assert_eq!(
        *events.lock().expect("runner start events"),
        vec!["task_created"]
    );
}

#[test]
fn failed_review_websocket_guard_allows_only_the_exact_retry_gate_request() {
    let fixture =
        failed_review_fixture(CodingAttemptScope::WorkItemGroup, FixtureCase::Recoverable);
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

#[test]
fn failed_review_recovery_reservation_is_atomic_and_releasable() {
    let registry = Arc::new(CodingRunRegistry::default());
    let barrier = Arc::new(Barrier::new(3));
    let attempts = (0..2)
        .map(|_| {
            let registry = Arc::clone(&registry);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                registry.try_reserve_attempt("coding_attempt_0001")
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
    assert!(registry.attempt_is_reserved_or_running("coding_attempt_0001"));
    drop(reservations);
    assert!(!registry.attempt_is_reserved_or_running("coding_attempt_0001"));

    let reservation = registry
        .try_reserve_attempt("coding_attempt_0001")
        .expect("reservation after release");
    let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
    assert!(
        registry
            .insert("coding_attempt_0001".to_string(), ordinary_tx)
            .is_none()
    );
    assert_eq!(registry.runner_count("coding_attempt_0001"), 0);
    let (command_tx, _command_rx) = mpsc::channel(1);
    let run_id = reservation
        .activate(command_tx)
        .expect("activate reserved runner");
    assert_eq!(registry.runner_count("coding_attempt_0001"), 1);
    let (late_ordinary_tx, _late_ordinary_rx) = mpsc::channel(1);
    assert!(
        registry
            .insert("coding_attempt_0001".to_string(), late_ordinary_tx)
            .is_none()
    );
    assert_eq!(registry.runner_count("coding_attempt_0001"), 1);
    assert!(
        registry
            .try_reserve_attempt("coding_attempt_0001")
            .is_none()
    );
    registry.remove("coding_attempt_0001", run_id);
    assert!(
        registry
            .try_reserve_attempt("coding_attempt_0001")
            .is_some()
    );
}

async fn wait_for_runner_count(registry: &CodingRunRegistry, attempt_id: &str, expected: usize) {
    for _ in 0..20 {
        if registry.runner_count(attempt_id) == expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(registry.runner_count(attempt_id), expected);
}
