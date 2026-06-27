use std::collections::HashSet;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ChoiceRequestData, ChoiceRequestSource, ProviderEvent, ProviderExecutionEvent,
    ProviderExecutionEventKind, ProviderExecutionEventStatus, ProviderPermissionMode,
    ProviderStatus, RiskLevel, StreamingProviderInput,
};

use super::{
    CODEX_DEFAULT_SANDBOX_MODE, CODEX_RESUME_STALL_ERROR, CODEX_RESUME_STALL_TIMEOUT,
    CODEX_RPC_REQUEST_TIMEOUT, emit_request_user_input_protocol_error, is_turn_completed,
    parse_agent_message_text, parse_approval_request, parse_execution_event, parse_failure,
    parse_user_input_request, provider_error, send_provider_event, write_approval_response,
    write_user_input_response,
};

pub(crate) async fn run_codex_session<W>(
    peer: JsonRpcPeer<W>,
    bridge: ApprovalBridge,
    event_tx: mpsc::Sender<ProviderEvent>,
    input: StreamingProviderInput,
    cancel: CancellationToken,
) -> Result<(), ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let _ = peer
        .request_with_timeout(
            json!({
                "jsonrpc": "2.0",
                "method": "initialize",
                "params": {
                    "clientInfo": {
                        "name": "cadence-aria",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                },
            }),
            CODEX_RPC_REQUEST_TIMEOUT,
        )
        .await?;
    peer.send(json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {},
    }))
    .await?;

    let resume_session_id = input
        .resume_provider_session_id
        .as_deref()
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
        .map(ToString::to_string);

    let thread_id = if let Some(session_id) = resume_session_id.as_deref() {
        let resume_response = peer
            .request_with_timeout(
                json!({
                    "jsonrpc": "2.0",
                    "method": "thread/resume",
                    "params": {
                        "threadId": session_id,
                        "cwd": input.working_dir.clone(),
                        "approvalPolicy": match input.permission_mode {
                            ProviderPermissionMode::Auto => "never",
                            ProviderPermissionMode::Supervised => "on-request",
                        },
                        "sandbox": CODEX_DEFAULT_SANDBOX_MODE,
                    },
                }),
                CODEX_RPC_REQUEST_TIMEOUT,
            )
            .await?;
        resume_response
            .pointer("/thread/id")
            .or_else(|| resume_response.pointer("/id"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or_else(|| Some(session_id.to_string()))
    } else {
        let thread_response = peer
            .request_with_timeout(
                json!({
                    "jsonrpc": "2.0",
                    "method": "thread/start",
                    "params": {
                        "cwd": input.working_dir.clone(),
                        "approvalPolicy": match input.permission_mode {
                            ProviderPermissionMode::Auto => "never",
                            ProviderPermissionMode::Supervised => "on-request",
                        },
                        "sandbox": CODEX_DEFAULT_SANDBOX_MODE,
                    },
                }),
                CODEX_RPC_REQUEST_TIMEOUT,
            )
            .await?;
        thread_response
            .pointer("/thread/id")
            .or_else(|| thread_response.pointer("/id"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
    };
    let turn_thread_id = thread_id.clone().unwrap_or_default();

    let turn_response = peer
        .request_with_timeout(
            json!({
                "jsonrpc": "2.0",
                "method": "turn/start",
                "params": {
                    "threadId": turn_thread_id,
                    "input": [
                        {
                            "type": "text",
                            "text": input.prompt.clone(),
                        }
                    ],
                },
            }),
            CODEX_RPC_REQUEST_TIMEOUT,
        )
        .await?;
    let turn_id = turn_response
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .unwrap_or("turn")
        .to_string();
    send_provider_event(
        &event_tx,
        ProviderEvent::StatusChanged(ProviderStatus::Running),
        &cancel,
    )
    .await?;
    send_provider_event(
        &event_tx,
        ProviderEvent::Execution(ProviderExecutionEvent {
            event_id: format!("turn_{turn_id}"),
            kind: ProviderExecutionEventKind::Turn,
            status: ProviderExecutionEventStatus::Started,
            title: "Turn started".to_string(),
            detail: None,
            command: None,
            cwd: Some(input.working_dir.display().to_string()),
            output: None,
            exit_code: None,
        }),
        &cancel,
    )
    .await?;

    let mut full_output = String::new();
    let mut streamed_agent_message_items = HashSet::new();
    let timeout_secs = input.timeout_secs.max(1);
    let timeout = tokio::time::sleep(Duration::from_secs(timeout_secs));
    tokio::pin!(timeout);
    let resume_stall_timeout = tokio::time::sleep(CODEX_RESUME_STALL_TIMEOUT);
    tokio::pin!(resume_stall_timeout);
    let mut waiting_for_resume_progress = resume_session_id.is_some();
    loop {
        let incoming = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(provider_error("Codex provider cancelled"));
            }
            _ = &mut timeout => {
                return Err(ProviderAdapterError::timeout(
                    full_output.clone(),
                    String::new(),
                    timeout_secs.saturating_mul(1000),
                ));
            }
            _ = &mut resume_stall_timeout, if waiting_for_resume_progress => {
                let session_id = resume_session_id.as_deref().unwrap_or("unknown");
                return Err(provider_error(format!(
                    "{CODEX_RESUME_STALL_ERROR} for thread {session_id}"
                )));
            }
            incoming = peer.next_incoming() => incoming.ok_or_else(|| {
                provider_error("Codex app-server stream ended before completion")
            })?,
        };

        if let Some(message) = parse_agent_message_text(&incoming) {
            waiting_for_resume_progress = false;
            if message.completed && streamed_agent_message_items.contains(&message.item_id) {
                continue;
            }
            streamed_agent_message_items.insert(message.item_id);
            full_output.push_str(&message.content);
            send_provider_event(
                &event_tx,
                ProviderEvent::TextDelta {
                    content: message.content,
                },
                &cancel,
            )
            .await?;
            continue;
        }

        if let Some(event) = parse_execution_event(&incoming) {
            waiting_for_resume_progress = false;
            send_provider_event(&event_tx, ProviderEvent::Execution(event), &cancel).await?;
            continue;
        }

        if let Some(request) = parse_approval_request(&incoming) {
            waiting_for_resume_progress = false;
            let decision = bridge
                .request_tool(
                    &request.tool_name,
                    &request.description,
                    RiskLevel::High,
                    cancel.clone(),
                )
                .await?;
            write_approval_response(&peer, request.rpc_id, decision.approved).await?;
            continue;
        }

        if let Some(request) = parse_user_input_request(&incoming) {
            waiting_for_resume_progress = false;
            let decision = match bridge
                .request_choice(
                    ChoiceRequestData {
                        id: request.id,
                        prompt: request.prompt,
                        options: request.options,
                        allow_multiple: false,
                        allow_free_text: request.allow_free_text,
                        source: ChoiceRequestSource::RequestUserInput,
                    },
                    cancel.clone(),
                )
                .await
            {
                Ok(decision) => decision,
                Err(error) => {
                    emit_request_user_input_protocol_error(
                        &event_tx,
                        "choice bridge",
                        &request.question_id,
                        &error.details,
                    )
                    .await;
                    return Err(error);
                }
            };
            if let Err(error) =
                write_user_input_response(&peer, request.rpc_id, &request.question_id, decision)
                    .await
            {
                emit_request_user_input_protocol_error(
                    &event_tx,
                    "response write",
                    &request.question_id,
                    &error.details,
                )
                .await;
                return Err(error);
            }
            continue;
        }

        if is_turn_completed(&incoming) {
            send_provider_event(
                &event_tx,
                ProviderEvent::Execution(ProviderExecutionEvent {
                    event_id: format!("turn_{turn_id}"),
                    kind: ProviderExecutionEventKind::Turn,
                    status: ProviderExecutionEventStatus::Completed,
                    title: "Turn completed".to_string(),
                    detail: None,
                    command: None,
                    cwd: Some(input.working_dir.display().to_string()),
                    output: None,
                    exit_code: None,
                }),
                &cancel,
            )
            .await?;
            send_provider_event(
                &event_tx,
                ProviderEvent::StatusChanged(ProviderStatus::Completed),
                &cancel,
            )
            .await?;
            send_provider_event(
                &event_tx,
                ProviderEvent::Completed {
                    full_output,
                    provider_session_id: thread_id,
                },
                &cancel,
            )
            .await?;
            return Ok(());
        }

        if let Some(message) = parse_failure(&incoming) {
            return Err(provider_error(message));
        }
    }
}
