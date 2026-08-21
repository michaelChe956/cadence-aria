use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderCompletion, ProviderEvent, ProviderExecutionEvent,
    ProviderExecutionEventKind, ProviderExecutionEventStatus, ProviderStatus, ProviderToolCall,
    ProviderToolResult, RiskLevel, StreamingProviderInput,
};

use super::client_services::{KimiClientServiceDispatcher, kimi_client_capabilities};
use super::mcp_bundle::{KimiMcpInjection, SessionMethodDecision, resolve_session_method};
use super::parse::{
    KimiPermissionOption, KimiPromptResult, KimiSessionUpdate, Parsed, parse_message,
};

const KIMI_RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const KIMI_ABORT_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);
#[cfg(not(test))]
const KIMI_RESUME_STALL_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
const KIMI_RESUME_STALL_TIMEOUT: Duration = Duration::from_millis(100);
pub(crate) const KIMI_SESSION_ABORTED: &str = "Kimi provider session aborted";

#[derive(Debug, Clone, PartialEq, Eq)]
enum IncomingDisposition {
    Progress,
    NotProgress,
    RestartPrompt(String),
    Abort,
}

struct CommandRelayGuard(CancellationToken);

impl Drop for CommandRelayGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// MCP 兼容入口（测试与既有调用方使用）：等价于不带 bundle 的
/// `run_kimi_session_with_mcp`，保持 `mcpServers: []`。
#[cfg_attr(not(test), allow(dead_code))]
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
    run_kimi_session_inner(peer, command_rx, event_tx, input, None, cancel).await
}

/// 带 MCP 受控注入的会话入口：`mcpServers` 由经校验的 bundle 派生；
/// resume 时 digest 漂移则拒绝 session/load、转 session/new 并记录 superseded 审计。
pub(crate) async fn run_kimi_session_with_mcp<W>(
    peer: JsonRpcPeer<W>,
    command_rx: mpsc::Receiver<ProviderCommand>,
    event_tx: mpsc::Sender<ProviderEvent>,
    input: StreamingProviderInput,
    mcp_injection: Option<KimiMcpInjection>,
    cancel: CancellationToken,
) -> Result<(), ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    run_kimi_session_inner(peer, command_rx, event_tx, input, mcp_injection, cancel).await
}

async fn run_kimi_session_inner<W>(
    peer: JsonRpcPeer<W>,
    mut command_rx: mpsc::Receiver<ProviderCommand>,
    event_tx: mpsc::Sender<ProviderEvent>,
    input: StreamingProviderInput,
    mcp_injection: Option<KimiMcpInjection>,
    cancel: CancellationToken,
) -> Result<(), ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let bridge = Arc::new(ApprovalBridge::new(
        input.permission_mode.clone(),
        event_tx.clone(),
    ));
    let bridge_commands = bridge.command_sender();
    let command_abort = CancellationToken::new();
    let relay_abort = command_abort.clone();
    let relay_cancel = CancellationToken::new();
    let relay_stop = relay_cancel.clone();
    let _relay_guard = CommandRelayGuard(relay_cancel);
    tokio::spawn(async move {
        loop {
            let command = tokio::select! {
                _ = relay_stop.cancelled() => break,
                command = command_rx.recv() => command,
            };
            let Some(command) = command else {
                relay_abort.cancel();
                break;
            };
            let is_abort = matches!(
                command,
                crate::cross_cutting::streaming_provider::ProviderCommand::Abort
            );
            if bridge_commands.send(command).await.is_err() {
                relay_abort.cancel();
                break;
            }
            if is_abort {
                relay_abort.cancel();
                break;
            }
        }
    });

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
                "clientCapabilities": kimi_client_capabilities(),
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

    let resume_id = input
        .resume_provider_session_id
        .clone()
        .filter(|id| !id.trim().is_empty());
    // MCP 受控注入（tasks.md 6.1）：mcpServers 由经校验的 bundle 派生；
    // resume 时 digest 漂移 → 拒绝 session/load、启动新会话并标记旧会话 superseded。
    if let Some(bundle) = mcp_injection.as_ref().map(KimiMcpInjection::bundle) {
        for line in bundle.argv_audit_lines() {
            tracing::info!(target: "kimi_code_provider", audit = %line, "kimi MCP server injection");
        }
    }
    let decision = resolve_session_method(resume_id.as_deref(), mcp_injection.as_ref());
    let mut pending_superseded = None;
    let (session_method, session_request) = match decision {
        SessionMethodDecision::Load {
            session_id,
            mcp_servers,
        } => (
            "session/load",
            json!({
                "jsonrpc":"2.0", "id":2, "method":"session/load",
                "params":{"sessionId":session_id,"cwd":input.working_dir,"mcpServers":mcp_servers}
            }),
        ),
        SessionMethodDecision::New {
            superseded,
            mcp_servers,
        } => {
            if let Some(superseded) = superseded.as_ref() {
                tracing::warn!(
                    target: "kimi_code_provider",
                    old_session_id = %superseded.session_id,
                    frozen_digest = %superseded.frozen_digest,
                    actual_digest = %superseded.actual_digest,
                    superseded = true,
                    "kimi MCP bundle digest drifted on resume; rejecting session/load and starting a new session (REQ-ENV-04)"
                );
            }
            pending_superseded = superseded;
            (
                "session/new",
                json!({
                    "jsonrpc":"2.0", "id":2, "method":"session/new",
                    "params":{"cwd":input.working_dir,"mcpServers":mcp_servers}
                }),
            )
        }
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
        .or_else(|| resume_id.clone())
        .ok_or_else(|| {
            provider_error(format!(
                "Kimi ACP {session_method} response did not contain sessionId"
            ))
        })?;

    // digest 漂移 → 旧会话标记 superseded：在 ProviderEvent 通道发可消费的
    // Execution 事件（gateway/审计层后续可订阅消费，REQ-ENV-04）。tracing 保留。
    if let Some(superseded) = pending_superseded {
        let _ = event_tx
            .send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: format!("kimi_session_superseded_{}", superseded.session_id),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Aborted,
                title: "Kimi session superseded on resume (MCP bundle digest drift)".to_string(),
                detail: Some(format!(
                    "session/load rejected for session {} (frozen_digest={}, actual_digest={}); a new session was started",
                    superseded.session_id, superseded.frozen_digest, superseded.actual_digest
                )),
                command: None,
                cwd: Some(input.working_dir.display().to_string()),
                output: Some(
                    json!({
                        "superseded": true,
                        "old_session_id": superseded.session_id,
                        "frozen_digest": superseded.frozen_digest,
                        "actual_digest": superseded.actual_digest,
                        "new_session_id": session_id,
                        "new_session_started": true,
                    })
                    .to_string(),
                ),
                exit_code: None,
            }))
            .await;
    }

    let dispatcher = KimiClientServiceDispatcher::new(
        peer.clone(),
        session_id.clone(),
        input.working_dir.clone(),
        input.role.clone(),
        input.permission_mode.clone(),
        Arc::clone(&bridge),
        event_tx.clone(),
        cancel.clone(),
    );

    let mut next_prompt_id = 4_u64;
    let prompt = peer.request_with_timeout(
        session_prompt_request(&session_id, &input.prompt, 3),
        deadline.saturating_duration_since(Instant::now()),
    );
    tokio::pin!(prompt);
    let mut full_output = String::new();
    let mut tool_outputs = HashMap::<String, String>::new();
    let mut completed_tools = HashSet::<String>::new();
    let mut askuser_question_counts = HashMap::<String, usize>::new();
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
            _ = command_abort.cancelled() => {
                abort_kimi_session(&peer, &session_id, &event_tx).await;
                return Err(aborted_session_error());
            }
            _ = cancel.cancelled() => {
                abort_kimi_session(&peer, &session_id, &event_tx).await;
                return Err(aborted_session_error());
            }
            _ = &mut idle_timer => {
                abort_kimi_session(&peer, &session_id, &event_tx).await;
                return Err(aborted_session_error());
            }
            response = &mut prompt => {
                let response = match response {
                    Ok(response) => response,
                    Err(error) => {
                        let mut restarted_prompt = false;
                        while let Some(incoming) = peer.try_next_incoming().await {
                            match handle_incoming(
                                &peer,
                                &dispatcher,
                                incoming,
                                &bridge,
                                &event_tx,
                                &cancel,
                                &command_abort,
                                &mut full_output,
                                &mut tool_outputs,
                                &mut completed_tools,
                                &mut askuser_question_counts,
                            ).await? {
                                IncomingDisposition::Progress | IncomingDisposition::NotProgress => {}
                                IncomingDisposition::RestartPrompt(free_text) => {
                                    prompt.set(peer.request_with_timeout(
                                        session_prompt_request(&session_id, &free_text, next_prompt_id),
                                        deadline.saturating_duration_since(Instant::now()),
                                    ));
                                    next_prompt_id += 1;
                                    idle_deadline = Instant::now() + kimi_idle_timeout(timeout_secs, resume_id.is_some());
                                    restarted_prompt = true;
                                }
                                IncomingDisposition::Abort => {
                                    abort_kimi_session(&peer, &session_id, &event_tx).await;
                                    return Err(aborted_session_error());
                                }
                            }
                        }
                        if restarted_prompt {
                            continue;
                        }
                        return Err(error);
                    }
                };
                ensure_response_success(&response, "session/prompt")?;
                // JsonRpcPeer has already delivered this response but can still have earlier
                // session/update notifications buffered. Drain those updates before terminality.
                let mut restarted_prompt = false;
                while let Some(incoming) = peer.try_next_incoming().await {
                    match handle_incoming(
                        &peer,
                        &dispatcher,
                        incoming,
                        &bridge,
                        &event_tx,
                        &cancel,
                        &command_abort,
                        &mut full_output,
                        &mut tool_outputs,
                        &mut completed_tools,
                        &mut askuser_question_counts,
                    ).await? {
                        IncomingDisposition::Progress | IncomingDisposition::NotProgress => {}
                        IncomingDisposition::RestartPrompt(free_text) => {
                            prompt.set(peer.request_with_timeout(
                                session_prompt_request(&session_id, &free_text, next_prompt_id),
                                deadline.saturating_duration_since(Instant::now()),
                            ));
                            next_prompt_id += 1;
                            idle_deadline = Instant::now() + kimi_idle_timeout(timeout_secs, resume_id.is_some());
                            restarted_prompt = true;
                        }
                        IncomingDisposition::Abort => {
                            abort_kimi_session(&peer, &session_id, &event_tx).await;
                            return Err(aborted_session_error());
                        }
                    }
                }
                if restarted_prompt {
                    continue;
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
            incoming = peer.next_incoming() => {
                let Some(incoming) = incoming else {
                    return Err(provider_error("Kimi ACP stream ended before prompt completion"));
                };
                match handle_incoming(
                    &peer,
                    &dispatcher,
                    incoming,
                    &bridge,
                    &event_tx,
                    &cancel,
                    &command_abort,
                    &mut full_output,
                    &mut tool_outputs,
                    &mut completed_tools,
                    &mut askuser_question_counts,
                ).await? {
                    IncomingDisposition::Progress => {
                        idle_deadline = Instant::now() + kimi_idle_timeout(timeout_secs, resume_id.is_some());
                    }
                    IncomingDisposition::NotProgress => {}
                    IncomingDisposition::RestartPrompt(free_text) => {
                        prompt.set(peer.request_with_timeout(
                            session_prompt_request(&session_id, &free_text, next_prompt_id),
                            deadline.saturating_duration_since(Instant::now()),
                        ));
                        next_prompt_id += 1;
                        idle_deadline = Instant::now() + kimi_idle_timeout(timeout_secs, resume_id.is_some());
                    }
                    IncomingDisposition::Abort => {
                        abort_kimi_session(&peer, &session_id, &event_tx).await;
                        return Err(aborted_session_error());
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_incoming<W>(
    peer: &JsonRpcPeer<W>,
    dispatcher: &KimiClientServiceDispatcher<W>,
    incoming: Value,
    bridge: &Arc<ApprovalBridge>,
    event_tx: &mpsc::Sender<ProviderEvent>,
    cancel: &CancellationToken,
    command_abort: &CancellationToken,
    full_output: &mut String,
    tool_outputs: &mut HashMap<String, String>,
    completed_tools: &mut HashSet<String>,
    askuser_question_counts: &mut HashMap<String, usize>,
) -> Result<IncomingDisposition, ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    // Server→client client-service requests (terminal/*, fs/*) are dispatched
    // to concurrent tasks and never processed inline, so wait_for_exit cannot
    // block the read loop while kill/release arrive.
    if let (Some(method), Some(id)) = (
        incoming.get("method").and_then(Value::as_str),
        incoming.get("id").cloned(),
    ) {
        let params = incoming.get("params").cloned().unwrap_or(Value::Null);
        if dispatcher.dispatch(method, id, params) {
            return Ok(IncomingDisposition::NotProgress);
        }
    }

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
                    if title == "AskUserQuestion"
                        && let Some(questions) =
                            arguments.get("questions").and_then(Value::as_array)
                    {
                        askuser_question_counts.insert(tool_call_id.clone(), questions.len());
                    }
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
            if request.title == "AskUserQuestion" {
                if askuser_question_counts
                    .get(&request.tool_call_id)
                    .is_some_and(|count| *count > 1)
                {
                    let count = askuser_question_counts
                        .get(&request.tool_call_id)
                        .copied()
                        .unwrap_or(0);
                    return Err(provider_error(format!(
                        "KIMI_ASK_USER_QUESTION_MULTIPLE_QUESTIONS_UNSUPPORTED: AskUserQuestion tool call {} carries {} questions; ACP request_permission can answer one question per request. Ask Kimi to ask one question at a time.",
                        request.tool_call_id, count
                    )));
                }
                let choice_request = super::parse::ask_user_question_choice_request(&request);
                let decision = match bridge.request_choice(choice_request, cancel.clone()).await {
                    Ok(decision) => decision,
                    Err(_error) if command_abort.is_cancelled() || cancel.is_cancelled() => {
                        return Ok(IncomingDisposition::Abort);
                    }
                    Err(error) => return Err(error),
                };
                if let Some(free_text) = decision
                    .free_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    peer.send(json!({
                        "jsonrpc":"2.0", "id":request.request_id,
                        "result":{"outcome":{"outcome":"cancelled"}}
                    }))
                    .await?;
                    return Ok(IncomingDisposition::RestartPrompt(free_text.to_string()));
                }
                let Some(selected_option_id) = decision.selected_option_ids.first() else {
                    peer.send(json!({
                        "jsonrpc":"2.0", "id":request.request_id,
                        "result":{"outcome":{"outcome":"cancelled"}}
                    }))
                    .await?;
                    return Ok(IncomingDisposition::Progress);
                };
                peer.send(selected_option_response(
                    request.request_id,
                    selected_option_id,
                ))
                .await?;
                Ok(IncomingDisposition::Progress)
            } else {
                if !has_supported_permission_option(&request.options) {
                    peer.send(json!({
                        "jsonrpc":"2.0", "id":request.request_id,
                        "result":{"outcome":{"outcome":"cancelled"}}
                    }))
                    .await?;
                    return Err(provider_error(format!(
                        "Kimi ACP request_permission has no supported options (title: {})",
                        request.title
                    )));
                }
                let decision = match bridge
                    .request_tool(
                        &request.title,
                        &request.content_text,
                        RiskLevel::High,
                        cancel.clone(),
                    )
                    .await
                {
                    Ok(decision) => decision,
                    Err(_error) if command_abort.is_cancelled() || cancel.is_cancelled() => {
                        return Ok(IncomingDisposition::Abort);
                    }
                    Err(error) => return Err(error),
                };
                let Some(option) =
                    permission_option_for_request(&request.options, decision.approved)
                else {
                    peer.send(json!({
                        "jsonrpc":"2.0", "id":request.request_id,
                        "result":{"outcome":{"outcome":"cancelled"}}
                    }))
                    .await?;
                    return Err(provider_error(format!(
                        "Kimi ACP request_permission has no {} option (title: {})",
                        if decision.approved {
                            "allow_once"
                        } else {
                            "reject_once"
                        },
                        request.title
                    )));
                };
                peer.send(selected_option_response(
                    request.request_id,
                    &option.option_id,
                ))
                .await?;
                Ok(IncomingDisposition::Progress)
            }
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

fn session_prompt_request(session_id: &str, text: &str, id: u64) -> Value {
    json!({
        "jsonrpc":"2.0", "id":id, "method":"session/prompt",
        "params":{"sessionId":session_id,"prompt":[{"type":"text","text":text}]}
    })
}

fn selected_option_response(request_id: Value, option_id: &str) -> Value {
    json!({
        "jsonrpc":"2.0", "id":request_id,
        "result":{"outcome":{"outcome":"selected","optionId":option_id}}
    })
}

fn has_supported_permission_option(options: &[KimiPermissionOption]) -> bool {
    options.iter().any(|option| {
        matches!(
            option.kind.as_str(),
            "allow_once" | "reject_once" | "reject"
        )
    })
}

fn permission_option_for_request(
    options: &[KimiPermissionOption],
    approved: bool,
) -> Option<&KimiPermissionOption> {
    let kind = if approved {
        "allow_once"
    } else {
        "reject_once"
    };
    options
        .iter()
        .find(|option| option.kind == kind)
        .or_else(|| {
            (!approved)
                .then(|| options.iter().find(|option| option.kind == "reject"))
                .flatten()
        })
}

fn kimi_idle_timeout(timeout_secs: u64, resuming: bool) -> Duration {
    if resuming {
        KIMI_RESUME_STALL_TIMEOUT.min(Duration::from_secs(timeout_secs))
    } else {
        Duration::from_secs(timeout_secs)
    }
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
    let request = peer
        .request_with_timeout_discarding_incoming(payload, KIMI_RPC_REQUEST_TIMEOUT.min(remaining));
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
