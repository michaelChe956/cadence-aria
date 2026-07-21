use crate::web::state::{CodingAttemptRunKey, CodingRunRegistry};

use super::super::CodingWsOutMessage;
use super::super::outbound::OutboundEventReceiver;

pub(crate) struct CodingAbortDrainResult {
    pub(crate) aborted_runners: usize,
    pub(crate) events: Vec<CodingWsOutMessage>,
}

pub(crate) async fn abort_attempt_while_draining_events(
    registry: &CodingRunRegistry,
    attempt_key: &CodingAttemptRunKey,
    event_rx: &mut OutboundEventReceiver,
) -> CodingAbortDrainResult {
    let mut events = Vec::new();
    let abort = registry.abort_attempt(attempt_key);
    tokio::pin!(abort);
    let aborted_runners = loop {
        tokio::select! {
            aborted_runners = &mut abort => break aborted_runners,
            event = event_rx.recv() => {
                match event {
                    Some(event) => events.push(event),
                    None => break abort.await,
                }
            }
        }
    };
    while let Ok(event) = event_rx.try_recv() {
        events.push(event);
    }
    CodingAbortDrainResult {
        aborted_runners,
        events,
    }
}
