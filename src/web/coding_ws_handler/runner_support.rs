use std::sync::Arc;

use tokio::sync::mpsc;

use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionStage, CodingProviderRole, CodingRoleRunStatus,
};
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::json_store::ProductStoreError;
use crate::product::models::ProviderName;
use crate::web::state::WebAppState;

use super::{
    CodingWsOutMessage, build_coding_session_state, emit_current_session_state,
    update_provider_selection,
};

pub(super) fn latest_analyst_role_run_evidence(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<String, CodingWorkspaceEngineError> {
    let run = coding_store
        .latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Rework,
            CodingProviderRole::Analyst,
        )?
        .ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream("analyst_retry_missing_evidence".to_string())
        })?;
    let evidence_ref = run
        .artifact_refs
        .iter()
        .rev()
        .find(|reference| reference.contains("analyst_evidence"))
        .cloned()
        .ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream("analyst_retry_missing_evidence".to_string())
        })?;
    coding_store
        .read_attempt_artifact_text(&attempt.id, &evidence_ref)
        .map_err(CodingWorkspaceEngineError::Store)
}

pub(super) fn testing_result_acceptance_pending_analyst(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<bool, CodingWorkspaceEngineError> {
    if attempt.stage != CodingExecutionStage::Testing {
        return Ok(false);
    }
    let Some(run) = coding_store.latest_role_run(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        CodingExecutionStage::Rework,
        CodingProviderRole::Analyst,
    )?
    else {
        return Ok(false);
    };
    Ok(run.status == CodingRoleRunStatus::Running
        && run.node_id.is_none()
        && run
            .artifact_refs
            .iter()
            .any(|reference| reference.contains("analyst_evidence")))
}

pub(super) async fn handle_pending_runner_commands(
    command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    coding_store: &CodingAttemptStore,
    engine: &CodingWorkspaceEngine,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    attempt: &CodingExecutionAttempt,
) -> Result<bool, CodingWorkspaceEngineError> {
    while let Ok(command) = command_rx.try_recv() {
        match command {
            CodingRunnerCommand::AbortAttempt => {
                let updated = engine
                    .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
                    .await?;
                emit_current_session_state(event_tx, coding_store, &updated).await?;
                return Ok(true);
            }
            CodingRunnerCommand::ProviderSelect { role, provider } => {
                let (updated, changed_role, changed_provider) =
                    update_provider_selection(coding_store, attempt, &role, provider)?;
                let _ = event_tx
                    .send(CodingWsOutMessage::CodingProviderConfigUpdated {
                        role: changed_role,
                        provider: changed_provider,
                    })
                    .await;
                let _ = event_tx
                    .send(build_coding_session_state(coding_store, updated)?)
                    .await;
            }
            CodingRunnerCommand::StageGateConfirm { .. } => {}
            CodingRunnerCommand::PermissionResponse { .. }
            | CodingRunnerCommand::ChoiceResponse { .. } => {}
        }
    }
    Ok(false)
}

pub(super) fn provider_for(
    state: &WebAppState,
    provider_name: &ProviderName,
    kind: &'static str,
) -> Result<Arc<dyn StreamingProviderAdapter>, CodingWorkspaceEngineError> {
    state.provider_registry.get(provider_name).ok_or_else(|| {
        CodingWorkspaceEngineError::Store(ProductStoreError::NotFound {
            kind,
            id: format!("{provider_name:?}"),
        })
    })
}
