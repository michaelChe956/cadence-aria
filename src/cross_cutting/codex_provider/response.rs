use serde_json::{Map, Value, json};

use crate::cross_cutting::approval_bridge::ChoiceDecision;
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::ChoiceAnswerData;

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
    let answer_entries = if decision.answers.is_empty() {
        vec![ChoiceAnswerData {
            question_id: question_id.to_string(),
            selected_option_ids: decision.selected_option_ids,
            free_text: decision.free_text,
        }]
    } else {
        decision.answers
    };
    let mut answer_map = Map::new();
    for answer in answer_entries {
        let mut answers = answer.selected_option_ids;
        if let Some(free_text) = answer.free_text.filter(|text| !text.trim().is_empty()) {
            answers.push(free_text);
        }
        answer_map.insert(answer.question_id, json!({ "answers": answers }));
    }
    peer.send(json!({
        "jsonrpc": "2.0",
        "id": rpc_id,
        "result": {
            "answers": answer_map,
        },
    }))
    .await
}
