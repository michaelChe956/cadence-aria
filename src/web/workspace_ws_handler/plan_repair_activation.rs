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
    // REQ-WIGA-06：plan_amendment 激活不再要求存活 coding socket——零 socket
    // 也建 hub（零 fan-out 由 delivery ack 立即结算失败，观察层记 Unsent，
    // 重连/重复确认后按 durable 事实补投递）。hub 保活与清理规则见
    // coding_socket_registry.rs 的 hubs 注释：零 socket 激活后若 runner 退出
    // 且从未有 socket attach/detach，hub entry 与空转路由任务按 attempt
    // 有界滞留（无 CPU 消耗、无正确性影响）。
    let event_tx = state.coding_sockets.hub_sender(&attempt_key);
    let _attempt_guard = state.coding_runs.lock_attempt(&attempt_key).await;
    if state.coding_runs.runner_count(&attempt_key) > 0 {
        // 重复确认：runner 已在推进业务，只补投递 durable 未送达的 amendment。
        crate::web::coding_ws_handler::delivery_ack::spawn_undelivered_amendment_redelivery(
            coding_store,
            attempt,
            event_tx,
        );
        return Ok(());
    }
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
