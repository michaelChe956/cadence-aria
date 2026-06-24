use serde_json::{Map, Value, json};

use crate::cross_cutting::approval_bridge::ChoiceDecision;
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;

pub(crate) async fn write_approval_response<W>(
    peer: &JsonRpcPeer<W>,
    rpc_id: Value,
    approved: bool,
) -> Result<(), ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let decision = if approved { "accept" } else { "decline" };
    peer.send(json!({
        "jsonrpc": "2.0",
        "id": rpc_id,
        "result": {
            "decision": decision,
        },
    }))
    .await
}

pub(crate) async fn write_user_input_response<W>(
    peer: &JsonRpcPeer<W>,
    rpc_id: Value,
    question_id: &str,
    decision: ChoiceDecision,
) -> Result<(), ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut answers = decision.selected_option_ids;
    if let Some(free_text) = decision.free_text.filter(|text| !text.trim().is_empty()) {
        answers.push(free_text);
    }
    let mut answer_map = Map::new();
    answer_map.insert(question_id.to_string(), json!({ "answers": answers }));
    peer.send(json!({
        "jsonrpc": "2.0",
        "id": rpc_id,
        "result": {
            "answers": answer_map,
        },
    }))
    .await
}
