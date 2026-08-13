use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::web::coding_ws_handler::{CodingWsOutMessage, emit_current_session_state};
use crate::web::state::{CodingAttemptRunKey, WebAppState};

use super::registration::CodingRunnerRegistrationGuard;
use super::{
    CodingRunnerStartProbe, execute_start_coding_flow, record_runner_start_event,
    should_emit_coding_runner_protocol_error,
};

pub(super) struct CodingRunnerTask {
    pub(super) state: WebAppState,
    pub(super) coding_store: CodingAttemptStore,
    pub(super) event_tx: mpsc::Sender<CodingWsOutMessage>,
    pub(super) attempt: CodingExecutionAttempt,
    pub(super) command_rx: mpsc::Receiver<CodingRunnerCommand>,
    pub(super) registry_run_id: u64,
    pub(super) cancellation: CancellationToken,
    pub(super) start_rx: Option<oneshot::Receiver<()>>,
    pub(super) probe: Option<CodingRunnerStartProbe>,
    #[cfg(test)]
    pub(super) panic_after_registration: Option<oneshot::Sender<()>>,
}

pub(super) fn spawn_coding_runner_task(task: CodingRunnerTask) {
    let CodingRunnerTask {
        state,
        coding_store,
        event_tx,
        attempt,
        command_rx,
        registry_run_id,
        cancellation,
        start_rx,
        probe,
        #[cfg(test)]
        panic_after_registration,
    } = task;
    let registry_attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let coding_runs = state.coding_runs.clone();
    tokio::spawn(async move {
        let _registration_guard =
            CodingRunnerRegistrationGuard::new(coding_runs, registry_attempt_key, registry_run_id);
        #[cfg(test)]
        if let Some(panic_entered_tx) = panic_after_registration {
            let _ = panic_entered_tx.send(());
            panic!("coding runner panic cleanup probe");
        }
        let engine_cancellation = cancellation.clone();
        let business = run_coding_runner_task_body(
            state,
            coding_store,
            event_tx,
            attempt,
            command_rx,
            engine_cancellation,
            start_rx,
            probe,
        );
        tokio::pin!(business);
        tokio::select! {
            biased;
            _ = &mut business => {}
            _ = cancellation.cancelled() => {
                business.await;
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
async fn run_coding_runner_task_body(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
    command_rx: mpsc::Receiver<CodingRunnerCommand>,
    cancellation: CancellationToken,
    start_rx: Option<oneshot::Receiver<()>>,
    probe: Option<CodingRunnerStartProbe>,
) {
    if let Some(start_rx) = start_rx {
        tokio::select! {
            result = start_rx => {
                if result.is_err() {
                    return;
                }
            }
            _ = cancellation.cancelled() => return,
        }
    }
    if let Some(probe) = probe {
        record_runner_start_event(Some(&probe.events), "provider_entry");
        let _ = probe.provider_entry_tx.send(());
        tokio::select! {
            result = probe.continue_rx => {
                if result.is_err() {
                    return;
                }
            }
            _ = cancellation.cancelled() => {
                record_runner_start_event(Some(&probe.events), "cancelled_before_provider");
                return;
            }
        }
    }
    let mut engine = CodingWorkspaceEngine::new(
        coding_store.clone(),
        GitWorkspaceService::new(),
        event_tx.clone(),
    )
    .with_cancellation(cancellation.clone());
    if attempt.target_snapshot.is_some()
        && let Some(factory) = state.gateway_factory()
        && let Ok(gateway) = factory.build(&attempt.project_id)
    {
        engine = engine.with_logical_provider_gateway(Arc::new(gateway));
    }
    let result = execute_start_coding_flow(
        &state,
        &coding_store,
        &engine,
        &event_tx,
        command_rx,
        &attempt,
    )
    .await;
    if cancellation.is_cancelled() {
        return;
    }
    if let Err(error) = result
        && !matches!(error, CodingWorkspaceEngineError::Aborted)
    {
        let latest_attempt =
            coding_store.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id);
        if let Ok(latest_attempt) = latest_attempt
            && !should_emit_coding_runner_protocol_error(&latest_attempt.status)
        {
            if let Err(snapshot_error) =
                emit_current_session_state(&event_tx, &coding_store, &latest_attempt, &cancellation)
                    .await
            {
                tracing::warn!(
                    attempt_id = attempt.id.as_str(),
                    error = %snapshot_error,
                    "failed to rebuild recoverable coding session state"
                );
            }
        } else {
            let code = match &error {
                CodingWorkspaceEngineError::ExecutionPlanNotConfirmed(_) => {
                    "work_item_execution_plan_not_confirmed".to_string()
                }
                _ if error
                    .to_string()
                    .contains("plan_amendment_blocks_provider_run") =>
                {
                    "plan_amendment_blocks_provider_run".to_string()
                }
                _ => "coding_start_failed".to_string(),
            };
            let event = CodingWsOutMessage::CodingProtocolError {
                code,
                message: error.to_string(),
            };
            let permit = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return,
                permit = event_tx.reserve() => permit,
            };
            if let Ok(permit) = permit {
                permit.send(event);
            }
        }
    }
}
