use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_workspace_engine::CodingWorkspaceEngineError;
use crate::web::coding_ws_handler::spawn_plan_amendment_runner_reserved;
use crate::web::state::CodingAttemptRunKey;

use super::*;

pub(crate) async fn activate_published_plan_amendment(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> Result<(), CodingWorkspaceEngineError> {
    let coding_store =
        CodingAttemptStore::new(ProductAppPaths::new(state.workspace_root.join(".aria")));
    let attempt = coding_store.get_attempt(project_id, issue_id, attempt_id)?;
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let _attempt_guard = state.coding_runs.lock_attempt(&attempt_key).await;
    if state.coding_runs.runner_count(&attempt_key) > 0 {
        return Ok(());
    }
    let event_tx = state.coding_sockets.sender(&attempt_key).ok_or_else(|| {
        CodingWorkspaceEngineError::ProviderStream(
            "plan_amendment_coding_socket_unavailable".to_string(),
        )
    })?;
    let attempt = coding_store.get_attempt(project_id, issue_id, attempt_id)?;
    let Some(reservation) = state.coding_runs.try_reserve_attempt(&attempt_key) else {
        if state.coding_runs.runner_count(&attempt_key) > 0 {
            return Ok(());
        }
        return Err(CodingWorkspaceEngineError::ProviderStream(
            "plan_amendment_runner_reservation_unavailable".to_string(),
        ));
    };
    let _command_tx = spawn_plan_amendment_runner_reserved(
        state.clone(),
        coding_store,
        event_tx,
        attempt,
        reservation,
    )?;
    Ok(())
}
