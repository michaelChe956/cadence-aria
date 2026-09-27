use super::*;
use crate::product::advance_store::AdvanceInput;

/// P2 Task 1：人工 WS Advance 改走共用 `advance_plan` 薄服务（与自动编排
/// 同一 `WorkspaceEngine::handle_advance`/journal 实现）；此处只映射原 WS
/// 帧（`map_advance_outcome`），不启动 coding runner。身份字段取 socket 的
/// immutable session 快照，engine 侧仍做活的身份校验。
pub(crate) async fn handle_advance_from_handler(
    app_state: crate::web::state::WebAppState,
    run_context: ProviderRunContext,
    outbound_tx: mpsc::Sender<OutboundControl>,
    command_id: String,
) {
    let record = run_context.session_record;
    let outcome = crate::web::advance_plan::advance_plan(
        &app_state,
        AdvanceInput {
            command_id: command_id.clone(),
            project_id: record.project_id.clone(),
            issue_id: record.issue_id.clone(),
            plan_id: record.entity_id.clone(),
        },
        crate::web::advance_plan::AdvancePlanOrigin::Manual,
    )
    .await;

    let message = match outcome {
        Ok(outcome) => crate::web::handlers::map_advance_outcome(command_id, outcome),
        Err(reason) => WsOutMessage::AdvanceRejected {
            command_id,
            code: "ADVANCE_HANDLER_FAILED".to_string(),
            reason,
        },
    };
    let _ = send_json_outbound(&outbound_tx, &message).await;
}
