use std::collections::HashMap;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderCompletion, ProviderEvent, ProviderStatus, ProviderToolCall,
    ProviderToolResult, StreamingProviderInput,
};

use super::{
    is_pi_terminal, parse_pi_failure, parse_pi_session_id, parse_pi_text_delta, parse_pi_tool_end,
    parse_pi_tool_start,
};

const PI_RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

type PendingResponses = HashMap<String, oneshot::Sender<Value>>;

pub(crate) async fn run_pi_session<W>(
    peer: JsonRpcPeer<W>,
    command_rx: mpsc::Receiver<ProviderCommand>,
    event_tx: mpsc::Sender<ProviderEvent>,
    input: StreamingProviderInput,
    cancel: CancellationToken,
) -> Result<(), ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let result = run_pi_session_inner(peer, command_rx, event_tx.clone(), input, cancel).await;
    if let Err(error) = &result {
        let _ = event_tx
            .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
            .await;
        let _ = event_tx
            .send(ProviderEvent::Failed {
                message: error.details.clone(),
            })
            .await;
    }
    result
}

async fn run_pi_session_inner<W>(
    peer: JsonRpcPeer<W>,
    mut command_rx: mpsc::Receiver<ProviderCommand>,
    event_tx: mpsc::Sender<ProviderEvent>,
    input: StreamingProviderInput,
    cancel: CancellationToken,
) -> Result<(), ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut pending_by_id = PendingResponses::new();
    let mut next_id = 1_u64;
    let mut full_output = String::new();

    let get_state = send_pi_command(
        &peer,
        &mut pending_by_id,
        &mut next_id,
        json!({ "type": "get_state" }),
    )
    .await?;
    let (get_state_response, _) = match await_pi_response(
        get_state,
        PiResponseWaitContext {
            peer: &peer,
            pending_by_id: &mut pending_by_id,
            event_tx: &event_tx,
            full_output: &mut full_output,
            command_rx: &mut command_rx,
            next_id: &mut next_id,
            cancel: &cancel,
        },
    )
    .await?
    {
        PiResponseWait::Response(response, settled_while_waiting) => {
            (response, settled_while_waiting)
        }
        PiResponseWait::Aborted => return Ok(()),
    };
    ensure_pi_success(&get_state_response)?;
    let session_id = parse_pi_session_id(&get_state_response).or_else(|| {
        input
            .resume_provider_session_id
            .clone()
            .filter(|id| !id.trim().is_empty())
    });

    let prompt = send_pi_command(
        &peer,
        &mut pending_by_id,
        &mut next_id,
        json!({ "type": "prompt", "message": input.prompt }),
    )
    .await?;
    let (prompt_response, settled_while_waiting) = match await_pi_response(
        prompt,
        PiResponseWaitContext {
            peer: &peer,
            pending_by_id: &mut pending_by_id,
            event_tx: &event_tx,
            full_output: &mut full_output,
            command_rx: &mut command_rx,
            next_id: &mut next_id,
            cancel: &cancel,
        },
    )
    .await?
    {
        PiResponseWait::Response(response, settled_while_waiting) => {
            (response, settled_while_waiting)
        }
        PiResponseWait::Aborted => return Ok(()),
    };
    ensure_pi_success(&prompt_response)?;
    send_event(
        &event_tx,
        ProviderEvent::StatusChanged(ProviderStatus::Running),
    )
    .await?;
    if settled_while_waiting {
        complete_pi_session(
            &event_tx,
            full_output,
            input.structured_output_contract.as_ref(),
            session_id.clone(),
        )
        .await?;
        return Ok(());
    }

    let timeout_secs = input.timeout_secs.max(1);
    let timeout = tokio::time::sleep(Duration::from_secs(timeout_secs));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            biased;
            _ = await_pi_abort(&mut command_rx, &cancel) => {
                abort_pi_session(&peer, &mut next_id, &event_tx).await?;
                return Ok(());
            }
            _ = &mut timeout => {
                return Err(ProviderAdapterError::timeout_with_details(
                    "Pi provider session timed out",
                    full_output,
                    String::new(),
                    timeout_secs.saturating_mul(1000),
                ));
            }
            incoming = peer.next_incoming() => {
                let incoming = incoming.ok_or_else(|| provider_error("Pi RPC stream ended before completion"))?;
                if dispatch_pi_response(&incoming, &mut pending_by_id) {
                    continue;
                }
                if let Some(error) = parse_pi_failure(&incoming) {
                    return Err(provider_error(error));
                }
                if is_pi_terminal(&incoming) {
                    complete_pi_session(
                        &event_tx,
                        full_output,
                        input.structured_output_contract.as_ref(),
                        session_id.clone(),
                    )
                    .await?;
                    return Ok(());
                }
                handle_pi_event(&incoming, &event_tx, &mut full_output).await?;
            }
        }
    }
}

async fn await_pi_abort(
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    cancel: &CancellationToken,
) {
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return,
            command = command_rx.recv(), if !command_rx.is_closed() => {
                if matches!(command, Some(ProviderCommand::Abort)) {
                    return;
                }
                // Pi is Auto-only and exposes no Aria approval or tool-result round trips.
            }
        }
    }
}

async fn send_pi_command<W>(
    peer: &JsonRpcPeer<W>,
    pending_by_id: &mut PendingResponses,
    next_id: &mut u64,
    mut command: Value,
) -> Result<oneshot::Receiver<Value>, ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let id = format!("pi-{}", *next_id);
    *next_id = next_id.saturating_add(1);
    let command_text = command.to_string();
    let object = command.as_object_mut().ok_or_else(|| {
        ProviderAdapterError::parse_error(
            "Pi RPC command must be a JSON object",
            command_text,
            String::new(),
        )
    })?;
    object.insert("id".to_string(), Value::String(id.clone()));

    let (response_tx, response_rx) = oneshot::channel();
    pending_by_id.insert(id.clone(), response_tx);
    if let Err(error) = peer.send(command).await {
        pending_by_id.remove(&id);
        return Err(error);
    }
    Ok(response_rx)
}

enum PiResponseWait {
    Response(Value, bool),
    Aborted,
}

struct PiResponseWaitContext<'a, W>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    peer: &'a JsonRpcPeer<W>,
    pending_by_id: &'a mut PendingResponses,
    event_tx: &'a mpsc::Sender<ProviderEvent>,
    full_output: &'a mut String,
    command_rx: &'a mut mpsc::Receiver<ProviderCommand>,
    next_id: &'a mut u64,
    cancel: &'a CancellationToken,
}

async fn await_pi_response<W>(
    mut response_rx: oneshot::Receiver<Value>,
    context: PiResponseWaitContext<'_, W>,
) -> Result<PiResponseWait, ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let timeout = tokio::time::sleep(PI_RPC_REQUEST_TIMEOUT);
    tokio::pin!(timeout);
    let mut settled_while_waiting = false;
    loop {
        tokio::select! {
            biased;
            _ = await_pi_abort(context.command_rx, context.cancel) => {
                abort_pi_session(context.peer, context.next_id, context.event_tx).await?;
                return Ok(PiResponseWait::Aborted);
            }
            _ = &mut timeout => return Err(ProviderAdapterError::timeout_with_details(
                "Pi RPC command timed out",
                context.full_output.clone(),
                String::new(),
                u64::try_from(PI_RPC_REQUEST_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
            )),
            response = &mut response_rx => {
                return response
                    .map(|response| PiResponseWait::Response(response, settled_while_waiting))
                    .map_err(|_| provider_error("Pi RPC response channel closed"));
            }
            incoming = context.peer.next_incoming() => {
                let Some(incoming) = incoming else {
                    if settled_while_waiting {
                        return Err(provider_error("Pi RPC response channel closed before command response"));
                    }
                    return Err(provider_error("Pi RPC stream ended while waiting for command response"));
                };
                if dispatch_pi_response(&incoming, context.pending_by_id) {
                    continue;
                }
                if let Some(error) = parse_pi_failure(&incoming) {
                    return Err(provider_error(error));
                }
                if is_pi_terminal(&incoming) {
                    settled_while_waiting = true;
                    continue;
                }
                handle_pi_event(&incoming, context.event_tx, context.full_output).await?;
            }
        }
    }
}

fn dispatch_pi_response(incoming: &Value, pending_by_id: &mut PendingResponses) -> bool {
    if incoming.get("type").and_then(Value::as_str) != Some("response") {
        return false;
    }
    let Some(id) = incoming.get("id").and_then(pi_id_key) else {
        return false;
    };
    let Some(response_tx) = pending_by_id.remove(&id) else {
        return false;
    };
    let _ = response_tx.send(incoming.clone());
    true
}

async fn handle_pi_event(
    incoming: &Value,
    event_tx: &mpsc::Sender<ProviderEvent>,
    full_output: &mut String,
) -> Result<(), ProviderAdapterError> {
    if let Some(content) = parse_pi_text_delta(incoming) {
        full_output.push_str(&content);
        send_event(event_tx, ProviderEvent::TextDelta { content }).await?;
        return Ok(());
    }
    if let Some(start) = parse_pi_tool_start(incoming) {
        send_event(
            event_tx,
            ProviderEvent::ToolCall(ProviderToolCall {
                id: start.tool_call_id,
                tool_name: start.tool_name,
                input: start.args,
            }),
        )
        .await?;
        return Ok(());
    }
    if let Some(end) = parse_pi_tool_end(incoming) {
        send_event(
            event_tx,
            ProviderEvent::ToolResult(ProviderToolResult {
                tool_use_id: end.tool_call_id,
                output: end.output,
                is_error: end.is_error,
            }),
        )
        .await?;
    }
    Ok(())
}

async fn complete_pi_session(
    event_tx: &mpsc::Sender<ProviderEvent>,
    full_output: String,
    contract: Option<&crate::cross_cutting::structured_output::StructuredOutputContract>,
    session_id: Option<String>,
) -> Result<(), ProviderAdapterError> {
    let completion = ProviderCompletion::from_output(full_output, contract, session_id);
    send_event(
        event_tx,
        ProviderEvent::StatusChanged(ProviderStatus::Completed),
    )
    .await?;
    send_event(event_tx, ProviderEvent::Completed(completion)).await
}

fn ensure_pi_success(response: &Value) -> Result<(), ProviderAdapterError> {
    if response.get("success").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    Err(provider_error(
        parse_pi_failure(response).unwrap_or_else(|| "Pi RPC command failed".to_string()),
    ))
}

async fn abort_pi_session<W>(
    peer: &JsonRpcPeer<W>,
    next_id: &mut u64,
    event_tx: &mpsc::Sender<ProviderEvent>,
) -> Result<(), ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let id = format!("pi-{}", *next_id);
    *next_id = next_id.saturating_add(1);
    peer.send(json!({ "id": id, "type": "abort" })).await?;
    send_event(
        event_tx,
        ProviderEvent::StatusChanged(ProviderStatus::Aborted),
    )
    .await
}

async fn send_event(
    event_tx: &mpsc::Sender<ProviderEvent>,
    event: ProviderEvent,
) -> Result<(), ProviderAdapterError> {
    event_tx
        .send(event)
        .await
        .map_err(|_| provider_error("Pi provider event receiver closed"))
}

fn pi_id_key(value: &Value) -> Option<String> {
    value
        .as_u64()
        .map(|id| id.to_string())
        .or_else(|| value.as_str().map(ToString::to_string))
}

fn provider_error(message: impl Into<String>) -> ProviderAdapterError {
    ProviderAdapterError::parse_error(message, String::new(), String::new())
}
