use super::*;
use crate::product::coding_models::CodingExecutionUnitStatus;
use crate::web::coding_ws_handler::socket::{CodingMessagePreparation, prepare_coding_message};

#[tokio::test]
async fn recoverable_group_review_provider_failure_blocks_and_preserves_active_unit() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let active_unit_id = fixture
        .attempt
        .active_unit_id
        .as_deref()
        .expect("active unit id");

    assert_eq!(fixture.attempt.status, CodingAttemptStatus::Blocked);
    assert_eq!(fixture.attempt.completed_at, None);
    assert_eq!(
        fixture.attempt.current_work_item_id.as_deref(),
        Some(fixture.attempt.work_item_id.as_str())
    );
    let active = fixture
        .store
        .get_active_coding_unit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("active unit lookup")
        .expect("active unit");
    assert_eq!(active.id, active_unit_id);
    assert_eq!(active.status, CodingExecutionUnitStatus::Running);
    assert!(
        recoverable_failed_code_review(&fixture.store, &fixture.attempt)
            .expect("recoverable review context")
            .is_some()
    );
}

#[tokio::test]
async fn recoverable_review_uses_exact_gate_and_cannot_start_ordinary_runner() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItemGroup).await;
    let gate = fixture
        .dirty_gate
        .as_ref()
        .expect("provider interruption gate");
    let role_runs_before = fixture
        .store
        .list_role_runs(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("role runs before rejected start");
    let registry = CodingRunRegistry::default();
    let (event_tx, _event_rx) = mpsc::channel(8);

    let rejected = prepare_coding_message(
        &fixture.store,
        &registry,
        &event_tx,
        (
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        ),
        &CodingWsInMessage::StartCoding,
    )
    .await
    .expect("ordinary start preparation");
    assert!(matches!(rejected, CodingMessagePreparation::Rejected));
    assert_eq!(
        fixture
            .store
            .list_role_runs(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("role runs after rejected start"),
        role_runs_before
    );

    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let recovered = engine
        .recover_failed_code_review_for_attempt(&fixture.attempt, &gate.gate_id)
        .await
        .expect("explicit failed review recovery");
    assert_eq!(recovered.status, CodingAttemptStatus::Running);
    assert_eq!(recovered.active_unit_id, fixture.attempt.active_unit_id);
    assert_eq!(
        recovered.current_work_item_id,
        fixture.attempt.current_work_item_id
    );
}

#[tokio::test]
async fn recoverable_single_review_provider_failure_also_uses_blocked() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItem).await;

    assert_eq!(fixture.attempt.status, CodingAttemptStatus::Blocked);
    assert_eq!(fixture.attempt.scope, CodingAttemptScope::WorkItem);
    assert_eq!(fixture.attempt.completed_at, None);
    assert!(fixture.attempt.active_unit_id.is_none());
    assert!(
        recoverable_failed_code_review(&fixture.store, &fixture.attempt)
            .expect("single recoverable review context")
            .is_some()
    );
}
