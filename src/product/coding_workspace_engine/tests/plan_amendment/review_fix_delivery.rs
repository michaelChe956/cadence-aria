use super::*;
use crate::product::coding_attempt_store::register_plan_amendment_delivery_mark_failpoint;

#[tokio::test]
async fn coding_amendment_delivery_send_failure_keeps_attempt_non_runnable_with_pending_marker() {
    let fixture = amendment_fixture().await;
    let (event_tx, event_rx) = mpsc::channel(1);
    drop(event_rx);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);

    let error = engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect_err("closed WS delivery channel must not resume the Attempt");

    assert!(error.to_string().contains("plan_amendment_delivery"));
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap()
            .status,
        CodingAttemptStatus::AmendmentApplyFailed
    );
    let marker_path = fixture
        .store
        .attempt_dir(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .join("amendment-event-deliveries")
        .join(format!("{}.json", fixture.manifest.id));
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(marker_path).unwrap()).unwrap();
    assert_eq!(marker["amendment_id"], fixture.manifest.id);
    assert_eq!(marker["status"], "pending");
}

#[tokio::test]
async fn coding_amendment_delivery_retries_same_event_after_send_before_mark_failure() {
    let mut fixture = amendment_fixture().await;
    let failpoint = register_plan_amendment_delivery_mark_failpoint(
        &fixture.store,
        &fixture.attempt,
        &fixture.manifest.id,
    );

    let error = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect_err("mark failpoint must interrupt delivery after send");

    assert!(error.to_string().contains("delivery_mark_failpoint"));
    let first_event_id =
        match tokio::time::timeout(std::time::Duration::from_secs(2), fixture._event_rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            CodingWsOutMessage::PlanAmendmentUpdated {
                event_id,
                amendment,
            } => {
                assert_eq!(amendment.id, fixture.manifest.id);
                event_id
            }
            event => panic!("unexpected event after send-before-mark failure: {event:?}"),
        };
    let failed = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    assert_eq!(failed.status, CodingAttemptStatus::AmendmentApplyFailed);
    assert_eq!(
        fixture
            .store
            .get_plan_amendment_delivery(&failed, &fixture.manifest.id)
            .unwrap()
            .status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Pending
    );
    drop(failpoint);

    let recovered = fixture
        .engine
        .recover_plan_amendment(&failed)
        .await
        .unwrap();

    assert_eq!(recovered.status, CodingAttemptStatus::Running);
    let second_event_id =
        match tokio::time::timeout(std::time::Duration::from_secs(2), fixture._event_rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            CodingWsOutMessage::PlanAmendmentUpdated {
                event_id,
                amendment,
            } => {
                assert_eq!(amendment.id, fixture.manifest.id);
                event_id
            }
            event => panic!("unexpected recovery event: {event:?}"),
        };
    assert_eq!(second_event_id, first_event_id);
    assert_eq!(
        fixture
            .store
            .get_plan_amendment_delivery(&recovered, &fixture.manifest.id)
            .unwrap()
            .status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coding_amendment_concurrent_recovery_reconciles_one_durable_delivery() {
    let mut fixture = amendment_fixture().await;
    let failpoint = register_plan_amendment_delivery_mark_failpoint(
        &fixture.store,
        &fixture.attempt,
        &fixture.manifest.id,
    );
    fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect_err("seed pending delivery");
    fixture._event_rx.recv().await.unwrap();
    drop(failpoint);
    let failed = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let (second_event_tx, mut second_event_rx) = mpsc::channel(8);
    let second_engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        second_event_tx,
    );

    let (left, right) = tokio::join!(
        fixture.engine.recover_plan_amendment(&failed),
        second_engine.recover_plan_amendment(&failed),
    );

    assert_eq!(left.unwrap().status, CodingAttemptStatus::Running);
    assert_eq!(right.unwrap().status, CodingAttemptStatus::Running);
    let mut recovery_events = Vec::new();
    while let Ok(event) = fixture._event_rx.try_recv() {
        recovery_events.push(event);
    }
    while let Ok(event) = second_event_rx.try_recv() {
        recovery_events.push(event);
    }
    assert_eq!(recovery_events.len(), 1);
    let event_id = match recovery_events.pop().unwrap() {
        CodingWsOutMessage::PlanAmendmentUpdated {
            event_id,
            amendment,
        } => {
            assert_eq!(amendment.id, fixture.manifest.id);
            event_id
        }
        event => panic!("unexpected concurrent recovery event: {event:?}"),
    };
    let delivery = fixture
        .store
        .get_plan_amendment_delivery(&failed, &fixture.manifest.id)
        .unwrap();
    assert_eq!(delivery.event_id, event_id);
    assert_eq!(
        delivery.status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered
    );
}
