use std::sync::Arc;

use tokio::sync::mpsc;

use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::json_store::ProductStoreError;
use crate::product::models::ProviderName;
use crate::web::provider_availability::host_real_workflow_ready;
use crate::web::state::WebAppState;
use crate::web::workspace_ws_handler::refresh_coding_runtime_revision_history;

use super::{
    CodingWsOutMessage, build_coding_session_state, emit_current_session_state,
    update_provider_selection,
};

pub(super) fn refresh_runtime_revision_history(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<(), CodingWorkspaceEngineError> {
    if attempt.scope != crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
        return Ok(());
    }
    refresh_coding_runtime_revision_history(app_paths, attempt).map_err(|error| {
        CodingWorkspaceEngineError::ProviderStream(format!(
            "runtime_revision_history_refresh_failed: {error}"
        ))
    })?;
    Ok(())
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
    if !state.test_provider_enabled {
        state
            .provider_gate
            .ensure_available(provider_name)
            .map_err(|error| {
                CodingWorkspaceEngineError::Store(ProductStoreError::Io(format!(
                    "{}: {}",
                    error.code(),
                    error
                )))
            })?;
        host_real_workflow_ready().map_err(|error| {
            CodingWorkspaceEngineError::Store(ProductStoreError::Io(format!(
                "{}: {}",
                error.code, error.message
            )))
        })?;
    }
    state.provider_registry.get(provider_name).ok_or_else(|| {
        CodingWorkspaceEngineError::Store(ProductStoreError::NotFound {
            kind,
            id: format!("{provider_name:?}"),
        })
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::web::runtime::WebRuntime;

    #[test]
    fn coding_provider_for_rejects_real_provider_when_health_is_unavailable() {
        let root = tempdir().expect("workspace");
        let state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        );

        let error = provider_for(&state, &ProviderName::Codex, "coding provider")
            .err()
            .expect("degraded initial health must reject real provider");

        assert!(error.to_string().contains("provider_unavailable"));
    }

    #[test]
    fn coding_provider_for_allows_fake_provider_from_test_registry() {
        let root = tempdir().expect("workspace");
        let state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        );

        assert!(provider_for(&state, &ProviderName::Fake, "coding provider").is_ok());
    }
}
