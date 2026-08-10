use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderCompletion, ProviderEvent, ProviderStatus, ProviderToolCall,
    ProviderToolResult, StreamingProviderInput,
};

use super::parse::{KimiPromptResult, KimiSessionUpdate, Parsed, parse_message};

const KIMI_RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const KIMI_ABORT_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);
#[cfg(not(test))]
const KIMI_RESUME_STALL_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const KIMI_SESSION_ABORTED: &str = "Kimi provider session aborted";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IncomingDisposition {
    Progress,
    NotProgress,
}

pub(crate) async fn run_kimi_session<W>(
    peer: JsonRpcPeer<W>,
    command_rx: mpsc::Receiver<ProviderCommand>,
    event_tx: mpsc::Sender<ProviderEvent>,
    input: StreamingProviderInput,
    cancel: CancellationToken,
) -> Result<(), ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    run_kimi_session_inner(peer, command_rx, event_tx, input, cancel).await
}

async fn run_kimi_session_inner<W>(
    peer: JsonRpcPeer<W>,
    mut command_rx: mpsc::Receiver<ProviderCommand>,
    event_tx: mpsc::Sender<ProviderEvent>,
    input: StreamingProviderInput,
    cancel: CancellationToken,
) -> Result<(), ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let timeout_secs = input.timeout_secs.max(1);
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    let initialize = match request_control(
        &peer,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientCapabilities": {},
                "clientInfo": {"name": "cadence-aria", "version": env!("CARGO_PKG_VERSION")}
            }
        }),
        deadline,
        &cancel,
        &event_tx,
    )
    .await?
    {
        Some(response) => response,
        None => return Err(aborted_session_error()),
    };
    ensure_response_success(&initialize, "initialize")?;
    validate_initialize(&initialize, input.resume_provider_session_id.is_some())?;
    peer.send(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }))
    .await?;

    let permission_mode = match input.permission_mode {
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto => "auto",
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Supervised => "default",
    };
    let resume_id = input
        .resume_provider_session_id
        .clone()
        .filter(|id| !id.trim().is_empty());
    let session_method = if resume_id.is_some() {
        "session/load"
    } else {
        "session/new"
    };
    let session_request = if let Some(session_id) = resume_id.as_deref() {
        json!({
            "jsonrpc":"2.0", "id":2, "method":"session/load",
            "params":{"sessionId":session_id,"cwd":input.working_dir,"mcpServers":[],"permissionMode":permission_mode}
        })
    } else {
        json!({
            "jsonrpc":"2.0", "id":2, "method":"session/new",
            "params":{"cwd":input.working_dir,"mcpServers":[],"permissionMode":permission_mode}
        })
    };
    let session_response =
        match request_control(&peer, session_request, deadline, &cancel, &event_tx).await? {
            Some(response) => response,
            None => return Err(aborted_session_error()),
        };
    ensure_response_success(&session_response, session_method)?;
    let session_id = session_response
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| {
            provider_error(format!(
                "Kimi ACP {session_method} response did not contain sessionId"
            ))
        })?;

    let prompt = peer.request_with_timeout(
        json!({
            "jsonrpc":"2.0", "id":3, "method":"session/prompt",
            "params":{"sessionId":session_id,"prompt":[{"type":"text","text":input.prompt}]}
        }),
        KIMI_RPC_REQUEST_TIMEOUT.min(deadline.saturating_duration_since(Instant::now())),
    );
    tokio::pin!(prompt);
    let mut full_output = String::new();
    let mut tool_outputs = HashMap::<String, String>::new();
    let mut completed_tools = HashSet::<String>::new();
    let mut idle_deadline = Instant::now() + kimi_idle_timeout(timeout_secs, resume_id.is_some());
    let _ = event_tx
        .send(ProviderEvent::StatusChanged(ProviderStatus::Running))
        .await;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let idle_remaining = idle_deadline.saturating_duration_since(Instant::now());
        let wait = remaining.min(idle_remaining);
        if wait.is_zero() {
            abort_kimi_session(&peer, &session_id, &event_tx).await;
            return Err(aborted_session_error());
        }
        let idle_timer = tokio::time::sleep(wait);
        tokio::pin!(idle_timer);
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                abort_kimi_session(&peer, &session_id, &event_tx).await;
                return Err(aborted_session_error());
            }
            command = command_rx.recv() => {
                if matches!(command, Some(ProviderCommand::Abort) | None) {
                    abort_kimi_session(&peer, &session_id, &event_tx).await;
                    return Err(aborted_session_error());
                }
            }
            _ = &mut idle_timer => {
                abort_kimi_session(&peer, &session_id, &event_tx).await;
                return Err(aborted_session_error());
            }
            incoming = peer.next_incoming() => {
                let Some(incoming) = incoming else {
                    return Err(provider_error("Kimi ACP stream ended before prompt completion"));
                };
                let disposition = handle_incoming(
                    &peer, incoming, &event_tx, &mut full_output, &mut tool_outputs, &mut completed_tools,
                ).await?;
                if disposition == IncomingDisposition::Progress {
                    idle_deadline = Instant::now() + kimi_idle_timeout(timeout_secs, resume_id.is_some());
                }
            }
            response = &mut prompt => {
                let response = response?;
                ensure_response_success(&response, "session/prompt")?;
                // JsonRpcPeer has already delivered this response but can still have earlier
                // session/update notifications buffered. Drain those updates before terminality.
                while let Some(incoming) = peer.try_next_incoming().await {
                    let _ = handle_incoming(
                        &peer, incoming, &event_tx, &mut full_output, &mut tool_outputs, &mut completed_tools,
                    ).await?;
                }
                match parse_message(&json!({"jsonrpc":"2.0","id":3,"result":response})) {
                    Parsed::PromptResult(KimiPromptResult::StopReason(reason)) if reason == "end_turn" => {
                        let completion = ProviderCompletion::from_output(full_output, input.structured_output_contract.as_ref(), Some(session_id));
                        let _ = event_tx.send(ProviderEvent::StatusChanged(ProviderStatus::Completed)).await;
                        let _ = event_tx.send(ProviderEvent::Completed(completion)).await;
                        return Ok(());
                    }
                    Parsed::PromptResult(KimiPromptResult::StopReason(reason)) => {
                        return Err(provider_error(format!("Kimi prompt stopped with reason {reason}")));
                    }
                    Parsed::PromptResult(KimiPromptResult::Error(message)) => return Err(provider_error(message)),
                    _ => return Err(provider_error("Kimi ACP prompt returned an invalid response")),
                }
            }
        }
    }
}

async fn handle_incoming<W>(
    peer: &JsonRpcPeer<W>,
    incoming: Value,
    event_tx: &mpsc::Sender<ProviderEvent>,
    full_output: &mut String,
    tool_outputs: &mut HashMap<String, String>,
    completed_tools: &mut HashSet<String>,
) -> Result<IncomingDisposition, ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    match parse_message(&incoming) {
        Parsed::SessionUpdate(update) => {
            match update {
                KimiSessionUpdate::AgentMessageChunk { content } => {
                    full_output.push_str(&content);
                    event_tx
                        .send(ProviderEvent::TextDelta { content })
                        .await
                        .map_err(|_| provider_error("Kimi event receiver closed"))?;
                }
                KimiSessionUpdate::AgentThoughtChunk { content } => {
                    tracing::debug!(target: "kimi_code_provider", thought = %content, "Kimi agent thought chunk");
                }
                KimiSessionUpdate::ToolCall {
                    tool_call_id,
                    title,
                    status,
                    arguments,
                    ..
                } if status == "pending" => {
                    event_tx
                        .send(ProviderEvent::ToolCall(ProviderToolCall {
                            id: tool_call_id,
                            tool_name: title,
                            input: arguments,
                        }))
                        .await
                        .map_err(|_| provider_error("Kimi event receiver closed"))?;
                }
                KimiSessionUpdate::ToolCall { .. } => {}
                KimiSessionUpdate::ToolCallUpdate {
                    tool_call_id,
                    status,
                    content,
                } => {
                    append_tool_content(
                        tool_outputs.entry(tool_call_id.clone()).or_default(),
                        &content,
                    );
                    if (status == "completed" || status == "failed")
                        && completed_tools.insert(tool_call_id.clone())
                    {
                        event_tx
                            .send(ProviderEvent::ToolResult(ProviderToolResult {
                                tool_use_id: tool_call_id.clone(),
                                output: tool_outputs.remove(&tool_call_id).unwrap_or_default(),
                                is_error: status == "failed",
                            }))
                            .await
                            .map_err(|_| provider_error("Kimi event receiver closed"))?;
                    }
                }
                KimiSessionUpdate::SessionInfoUpdate { title } => {
                    tracing::debug!(target: "kimi_code_provider", %title, "Kimi session info update")
                }
                KimiSessionUpdate::UsageUpdate { used, size } => {
                    tracing::debug!(target: "kimi_code_provider", used, size, "Kimi usage update")
                }
                KimiSessionUpdate::AvailableCommandsUpdate => {}
            }
            Ok(IncomingDisposition::Progress)
        }
        Parsed::RequestPermission(request) => {
            // TODO(Task3): ApprovalBridge/ChoiceRequest ACP response handling.
            if request.options.iter().any(|option| {
                !matches!(
                    option.kind.as_str(),
                    "allow_once" | "allow_always" | "reject_once"
                )
            }) {
                let _ = peer
                    .send(json!({
                        "jsonrpc":"2.0", "id":request.request_id,
                        "result":{"outcome":"cancelled"}
                    }))
                    .await;
            }
            Err(provider_error(format!(
                "request_permission handling not configured until Task 3 (title: {})",
                request.title
            )))
        }
        Parsed::Unknown(method) => {
            if incoming.get("id").is_some() && incoming.get("method").is_some() {
                let _ = peer.send(json!({"jsonrpc":"2.0","id":incoming["id"],"error":{"code":-32601,"message":"Method not found"}})).await;
            } else {
                tracing::debug!(target: "kimi_code_provider", %method, "Ignoring unknown Kimi ACP notification");
            }
            Ok(IncomingDisposition::NotProgress)
        }
        Parsed::PromptResult(_) => Ok(IncomingDisposition::NotProgress),
    }
}

#[cfg(not(test))]
fn kimi_idle_timeout(timeout_secs: u64, resuming: bool) -> Duration {
    if resuming {
        KIMI_RESUME_STALL_TIMEOUT.min(Duration::from_secs(timeout_secs))
    } else {
        Duration::from_secs(timeout_secs)
    }
}

#[cfg(test)]
fn kimi_idle_timeout(timeout_secs: u64, _resuming: bool) -> Duration {
    Duration::from_secs(timeout_secs)
}

fn append_tool_content(accumulated: &mut String, content: &str) {
    if content.starts_with(accumulated.as_str()) {
        accumulated.clear();
    }
    accumulated.push_str(content);
}

fn aborted_session_error() -> ProviderAdapterError {
    provider_error(KIMI_SESSION_ABORTED)
}

async fn request_control<W>(
    peer: &JsonRpcPeer<W>,
    payload: Value,
    deadline: Instant,
    cancel: &CancellationToken,
    event_tx: &mpsc::Sender<ProviderEvent>,
) -> Result<Option<Value>, ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        emit_aborted(event_tx).await;
        return Ok(None);
    }
    let request = peer.request_with_timeout(payload, KIMI_RPC_REQUEST_TIMEOUT.min(remaining));
    tokio::pin!(request);
    tokio::select! {
        _ = cancel.cancelled() => {
            emit_aborted(event_tx).await;
            Ok(None)
        }
        result = &mut request => Ok(Some(result?)),
        _ = tokio::time::sleep(remaining) => {
            emit_aborted(event_tx).await;
            Ok(None)
        }
    }
}

async fn abort_kimi_session<W>(
    peer: &JsonRpcPeer<W>,
    session_id: &str,
    event_tx: &mpsc::Sender<ProviderEvent>,
) where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    // Cancellation is best-effort: a closed stdin must still be surfaced as Aborted,
    // and the spawning lifecycle will terminate the managed process group afterwards.
    let _ = peer
        .send(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session_id}}))
        .await;
    let drain_deadline = Instant::now() + KIMI_ABORT_DRAIN_TIMEOUT;
    while Instant::now() < drain_deadline {
        let remaining = drain_deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, peer.next_incoming()).await {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    emit_aborted(event_tx).await;
}

async fn emit_aborted(event_tx: &mpsc::Sender<ProviderEvent>) {
    let _ = event_tx
        .send(ProviderEvent::StatusChanged(ProviderStatus::Aborted))
        .await;
}

fn ensure_response_success(response: &Value, method: &str) -> Result<(), ProviderAdapterError> {
    let error = response.get("error").or_else(|| {
        response
            .get("code")
            .and_then(Value::as_i64)
            .map(|_| response)
    });
    if let Some(error) = error {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("ACP request failed");
        return Err(provider_error(format!(
            "Kimi ACP {method} failed: {message}"
        )));
    }
    Ok(())
}

fn validate_initialize(response: &Value, resume: bool) -> Result<(), ProviderAdapterError> {
    if response.get("protocolVersion").and_then(Value::as_u64) != Some(1) {
        return Err(provider_error("Kimi ACP protocolVersion 1 is required"));
    }
    if resume
        && (response
            .pointer("/agentCapabilities/loadSession")
            .and_then(Value::as_bool)
            != Some(true)
            || response
                .pointer("/agentCapabilities/sessionCapabilities/resume")
                .is_none())
    {
        return Err(provider_error("Kimi ACP does not support session resume"));
    }
    Ok(())
}

fn provider_error(message: impl Into<String>) -> ProviderAdapterError {
    ProviderAdapterError::parse_error(message, String::new(), String::new())
}
