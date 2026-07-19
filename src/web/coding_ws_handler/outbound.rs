use axum::extract::ws::Message;
use futures_util::{Sink, SinkExt};
use tokio::sync::mpsc;

use super::CodingWsOutMessage;
use super::delivery_ack::{confirm_plan_amendment_socket_write, fail_plan_amendment_socket_write};

pub(crate) struct OutboundEventReceiver {
    receiver: mpsc::Receiver<CodingWsOutMessage>,
}

impl OutboundEventReceiver {
    pub(crate) fn new(receiver: mpsc::Receiver<CodingWsOutMessage>) -> Self {
        Self { receiver }
    }

    pub(crate) async fn recv(&mut self) -> Option<CodingWsOutMessage> {
        self.receiver.recv().await
    }

    pub(crate) fn try_recv(&mut self) -> Result<CodingWsOutMessage, mpsc::error::TryRecvError> {
        self.receiver.try_recv()
    }
}

impl Drop for OutboundEventReceiver {
    fn drop(&mut self) {
        self.receiver.close();
        while let Ok(event) = self.receiver.try_recv() {
            fail_plan_amendment_socket_write(&event);
        }
    }
}

struct OutboundWriteSettlement<'a> {
    event: &'a CodingWsOutMessage,
    armed: bool,
}

impl<'a> OutboundWriteSettlement<'a> {
    fn new(event: &'a CodingWsOutMessage) -> Self {
        Self { event, armed: true }
    }

    fn confirm(mut self) {
        confirm_plan_amendment_socket_write(self.event);
        self.armed = false;
    }

    fn fail(mut self) {
        fail_plan_amendment_socket_write(self.event);
        self.armed = false;
    }
}

impl Drop for OutboundWriteSettlement<'_> {
    fn drop(&mut self) {
        if self.armed {
            fail_plan_amendment_socket_write(self.event);
        }
    }
}

pub(crate) async fn send_coding_json<S>(socket: &mut S, message: &CodingWsOutMessage) -> bool
where
    S: Sink<Message> + Unpin,
{
    match serde_json::to_string(message) {
        Ok(json) => socket.send(Message::Text(json.into())).await.is_ok(),
        Err(_) => false,
    }
}

pub(crate) async fn send_coding_event<S>(socket: &mut S, event: &CodingWsOutMessage) -> bool
where
    S: Sink<Message> + Unpin,
{
    let settlement = OutboundWriteSettlement::new(event);
    let written = send_coding_json(socket, event).await;
    if written {
        settlement.confirm();
    } else {
        settlement.fail();
    }
    written
}

pub(crate) async fn flush_queued_coding_events<S>(
    socket: &mut S,
    receiver: &mut OutboundEventReceiver,
) where
    S: Sink<Message> + Unpin,
{
    while let Ok(event) = receiver.try_recv() {
        if !send_coding_event(socket, &event).await {
            break;
        }
    }
}
