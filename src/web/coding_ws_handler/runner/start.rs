use std::sync::{Arc, Mutex};

#[cfg(test)]
use tokio::sync::mpsc;
use tokio::sync::oneshot;

#[cfg(test)]
use crate::product::coding_attempt_store::CodingAttemptStore;
#[cfg(test)]
use crate::product::coding_models::CodingExecutionAttempt;
#[cfg(test)]
use crate::web::state::{CodingAttemptRunKey, WebAppState};

#[cfg(test)]
use super::{CodingRunnerTask, CodingWsOutMessage, spawn_coding_runner_task};

pub(crate) struct CodingRunnerStartProbe {
    pub(crate) events: Arc<Mutex<Vec<&'static str>>>,
    pub(crate) provider_entry_tx: oneshot::Sender<()>,
    pub(crate) continue_rx: oneshot::Receiver<()>,
}

pub(super) fn record_runner_start_event(
    events: Option<&Arc<Mutex<Vec<&'static str>>>>,
    event: &'static str,
) {
    if let Some(events) = events {
        events
            .lock()
            .expect("coding runner start events")
            .push(event);
    }
}

#[cfg(test)]
pub(crate) fn spawn_coding_runner_panicking_after_registration(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
) -> oneshot::Receiver<()> {
    let (command_tx, command_rx) = mpsc::channel(1);
    let registry_attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let registry_run_id = state
        .coding_runs
        .insert(&registry_attempt_key, command_tx)
        .expect("panic test runner registration");
    let (panic_entered_tx, panic_entered_rx) = oneshot::channel();
    spawn_coding_runner_task(CodingRunnerTask {
        state,
        coding_store,
        event_tx,
        attempt,
        command_rx,
        registry_run_id,
        start_rx: None,
        probe: None,
        panic_after_registration: Some(panic_entered_tx),
    });
    panic_entered_rx
}
