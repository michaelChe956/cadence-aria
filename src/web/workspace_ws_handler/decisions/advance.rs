use super::*;
use crate::product::advance_store::AdvanceInput;

pub(crate) async fn handle_advance_from_handler(
    run_context: ProviderRunContext,
    outbound_tx: mpsc::Sender<OutboundControl>,
    command_id: String,
) {
    let outcome = {
        let mut engine = run_context.engine.lock().await;
        let project_id = engine.session().project_id.clone();
        let issue_id = engine.session().issue_id.clone();
        let plan_id = engine.session().entity_id.clone();
        engine
            .handle_advance(AdvanceInput {
                command_id: command_id.clone(),
                project_id,
                issue_id,
                plan_id,
            })
            .await
    };

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
