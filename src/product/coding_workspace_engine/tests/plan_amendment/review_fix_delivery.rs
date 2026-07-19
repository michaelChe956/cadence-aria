use std::io;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll};

use axum::extract::ws::Message;
use futures_util::Sink;

use super::*;
use crate::product::coding_attempt_store::register_plan_amendment_delivery_mark_failpoint;
use crate::web::coding_ws_handler::delivery_ack::register_plan_amendment_socket_write;
use crate::web::coding_ws_handler::{OutboundEventReceiver, send_coding_event};

struct PendingDeliverySink {
    flush_entered: Arc<AtomicBool>,
}

impl Sink<Message> for PendingDeliverySink {
    type Error = io::Error;

    fn poll_ready(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, _message: Message) -> Result<(), Self::Error> {
        Ok(())
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.flush_entered.store(true, Ordering::SeqCst);
        Poll::Pending
    }

    fn poll_close(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn coding_amendment_delivery_enqueue_keeps_marker_pending_until_socket_write() {
    let fixture = amendment_fixture().await;
    let attempt = fixture.attempt.clone();
    let manifest = fixture.manifest.clone();
    let store = fixture.store.clone();
    let marker_attempt = attempt.clone();
    let marker_manifest = manifest.clone();
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let task = tokio::spawn(async move {
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
            .apply_plan_amendment(&attempt, &manifest)
            .await
    });

    let event = tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        CodingWsOutMessage::PlanAmendmentUpdated { .. }
    ));
    assert_eq!(
        fixture
            .store
            .get_plan_amendment_delivery(&marker_attempt, &marker_manifest.id)
            .unwrap()
            .status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Pending
    );
    assert!(
        !task.is_finished(),
        "producer must wait for the socket writer acknowledgement"
    );
    task.abort();
}

#[tokio::test]
async fn coding_amendment_delivery_channel_closed_keeps_attempt_non_runnable_with_pending_marker() {
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
async fn coding_amendment_delivery_socket_write_failure_keeps_attempt_non_runnable_with_pending_marker()
 {
    let fixture = amendment_fixture().await;
    let attempt = fixture.attempt.clone();
    let manifest = fixture.manifest.clone();
    let store = fixture.store.clone();
    let marker_attempt = attempt.clone();
    let marker_manifest = manifest.clone();
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let task = tokio::spawn(async move {
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
            .apply_plan_amendment(&attempt, &manifest)
            .await
    });

    let event = tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    crate::web::coding_ws_handler::delivery_ack::fail_plan_amendment_socket_write(&event);
    let error = task
        .await
        .unwrap()
        .expect_err("socket write failure must not resume the Attempt");

    assert!(
        error
            .to_string()
            .contains("plan_amendment_socket_write_failed")
    );
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &marker_attempt.project_id,
                &marker_attempt.issue_id,
                &marker_attempt.id,
            )
            .unwrap()
            .status,
        CodingAttemptStatus::AmendmentApplyFailed
    );
    assert_eq!(
        fixture
            .store
            .get_plan_amendment_delivery(&marker_attempt, &marker_manifest.id)
            .unwrap()
            .status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Pending
    );
}

#[tokio::test]
async fn coding_amendment_delivery_writer_abort_releases_arbitration_and_recovers_same_event() {
    let fixture = amendment_fixture().await;
    let attempt = fixture.attempt.clone();
    let manifest = fixture.manifest.clone();
    let store = fixture.store.clone();
    let (event_tx, event_rx) = mpsc::channel(8);
    let mut outbound = OutboundEventReceiver::new(event_rx);
    let apply = tokio::spawn(async move {
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
            .apply_plan_amendment(&attempt, &manifest)
            .await
    });
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), outbound.recv())
        .await
        .unwrap()
        .unwrap();
    let event_id = plan_amendment_event_id(&event);
    let flush_entered = Arc::new(AtomicBool::new(false));
    let writer_flush_entered = Arc::clone(&flush_entered);
    let writer = tokio::spawn(async move {
        let mut sink = PendingDeliverySink {
            flush_entered: writer_flush_entered,
        };
        send_coding_event(&mut sink, &event).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !flush_entered.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("writer must enter the pending socket flush");
    writer.abort();
    assert!(writer.await.unwrap_err().is_cancelled());
    let error = tokio::time::timeout(std::time::Duration::from_secs(2), apply)
        .await
        .expect("writer abort must release the amendment producer")
        .unwrap()
        .expect_err("writer abort must fail delivery before the Attempt becomes runnable");
    assert!(
        error
            .to_string()
            .contains("plan_amendment_socket_write_failed")
    );
    drop(outbound);

    assert_pending_delivery_and_recover_same_event(&fixture, &event_id).await;
}

#[tokio::test]
async fn coding_amendment_delivery_receiver_drop_releases_arbitration_and_recovers_same_event() {
    let fixture = amendment_fixture().await;
    let attempt = fixture.attempt.clone();
    let manifest = fixture.manifest.clone();
    let store = fixture.store.clone();
    let (event_tx, event_rx) = mpsc::channel(1);
    let capacity_probe = event_tx.clone();
    let outbound = OutboundEventReceiver::new(event_rx);
    let apply = tokio::spawn(async move {
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
            .apply_plan_amendment(&attempt, &manifest)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while capacity_probe.capacity() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("producer must enqueue the amendment event before receiver drop");
    let event_id = fixture
        .store
        .get_plan_amendment_delivery(&fixture.attempt, &fixture.manifest.id)
        .unwrap()
        .event_id;

    drop(outbound);

    let error = tokio::time::timeout(std::time::Duration::from_secs(2), apply)
        .await
        .expect("receiver drop must release the amendment producer")
        .unwrap()
        .expect_err("receiver drop must fail delivery before the Attempt becomes runnable");
    assert!(
        error
            .to_string()
            .contains("plan_amendment_delivery_channel_closed")
    );

    assert_pending_delivery_and_recover_same_event(&fixture, &event_id).await;
}

#[tokio::test]
async fn coding_amendment_delivery_outstanding_permit_receiver_drop_releases_waiter_and_recovers_same_event()
 {
    let fixture = amendment_fixture().await;
    let attempt = fixture.attempt.clone();
    let manifest = fixture.manifest.clone();
    let store = fixture.store.clone();
    let (event_tx, event_rx) = mpsc::channel(2);
    let outstanding_permit = event_tx
        .clone()
        .reserve_owned()
        .await
        .expect("the first channel slot must be reserved before amendment delivery");
    let capacity_probe = event_tx.clone();
    let outbound = OutboundEventReceiver::new(event_rx);
    let apply = tokio::spawn(async move {
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
            .apply_plan_amendment(&attempt, &manifest)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while capacity_probe.capacity() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("amendment delivery must occupy the slot behind the outstanding permit");
    let event_id = fixture
        .store
        .get_plan_amendment_delivery(&fixture.attempt, &fixture.manifest.id)
        .unwrap()
        .event_id;

    drop(outbound);
    let permit_sender = outstanding_permit.send(CodingWsOutMessage::CodingPong);
    drop(permit_sender);

    let error = tokio::time::timeout(std::time::Duration::from_millis(250), apply)
        .await
        .expect("receiver closure must release an ACK wait hidden behind an outstanding permit")
        .unwrap()
        .expect_err("receiver closure must fail delivery before the Attempt becomes runnable");
    assert!(
        error
            .to_string()
            .contains("plan_amendment_delivery_channel_closed"),
        "unexpected delivery error: {error}"
    );

    let retry_registration = register_plan_amendment_socket_write(&event_id)
        .expect("failed channel wait must remove the stale ACK registration");
    drop(retry_registration);
    assert_pending_delivery_and_recover_same_event(&fixture, &event_id).await;
}

#[tokio::test]
async fn coding_amendment_delivery_retries_same_event_after_send_before_mark_failure() {
    let fixture = amendment_fixture().await;
    let failpoint = register_plan_amendment_delivery_mark_failpoint(
        &fixture.store,
        &fixture.attempt,
        &fixture.manifest.id,
    );
    let attempt = fixture.attempt.clone();
    let manifest = fixture.manifest.clone();
    let store = fixture.store.clone();
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let apply = tokio::spawn(async move {
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
            .apply_plan_amendment(&attempt, &manifest)
            .await
    });
    let first_event = tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let first_event_id = match &first_event {
        CodingWsOutMessage::PlanAmendmentUpdated {
            event_id,
            amendment,
        } => {
            assert_eq!(amendment.id, fixture.manifest.id);
            event_id.clone()
        }
        event => panic!("unexpected event after send-before-mark failure: {event:?}"),
    };
    crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(&first_event);
    let error = apply
        .await
        .unwrap()
        .expect_err("mark failpoint must interrupt delivery after socket write");
    assert!(error.to_string().contains("delivery_mark_failpoint"));
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

    let (reconnect_tx, mut reconnect_rx) = mpsc::channel(8);
    let reconnect_engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        reconnect_tx,
    );
    let recovery_attempt = failed.clone();
    let recovery = tokio::spawn(async move {
        reconnect_engine
            .recover_plan_amendment(&recovery_attempt)
            .await
    });
    let second_event = tokio::time::timeout(std::time::Duration::from_secs(2), reconnect_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let second_event_id = match &second_event {
        CodingWsOutMessage::PlanAmendmentUpdated {
            event_id,
            amendment,
        } => {
            assert_eq!(amendment.id, fixture.manifest.id);
            event_id.clone()
        }
        event => panic!("unexpected recovery event: {event:?}"),
    };
    crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(&second_event);
    let recovered = recovery.await.unwrap().unwrap();

    assert_eq!(recovered.status, CodingAttemptStatus::Running);
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
    let (left_event_tx, mut left_event_rx) = mpsc::channel(8);
    let left_engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        left_event_tx,
    );
    let (right_event_tx, mut right_event_rx) = mpsc::channel(8);
    let right_engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        right_event_tx,
    );
    let left_attempt = failed.clone();
    let right_attempt = failed.clone();
    let left = tokio::spawn(async move { left_engine.recover_plan_amendment(&left_attempt).await });
    let right =
        tokio::spawn(async move { right_engine.recover_plan_amendment(&right_attempt).await });
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::select! {
            event = left_event_rx.recv() => event.unwrap(),
            event = right_event_rx.recv() => event.unwrap(),
        }
    })
    .await
    .expect("one recovery must reach socket delivery");
    crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(&event);

    let left = tokio::time::timeout(std::time::Duration::from_secs(2), left)
        .await
        .expect("left recovery deadlocked")
        .unwrap();
    let right = tokio::time::timeout(std::time::Duration::from_secs(2), right)
        .await
        .expect("right recovery deadlocked")
        .unwrap();

    assert_eq!(left.unwrap().status, CodingAttemptStatus::Running);
    assert_eq!(right.unwrap().status, CodingAttemptStatus::Running);
    assert!(left_event_rx.try_recv().is_err());
    assert!(right_event_rx.try_recv().is_err());
    let event_id = match event {
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

fn plan_amendment_event_id(event: &CodingWsOutMessage) -> String {
    match event {
        CodingWsOutMessage::PlanAmendmentUpdated { event_id, .. } => event_id.clone(),
        event => panic!("unexpected amendment delivery event: {event:?}"),
    }
}

async fn assert_pending_delivery_and_recover_same_event(
    fixture: &AmendmentFixture,
    expected_event_id: &str,
) {
    let failed = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    assert_eq!(failed.status, CodingAttemptStatus::AmendmentApplyFailed);
    let pending = fixture
        .store
        .get_plan_amendment_delivery(&failed, &fixture.manifest.id)
        .unwrap();
    assert_eq!(pending.event_id, expected_event_id);
    assert_eq!(
        pending.status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Pending
    );

    let (event_tx, event_rx) = mpsc::channel(8);
    let mut outbound = OutboundEventReceiver::new(event_rx);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let recovery = tokio::spawn(async move { engine.recover_plan_amendment(&failed).await });
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), outbound.recv())
        .await
        .expect("recovery must reacquire amendment arbitration")
        .expect("recovery must enqueue the pending amendment event");
    assert_eq!(plan_amendment_event_id(&event), expected_event_id);
    crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(&event);
    let recovered = tokio::time::timeout(std::time::Duration::from_secs(2), recovery)
        .await
        .expect("same-event recovery must finish")
        .unwrap()
        .unwrap();
    assert_eq!(recovered.status, CodingAttemptStatus::Running);
    assert_eq!(
        fixture
            .store
            .get_plan_amendment_delivery(&recovered, &fixture.manifest.id)
            .unwrap()
            .status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered
    );
}
