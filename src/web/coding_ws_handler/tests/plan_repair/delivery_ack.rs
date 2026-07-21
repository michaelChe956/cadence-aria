use std::collections::BTreeMap;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::extract::ws::Message;
use futures_util::Sink;

use crate::product::models::{AmendmentResumeMode, AmendmentResumeTarget, PlanAmendmentManifest};
use crate::web::coding_ws_handler::delivery_ack::register_plan_amendment_socket_write;
use crate::web::coding_ws_handler::{OutboundEventReceiver, send_coding_event};

use super::*;

struct TestSocketSink {
    fail_write: bool,
    messages: Vec<Message>,
}

struct PendingSocketSink {
    flush_entered: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Sink<Message> for PendingSocketSink {
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
        self.flush_entered
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Poll::Pending
    }

    fn poll_close(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

impl TestSocketSink {
    fn successful() -> Self {
        Self {
            fail_write: false,
            messages: Vec::new(),
        }
    }

    fn failing() -> Self {
        Self {
            fail_write: true,
            messages: Vec::new(),
        }
    }
}

impl Sink<Message> for TestSocketSink {
    type Error = io::Error;

    fn poll_ready(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, message: Message) -> Result<(), Self::Error> {
        if self.fail_write {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "socket write failed",
            ));
        }
        self.messages.push(message);
        Ok(())
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn coding_ws_plan_repair_socket_writer_success_acknowledges_delivery() {
    let event = plan_amendment_event("event_socket_write_success");
    let waiter = register_plan_amendment_socket_write("event_socket_write_success").unwrap();
    let mut socket = TestSocketSink::successful();

    assert!(send_coding_event(&mut socket, &event).await);
    tokio::time::timeout(Duration::from_millis(250), waiter.wait())
        .await
        .expect("successful socket write must settle the acknowledgement")
        .expect("successful socket write must acknowledge delivery");
    assert_eq!(socket.messages.len(), 1);
}

#[tokio::test]
async fn coding_ws_plan_repair_socket_writer_failure_rejects_delivery_acknowledgement() {
    let event = plan_amendment_event("event_socket_write_failure");
    let waiter = register_plan_amendment_socket_write("event_socket_write_failure").unwrap();
    let mut socket = TestSocketSink::failing();

    assert!(!send_coding_event(&mut socket, &event).await);
    let error = tokio::time::timeout(Duration::from_millis(250), waiter.wait())
        .await
        .expect("failed socket write must settle the acknowledgement")
        .expect_err("failed socket write must reject delivery acknowledgement");
    assert!(
        error
            .to_string()
            .contains("plan_amendment_socket_write_failed:event_socket_write_failure")
    );
    assert!(socket.messages.is_empty());
}

#[tokio::test]
async fn coding_ws_plan_repair_socket_writer_abort_rejects_dequeued_delivery_acknowledgement() {
    let event_id = "event_socket_write_abort";
    let event = plan_amendment_event(event_id);
    let waiter = register_plan_amendment_socket_write(event_id).unwrap();
    let flush_entered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer_flush_entered = std::sync::Arc::clone(&flush_entered);
    let writer = tokio::spawn(async move {
        let mut socket = PendingSocketSink {
            flush_entered: writer_flush_entered,
        };
        send_coding_event(&mut socket, &event).await
    });

    tokio::time::timeout(Duration::from_millis(250), async {
        while !flush_entered.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("writer must dequeue the event and enter the pending socket flush");
    writer.abort();
    assert!(writer.await.unwrap_err().is_cancelled());

    let error = tokio::time::timeout(Duration::from_millis(250), waiter.wait())
        .await
        .expect("aborting a dequeued socket write must settle the acknowledgement")
        .expect_err("aborting a dequeued socket write must reject delivery acknowledgement");
    assert!(
        error
            .to_string()
            .contains("plan_amendment_socket_write_failed:event_socket_write_abort")
    );

    let retry_waiter = register_plan_amendment_socket_write(event_id).unwrap();
    let retry_event = plan_amendment_event(event_id);
    let mut retry_socket = TestSocketSink::successful();
    assert!(send_coding_event(&mut retry_socket, &retry_event).await);
    retry_waiter.wait().await.unwrap();
}

#[tokio::test]
async fn coding_ws_plan_repair_outbound_receiver_drop_rejects_queued_delivery_acknowledgement() {
    let event_id = "event_socket_receiver_drop";
    let event = plan_amendment_event(event_id);
    let waiter = register_plan_amendment_socket_write(event_id).unwrap();
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(1);
    let receiver = OutboundEventReceiver::new(event_rx);
    event_tx.send(event).await.unwrap();

    drop(receiver);

    let error = tokio::time::timeout(Duration::from_millis(250), waiter.wait())
        .await
        .expect("dropping the outbound receiver must settle queued acknowledgements")
        .expect_err("dropping the outbound receiver must reject queued delivery acknowledgement");
    assert!(
        error
            .to_string()
            .contains("plan_amendment_socket_write_failed:event_socket_receiver_drop")
    );

    let retry_waiter = register_plan_amendment_socket_write(event_id).unwrap();
    let retry_event = plan_amendment_event(event_id);
    let mut retry_socket = TestSocketSink::successful();
    assert!(send_coding_event(&mut retry_socket, &retry_event).await);
    retry_waiter.wait().await.unwrap();
}

#[tokio::test]
async fn coding_ws_plan_repair_outstanding_permit_receiver_drop_rejects_channel_aware_delivery_wait()
 {
    let ack_only_event_id = "event_socket_outstanding_permit_ack_only";
    let ack_only_waiter = register_plan_amendment_socket_write(ack_only_event_id).unwrap();
    let (ack_only_tx, ack_only_rx) = tokio::sync::mpsc::channel(1);
    let ack_only_permit = ack_only_tx.clone().reserve_owned().await.unwrap();
    let ack_only_receiver = OutboundEventReceiver::new(ack_only_rx);
    drop(ack_only_receiver);
    let ack_only_sender = ack_only_permit.send(plan_amendment_event(ack_only_event_id));
    assert!(ack_only_sender.is_closed());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), ack_only_waiter.wait())
            .await
            .is_err(),
        "socket-only acknowledgement wait must remain pending when an outstanding permit sends after receiver drop"
    );

    let event_id = "event_socket_outstanding_permit_channel_closed";
    let waiter = register_plan_amendment_socket_write(event_id).unwrap();
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(1);
    let outstanding_permit = event_tx.clone().reserve_owned().await.unwrap();
    let receiver = OutboundEventReceiver::new(event_rx);
    drop(receiver);
    let permit_sender = outstanding_permit.send(plan_amendment_event(event_id));
    assert!(permit_sender.is_closed());

    let error = tokio::time::timeout(
        Duration::from_millis(250),
        waiter.wait_or_channel_closed(&event_tx),
    )
    .await
    .expect("channel-aware wait must finish after receiver drop")
    .expect_err("channel-aware wait must reject delivery after receiver drop");
    assert!(error.to_string().contains(
        "plan_amendment_delivery_channel_closed:event_socket_outstanding_permit_channel_closed"
    ));

    let retry_waiter = register_plan_amendment_socket_write(event_id)
        .expect("channel-aware failure must remove the stale acknowledgement registration");
    drop(retry_waiter);
}

fn plan_amendment_event(event_id: &str) -> CodingWsOutMessage {
    CodingWsOutMessage::PlanAmendmentUpdated {
        event_id: event_id.to_string(),
        amendment: Box::new(PlanAmendmentManifest {
            id: "plan_amendment_socket_write".to_string(),
            repair_request_id: "plan_repair_request_socket_write".to_string(),
            previous_plan_revision_id: "plan_revision_0001".to_string(),
            new_plan_revision_id: "plan_revision_0002".to_string(),
            revised_work_items: BTreeMap::new(),
            superseded_revisions: Vec::new(),
            dependency_graph_changes: Vec::new(),
            contract_deltas: Vec::new(),
            unaffected_units: Vec::new(),
            revalidation_required_units: Vec::new(),
            stale_units: Vec::new(),
            replacement_units: BTreeMap::new(),
            resume_target: AmendmentResumeTarget {
                logical_work_item_id: "work_item_socket_write".to_string(),
                mode: AmendmentResumeMode::Reexecute,
            },
            created_at: "2026-07-19T00:00:00Z".to_string(),
        }),
    }
}
