use tokio::sync::mpsc;

use crate::product::coding_attempt_store::FailedCodeReviewRecoveryPhase;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage, CodingRoleRunStatus,
};
use crate::product::coding_workspace_engine::{
    CodingWorkspaceEngine, recoverable_failed_code_review,
};
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::web::coding_ws_handler::CodingWsInMessage;
use crate::web::coding_ws_handler::socket::failed_code_review_recovery_request;

use super::super::{CodingWsOutMessage, build_coding_session_state};
use super::support::{provider_interrupted_review_fixture, seed_repeated_interrupted_review};

#[tokio::test]
async fn completed_journal_rotates_when_later_review_is_interrupted() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let repeated = seed_repeated_interrupted_review(&fixture).await;
    let second_gate_id = repeated.second_gate.gate_id.clone();
    let recovery = recoverable_failed_code_review(&fixture.store, &repeated.blocked_attempt)
        .expect("recoverable second interruption")
        .expect("second recovery identity");
    assert_eq!(recovery.gate_id, second_gate_id);
    assert_eq!(recovery.failed_node_id, "coding_node_0010");
    assert_eq!(recovery.stale_role_run_id, repeated.first_retry_role_run_id);

    let state = build_coding_session_state(&fixture.store, repeated.blocked_attempt.clone())
        .expect("session state for second interruption");
    let CodingWsOutMessage::CodingSessionState { pending_gates, .. } = state else {
        panic!("expected coding session state");
    };
    assert!(
        pending_gates
            .iter()
            .any(|gate| gate.gate_id == second_gate_id)
    );
    assert!(failed_code_review_recovery_request(
        &fixture.store,
        &repeated.blocked_attempt,
        &CodingWsInMessage::GateResponse {
            gate_id: second_gate_id.clone(),
            action_id: "retry_review".to_string(),
            extra_context: None,
        },
    ));

    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let running = engine
        .recover_failed_code_review_for_attempt(&fixture.attempt, &second_gate_id)
        .await
        .expect("recover second interrupted review");
    assert_eq!(running.status, CodingAttemptStatus::Running);
    assert_eq!(running.stage, CodingExecutionStage::CodeReview);
    assert_eq!(
        running.active_unit_id,
        repeated.blocked_attempt.active_unit_id
    );

    let current = fixture
        .store
        .get_failed_code_review_recovery_journal(
            &running.project_id,
            &running.issue_id,
            &running.id,
        )
        .expect("current journal")
        .expect("second recovery journal");
    assert_eq!(current.expected_gate_id, second_gate_id);
    assert_eq!(current.phase, FailedCodeReviewRecoveryPhase::GateResolved);
    assert_eq!(
        fixture
            .store
            .get_archived_failed_code_review_recovery_journal(
                &running.project_id,
                &running.issue_id,
                &running.id,
                &repeated.first_journal.expected_gate_id,
            )
            .expect("archived first journal")
            .expect("first recovery history"),
        repeated.first_journal
    );

    let runs = fixture
        .store
        .list_role_runs(&running.project_id, &running.issue_id, &running.id)
        .expect("role runs after second recovery");
    let first_retry = runs
        .iter()
        .find(|run| run.id == repeated.first_retry_role_run_id)
        .expect("first retry reviewer run");
    let second_retry = runs
        .iter()
        .find(|run| run.reason_code.as_deref() == Some(current.recovery_key.as_str()))
        .expect("second retry reviewer run");
    assert_eq!(first_retry.status, CodingRoleRunStatus::Failed);
    assert_eq!(
        first_retry.superseded_by_run_id.as_deref(),
        Some(second_retry.id.as_str())
    );
    assert_eq!(
        second_retry.supersedes_run_id.as_deref(),
        Some(repeated.first_retry_role_run_id.as_str())
    );

    let first_gate_retry = CodingWsInMessage::GateResponse {
        gate_id: repeated.first_journal.expected_gate_id,
        action_id: "retry_review".to_string(),
        extra_context: None,
    };
    assert!(!failed_code_review_recovery_request(
        &fixture.store,
        &running,
        &first_gate_retry,
    ));
}
