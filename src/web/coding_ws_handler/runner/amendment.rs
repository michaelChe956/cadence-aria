use tokio::sync::mpsc;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::coding_workspace_engine::CodingWorkspaceEngineError;
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::web::state::{CodingRunReservation, WebAppState};

use super::{CodingRunnerTask, CodingWsOutMessage, spawn_coding_runner_task};

pub(crate) fn spawn_plan_amendment_runner_reserved(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
    reservation: CodingRunReservation,
) -> Result<mpsc::Sender<CodingRunnerCommand>, CodingWorkspaceEngineError> {
    let (command_tx, command_rx) = mpsc::channel(32);
    let registry_run_id = reservation.activate(command_tx.clone()).ok_or_else(|| {
        CodingWorkspaceEngineError::ProviderStream(
            "plan_amendment_runner_reservation_lost".to_string(),
        )
    })?;
    spawn_coding_runner_task(CodingRunnerTask {
        state,
        coding_store,
        event_tx,
        attempt,
        command_rx,
        registry_run_id,
        start_rx: None,
        probe: None,
    });
    Ok(command_tx)
}
