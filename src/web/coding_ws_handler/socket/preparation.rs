use tokio::sync::mpsc;

#[cfg(test)]
use tokio::sync::oneshot;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::web::state::{CodingAttemptMutationLease, CodingRunRegistry, CodingRunReservation};

use super::{
    CodingMessageAdmission, CodingWsInMessage, CodingWsOutMessage, coding_message_admission,
};

pub(crate) type CodingAttemptIdentity<'a> = (&'a str, &'a str, &'a str);

pub(crate) enum CodingMessagePreparation {
    Hello,
    Ping,
    Allowed {
        attempt: CodingExecutionAttempt,
        mutation_lease: CodingAttemptMutationLease,
    },
    FailedReviewRecovery {
        attempt: CodingExecutionAttempt,
        gate_id: String,
        reservation: CodingRunReservation,
    },
    Rejected,
    RecoveryAlreadyActive,
}

#[derive(Debug)]
pub(crate) enum CodingMessagePreparationError {
    AttemptUnavailable,
    Recovery(CodingWorkspaceEngineError),
}

#[cfg(test)]
pub(crate) struct CodingRecoveryPreparationProbe {
    pub(crate) reserved_tx: oneshot::Sender<()>,
    pub(crate) continue_rx: oneshot::Receiver<()>,
}

pub(crate) async fn prepare_coding_message(
    coding_store: &CodingAttemptStore,
    coding_runs: &CodingRunRegistry,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    identity: CodingAttemptIdentity<'_>,
    inbound: &CodingWsInMessage,
) -> Result<CodingMessagePreparation, CodingMessagePreparationError> {
    prepare_coding_message_inner(coding_store, coding_runs, event_tx, identity, inbound, None).await
}

#[cfg(test)]
pub(crate) async fn prepare_coding_message_with_probe(
    coding_store: &CodingAttemptStore,
    coding_runs: &CodingRunRegistry,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    identity: CodingAttemptIdentity<'_>,
    inbound: &CodingWsInMessage,
    probe: CodingRecoveryPreparationProbe,
) -> Result<CodingMessagePreparation, CodingMessagePreparationError> {
    prepare_coding_message_inner(
        coding_store,
        coding_runs,
        event_tx,
        identity,
        inbound,
        Some(probe),
    )
    .await
}

async fn prepare_coding_message_inner(
    coding_store: &CodingAttemptStore,
    coding_runs: &CodingRunRegistry,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    identity: CodingAttemptIdentity<'_>,
    inbound: &CodingWsInMessage,
    #[cfg(test)] probe: Option<CodingRecoveryPreparationProbe>,
    #[cfg(not(test))] _probe: Option<()>,
) -> Result<CodingMessagePreparation, CodingMessagePreparationError> {
    match inbound {
        CodingWsInMessage::CodingHello { .. } => return Ok(CodingMessagePreparation::Hello),
        CodingWsInMessage::CodingPing => return Ok(CodingMessagePreparation::Ping),
        _ => {}
    }

    let (project_id, issue_id, attempt_id) = identity;
    let attempt_guard = coding_runs.lock_attempt(attempt_id).await;
    let mutation_lease = coding_runs.lock_attempt_mutation(attempt_id).await;
    let current_attempt = coding_store
        .get_attempt(project_id, issue_id, attempt_id)
        .map_err(|_| CodingMessagePreparationError::AttemptUnavailable)?;
    match coding_message_admission(coding_store, coding_runs, &current_attempt, inbound) {
        CodingMessageAdmission::Rejected => {
            drop(attempt_guard);
            Ok(CodingMessagePreparation::Rejected)
        }
        CodingMessageAdmission::Allowed => {
            drop(attempt_guard);
            Ok(CodingMessagePreparation::Allowed {
                attempt: current_attempt,
                mutation_lease,
            })
        }
        CodingMessageAdmission::FailedReviewRecovery => {
            let Some(reservation) = coding_runs.try_reserve_attempt(&current_attempt.id) else {
                drop(attempt_guard);
                return Ok(CodingMessagePreparation::RecoveryAlreadyActive);
            };
            let CodingWsInMessage::GateResponse { gate_id, .. } = inbound else {
                unreachable!("failed review recovery guard only accepts gate responses");
            };
            #[cfg(test)]
            if let Some(probe) = probe {
                let _ = probe.reserved_tx.send(());
                let _ = probe.continue_rx.await;
            }
            let engine = CodingWorkspaceEngine::new(
                coding_store.clone(),
                GitWorkspaceService::new(),
                event_tx.clone(),
            );
            let updated = engine
                .recover_failed_code_review_for_attempt(&current_attempt, gate_id)
                .await
                .map_err(CodingMessagePreparationError::Recovery)?;
            drop(mutation_lease);
            drop(attempt_guard);
            Ok(CodingMessagePreparation::FailedReviewRecovery {
                attempt: updated,
                gate_id: gate_id.clone(),
                reservation,
            })
        }
    }
}
