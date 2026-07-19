use std::collections::BTreeMap;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::extract::ws::Message;
use futures_util::Sink;

use crate::product::models::{AmendmentResumeMode, AmendmentResumeTarget, PlanAmendmentManifest};
use crate::web::coding_ws_handler::delivery_ack::register_plan_amendment_socket_write;
use crate::web::coding_ws_handler::send_coding_event;

use super::*;

struct TestSocketSink {
    fail_write: bool,
    messages: Vec<Message>,
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
