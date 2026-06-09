use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::{ApprovalBridge, ChoiceDecision};
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::process_manager::ProcessManager;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestData, ChoiceRequestSource, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, ProviderSession, ProviderStatus, RiskLevel, StreamChunk,
    StreamingProviderAdapter, StreamingProviderInput,
};
use crate::protocol::contracts::AdapterInput;

#[derive(Debug, Clone)]
pub struct CodexProvider {
    command: PathBuf,
}

const CODEX_RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const CODEX_RESUME_STALL_ERROR: &str = "Codex resume stalled before provider progress";
#[cfg(not(test))]
const CODEX_RESUME_STALL_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
const CODEX_RESUME_STALL_TIMEOUT: Duration = Duration::from_millis(100);

impl CodexProvider {
    pub fn new(command: PathBuf) -> Self {
        Self { command }
    }

    fn build_args(&self) -> Vec<String> {
        vec![
            "app-server".to_string(),
            "--enable".to_string(),
            "default_mode_request_user_input".to_string(),
        ]
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CodexProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let args = self.build_args();
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let command = self.command.to_string_lossy().to_string();
        let process = ProcessManager::spawn(
            &command,
            &arg_refs,
            &input.working_dir,
            &input.env_vars,
            cancel.clone(),
        )
        .await?;

        let peer = JsonRpcPeer::new(process.stdout, process.stdin);
        let stderr = process.stderr;
        let mut child = process.child;
        let (event_tx, event_rx) = mpsc::channel(32);
        let bridge = ApprovalBridge::new(input.permission_mode.clone(), event_tx.clone());
        let commands = bridge.command_sender();
        let _ = event_tx
            .send(ProviderEvent::StatusChanged(ProviderStatus::Starting))
            .await;
        let _ = event_tx
            .send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "provider".to_string(),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Started,
                title: "Codex provider started".to_string(),
                detail: None,
                command: None,
                cwd: Some(input.working_dir.display().to_string()),
                output: None,
                exit_code: None,
            }))
            .await;

        tokio::spawn(async move {
            let stderr_output = Arc::new(Mutex::new(String::new()));
            let stderr_output_for_task = Arc::clone(&stderr_output);
            let stderr_task = tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut output = stderr_output_for_task.lock().await;
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&line);
                }
            });

            let result =
                run_codex_session(peer, bridge, event_tx.clone(), input, cancel.clone()).await;
            if result.is_err() {
                let _ = child.start_kill();
            }
            let status = child.wait().await;
            let _ = stderr_task.await;
            if let Err(error) = result {
                let stderr = combine_stderr(stderr_output.lock().await.clone(), error.stderr);
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "provider".to_string(),
                        kind: ProviderExecutionEventKind::Provider,
                        status: ProviderExecutionEventStatus::Failed,
                        title: "Codex provider failed".to_string(),
                        detail: Some(error.details.clone()),
                        command: None,
                        cwd: None,
                        output: if stderr.trim().is_empty() {
                            None
                        } else {
                            Some(stderr.clone())
                        },
                        exit_code: None,
                    }))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Failed {
                        message: format_codex_failure(error.details, status, stderr),
                    })
                    .await;
            }
        });

        Ok(ProviderSession {
            events: event_rx,
            commands,
        })
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let working_dir = input.worktree_path.as_ref().map(PathBuf::from).unwrap_or(
            std::env::current_dir().map_err(|error| {
                ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
            })?,
        );
        let provider_input = StreamingProviderInput {
            provider_type: input.provider_type.clone(),
            role: input.role.clone(),
            prompt: input.prompt.clone(),
            working_dir,
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            env_vars: BTreeMap::new(),
            timeout_secs: input.timeout,
        };
        let bridge_cancel = cancel.clone();
        let mut session = self.start(provider_input, cancel).await?;
        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
            let _commands = session.commands;
            loop {
                let event = tokio::select! {
                    _ = bridge_cancel.cancelled() => return,
                    event = session.events.recv() => match event {
                        Some(event) => event,
                        None => return,
                    },
                };
                let chunk = match event {
                    ProviderEvent::TextDelta { content } => StreamChunk::Text(content),
                    ProviderEvent::Completed { full_output, .. } => {
                        StreamChunk::Done { full_output }
                    }
                    ProviderEvent::Failed { message } => StreamChunk::Error(message),
                    ProviderEvent::ProtocolError { message, .. } => StreamChunk::Error(message),
                    ProviderEvent::PermissionTimeout { permission_id } => {
                        StreamChunk::Error(format!("Permission request {permission_id} timed out"))
                    }
                    ProviderEvent::PermissionRequest(_)
                    | ProviderEvent::ChoiceRequest(_)
                    | ProviderEvent::StatusChanged(_)
                    | ProviderEvent::Execution(_)
                    | ProviderEvent::ToolCall(_)
                    | ProviderEvent::ToolResult(_) => {
                        continue;
                    }
                };
                tokio::select! {
                    _ = bridge_cancel.cancelled() => return,
                    send_result = tx.send(chunk) => {
                        if send_result.is_err() {
                            return;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

#[derive(Debug, Clone)]
struct CodexApprovalRequest {
    rpc_id: Value,
    tool_name: String,
    description: String,
}

#[derive(Debug, Clone)]
struct CodexUserInputRequest {
    rpc_id: Value,
    id: String,
    question_id: String,
    prompt: String,
    options: Vec<ChoiceOptionData>,
    allow_free_text: bool,
}

#[derive(Debug, Clone)]
struct AgentMessageText {
    item_id: String,
    content: String,
    completed: bool,
}

async fn run_codex_session<W>(
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
            let decision = bridge
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
                .await?;
            write_user_input_response(&peer, request.rpc_id, &request.question_id, decision)
                .await?;
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

fn parse_agent_message_text(value: &Value) -> Option<AgentMessageText> {
    if value.get("method")?.as_str()? == "item/agentMessage/delta" {
        let content = value
            .pointer("/params/delta")
            .and_then(Value::as_str)
            .filter(|content| !content.is_empty())
            .map(ToString::to_string)?;
        return Some(AgentMessageText {
            item_id: value
                .pointer("/params/itemId")
                .and_then(Value::as_str)
                .unwrap_or("agent_message")
                .to_string(),
            content,
            completed: false,
        });
    }

    if value.get("method")?.as_str()? == "item/completed" {
        let item = value.pointer("/params/item")?;
        if !matches!(
            item.get("type").and_then(Value::as_str),
            Some("agentMessage" | "agent_message")
        ) {
            return None;
        }
        let content = agent_message_completed_text(item)?;
        return Some(AgentMessageText {
            item_id: item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("agent_message")
                .to_string(),
            content,
            completed: true,
        });
    }

    if value.get("method")?.as_str()? != "codex/event" {
        return None;
    }
    let msg = value.get("params")?.get("msg")?;
    if msg.get("type")?.as_str()? != "item_completed" {
        return None;
    }
    let item = msg.get("item")?;
    if item.get("type")?.as_str()? != "message" || item.get("role")?.as_str()? != "assistant" {
        return None;
    }
    let content = item
        .get("content")?
        .as_array()?
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");

    (!content.is_empty()).then(|| AgentMessageText {
        item_id: item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("legacy_message")
            .to_string(),
        content,
        completed: true,
    })
}

fn agent_message_completed_text(item: &Value) -> Option<String> {
    if let Some(text) = item
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }

    let content = item.get("content")?.as_array()?;
    let text = content
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    (!text.is_empty()).then_some(text)
}

fn parse_execution_event(value: &Value) -> Option<ProviderExecutionEvent> {
    let method = value.get("method")?.as_str()?;
    if method != "item/started" && method != "item/completed" {
        return None;
    }

    let item = value.pointer("/params/item")?;
    if !is_command_execution_item(item) {
        return None;
    }

    let item_id = item.get("id").and_then(Value::as_str).unwrap_or("command");
    let command = command_description(item);
    let cwd = item
        .get("cwd")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/params/cwd").and_then(Value::as_str))
        .map(ToString::to_string);
    let exit_code = item
        .get("exitCode")
        .or_else(|| item.get("exit_code"))
        .and_then(Value::as_i64)
        .and_then(|code| i32::try_from(code).ok());
    let output = command_output(item);

    if method == "item/started" {
        return Some(ProviderExecutionEvent {
            event_id: format!("command_{item_id}"),
            kind: ProviderExecutionEventKind::Command,
            status: ProviderExecutionEventStatus::Started,
            title: "Command started".to_string(),
            detail: None,
            command,
            cwd,
            output: None,
            exit_code: None,
        });
    }

    Some(ProviderExecutionEvent {
        event_id: format!("command_{item_id}"),
        kind: ProviderExecutionEventKind::Command,
        status: if exit_code.is_some_and(|code| code != 0) {
            ProviderExecutionEventStatus::Failed
        } else {
            ProviderExecutionEventStatus::Completed
        },
        title: if exit_code.is_some_and(|code| code != 0) {
            "Command failed".to_string()
        } else {
            "Command completed".to_string()
        },
        detail: exit_code.map(|code| format!("exit code {code}")),
        command,
        cwd,
        output,
        exit_code,
    })
}

fn is_command_execution_item(item: &Value) -> bool {
    matches!(
        item.get("type").and_then(Value::as_str),
        Some("commandExecution" | "command_execution")
    )
}

fn command_output(item: &Value) -> Option<String> {
    ["aggregatedOutput", "aggregated_output", "output", "stdout"]
        .iter()
        .find_map(|field| item.get(field).and_then(Value::as_str))
        .filter(|output| !output.is_empty())
        .map(ToString::to_string)
}

fn parse_approval_request(value: &Value) -> Option<CodexApprovalRequest> {
    let method = value.get("method")?.as_str()?;
    if method == "codex/server_request" {
        let params = value.get("params")?;
        if params.get("type")?.as_str()? != "command_execution_request_approval" {
            return None;
        }
        let request_params = params.get("params").unwrap_or(params);
        return Some(CodexApprovalRequest {
            rpc_id: value
                .get("id")
                .cloned()
                .or_else(|| params.get("request_id").cloned())
                .unwrap_or(Value::Null),
            tool_name: "command".to_string(),
            description: command_description(request_params)
                .unwrap_or_else(|| "Codex command approval request".to_string()),
        });
    }

    if method == "item/commandExecution/requestApproval" {
        let params = value.get("params").unwrap_or(value);
        return Some(CodexApprovalRequest {
            rpc_id: value.get("id").cloned().unwrap_or(Value::Null),
            tool_name: "command".to_string(),
            description: command_description(params)
                .unwrap_or_else(|| "Codex command approval request".to_string()),
        });
    }

    None
}

fn parse_user_input_request(value: &Value) -> Option<CodexUserInputRequest> {
    if value.get("method")?.as_str()? != "item/tool/requestUserInput" {
        return None;
    }

    let rpc_id = value.get("id")?.clone();
    let id = rpc_id_string(&rpc_id)?;
    let question = value
        .pointer("/params/questions")
        .and_then(Value::as_array)?
        .first()?;
    let question_id = question.get("id").and_then(Value::as_str)?.to_string();
    let question_text = question
        .get("question")
        .and_then(Value::as_str)
        .or_else(|| question.get("header").and_then(Value::as_str))?
        .to_string();
    let options = question
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    let label = option.get("label").and_then(Value::as_str)?;
                    Some(ChoiceOptionData {
                        id: label.to_string(),
                        label: label.to_string(),
                        description: option
                            .get("description")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let allow_free_text = options.is_empty()
        || question
            .get("isOther")
            .and_then(Value::as_bool)
            .unwrap_or(false);

    Some(CodexUserInputRequest {
        rpc_id,
        id,
        question_id,
        prompt: question_text,
        options,
        allow_free_text,
    })
}

fn command_description(params: &Value) -> Option<String> {
    let command = params.get("command")?;
    if let Some(command) = command.as_str() {
        return Some(command.to_string());
    }
    let args = command.as_array()?;
    let text = args
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() { None } else { Some(text) }
}

async fn write_approval_response<W>(
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

async fn write_user_input_response<W>(
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

fn rpc_id_string(value: &Value) -> Option<String> {
    value
        .as_u64()
        .map(|id| id.to_string())
        .or_else(|| value.as_str().map(ToString::to_string))
}

fn is_turn_completed(value: &Value) -> bool {
    value.get("method").and_then(Value::as_str) == Some("turn/completed")
        || value
            .pointer("/params/msg/type")
            .and_then(Value::as_str)
            .is_some_and(|event_type| event_type == "turn_completed")
}

fn parse_failure(value: &Value) -> Option<String> {
    let event_type = value.pointer("/params/msg/type").and_then(Value::as_str)?;
    if event_type == "turn_failed" || event_type == "error" {
        return value
            .pointer("/params/msg/message")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/params/msg/error").and_then(Value::as_str))
            .map(ToString::to_string)
            .or_else(|| Some("Codex turn failed".to_string()));
    }
    None
}

async fn send_provider_event(
    event_tx: &mpsc::Sender<ProviderEvent>,
    event: ProviderEvent,
    cancel: &CancellationToken,
) -> Result<(), ProviderAdapterError> {
    tokio::select! {
        _ = cancel.cancelled() => Err(provider_error("Codex provider cancelled")),
        result = event_tx.send(event) => result.map_err(|_| {
            provider_error("provider event receiver closed")
        }),
    }
}

fn provider_error(message: impl Into<String>) -> ProviderAdapterError {
    ProviderAdapterError::parse_error(message, String::new(), String::new())
}

fn combine_stderr(process_stderr: String, error_stderr: String) -> String {
    match (process_stderr.trim(), error_stderr.trim()) {
        ("", "") => String::new(),
        (process, "") => process.to_string(),
        ("", write_error) => write_error.to_string(),
        (process, write_error) => format!("{process}\n{write_error}"),
    }
}

fn format_codex_failure(
    details: String,
    status: Result<ExitStatus, std::io::Error>,
    stderr: String,
) -> String {
    let status_text = match status {
        Ok(status) => format!("exit status: {status}"),
        Err(error) => format!("failed to wait for process: {error}"),
    };
    if stderr.trim().is_empty() {
        format!("{details} ({status_text})")
    } else {
        format!("{details} ({status_text}); stderr: {}", stderr.trim())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::cross_cutting::streaming_provider::{
        ProviderCommand, ProviderEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
        ProviderPermissionMode, StreamingProviderAdapter, StreamingProviderInput,
    };
    use crate::protocol::contracts::{AdapterRole, ProviderType};

    use super::CodexProvider;

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    fn executable_fixture(relative_path: &str) -> PathBuf {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = std::fs::metadata(&path)
                .unwrap_or_else(|error| panic!("fixture metadata {}: {error}", path.display()))
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions)
                .unwrap_or_else(|error| panic!("chmod fixture {}: {error}", path.display()));
        }
        path
    }

    fn streaming_input(
        provider_type: ProviderType,
        permission_mode: ProviderPermissionMode,
    ) -> StreamingProviderInput {
        StreamingProviderInput {
            provider_type,
            role: AdapterRole::Orchestrator,
            prompt: "fixture prompt".to_string(),
            working_dir: std::env::current_dir().unwrap(),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode,
            env_vars: BTreeMap::new(),
            timeout_secs: 60,
        }
    }

    async fn recv_completed(events: &mut mpsc::Receiver<ProviderEvent>) -> String {
        loop {
            match tokio::time::timeout(TEST_TIMEOUT, events.recv())
                .await
                .expect("provider should emit completion")
                .expect("provider event channel should stay open")
            {
                ProviderEvent::Completed { full_output, .. } => return full_output,
                ProviderEvent::StatusChanged(_)
                | ProviderEvent::Execution(_)
                | ProviderEvent::TextDelta { .. }
                | ProviderEvent::PermissionRequest(_)
                | ProviderEvent::ChoiceRequest(_)
                | ProviderEvent::ToolCall(_)
                | ProviderEvent::ToolResult(_) => {}
                ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
                ProviderEvent::ProtocolError { message, .. } => {
                    panic!("provider protocol error: {message}")
                }
                ProviderEvent::PermissionTimeout { permission_id } => {
                    panic!("provider permission timed out: {permission_id}")
                }
            }
        }
    }

    #[test]
    fn codex_provider_enables_default_mode_request_user_input_feature() {
        let provider = CodexProvider::new(PathBuf::from("codex"));

        assert_eq!(
            provider.build_args(),
            vec![
                "app-server".to_string(),
                "--enable".to_string(),
                "default_mode_request_user_input".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn codex_resume_uses_existing_thread_without_starting_new_thread() {
        let fixture =
            executable_fixture("tests/fixtures/provider/codex_app_server_resume_fixture.sh");
        let provider = CodexProvider::new(fixture);
        let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
        input.resume_provider_session_id = Some("codex-thread-123".to_string());
        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .unwrap();

        let completed = recv_completed(&mut session.events).await;

        assert_eq!(completed, "resumed done");
    }

    #[tokio::test]
    async fn codex_thread_start_creates_persistent_thread_for_later_resume() {
        let fixture = executable_fixture(
            "tests/fixtures/provider/codex_app_server_persistent_thread_fixture.sh",
        );
        let provider = CodexProvider::new(fixture);
        let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .unwrap();

        let completed = recv_completed(&mut session.events).await;

        assert_eq!(completed, "persistent thread done");
    }

    #[tokio::test]
    async fn codex_provider_bridges_permission_and_completes() {
        let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_fixture.sh");
        let provider = CodexProvider::new(fixture);
        let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised);
        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .unwrap();

        let mut saw_text = false;
        let permission_id = loop {
            match session.events.recv().await.unwrap() {
                ProviderEvent::TextDelta { content } => {
                    saw_text = content.contains("Codex fixture chunk");
                }
                ProviderEvent::PermissionRequest(data) => break data.id,
                ProviderEvent::StatusChanged(_)
                | ProviderEvent::Execution(_)
                | ProviderEvent::ChoiceRequest(_)
                | ProviderEvent::ToolCall(_)
                | ProviderEvent::ToolResult(_) => {}
                other => panic!("unexpected event before permission: {other:?}"),
            }
        };
        assert!(saw_text);

        session
            .commands
            .send(ProviderCommand::PermissionResponse {
                id: permission_id,
                approved: true,
                reason: None,
            })
            .await
            .unwrap();

        let completed = recv_completed(&mut session.events).await;
        assert_eq!(completed, "Codex fixture chunk");
    }

    #[tokio::test]
    async fn codex_provider_handles_current_app_server_protocol_and_agent_message_delta() {
        let fixture =
            executable_fixture("tests/fixtures/provider/codex_app_server_current_fixture.sh");
        let provider = CodexProvider::new(fixture);
        let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .unwrap();

        let completed = recv_completed(&mut session.events).await;

        assert!(completed.contains("# Story Spec"));
        assert!(completed.contains("## 功能需求"));
        assert!(completed.contains("## 成功标准"));
    }

    #[tokio::test]
    async fn codex_provider_responds_to_current_command_approval_with_json_rpc_result() {
        let fixture = executable_fixture(
            "tests/fixtures/provider/codex_app_server_current_permission_fixture.sh",
        );
        let provider = CodexProvider::new(fixture);
        let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised);
        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .unwrap();

        let permission = loop {
            match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
                .await
                .expect("provider should emit current command approval")
                .expect("provider event channel should stay open")
            {
                ProviderEvent::PermissionRequest(request) => break request,
                ProviderEvent::StatusChanged(_)
                | ProviderEvent::Execution(_)
                | ProviderEvent::TextDelta { .. }
                | ProviderEvent::ChoiceRequest(_)
                | ProviderEvent::ToolCall(_)
                | ProviderEvent::ToolResult(_) => {}
                ProviderEvent::Completed { full_output, .. } => {
                    panic!("provider completed before permission request: {full_output}")
                }
                ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
                ProviderEvent::ProtocolError { message, .. } => {
                    panic!("provider protocol error: {message}")
                }
                ProviderEvent::PermissionTimeout { permission_id } => {
                    panic!("provider permission timed out: {permission_id}")
                }
            }
        };
        assert_eq!(permission.tool_name, "command");
        assert!(permission.description.contains("pnpm -C web install"));

        session
            .commands
            .send(ProviderCommand::PermissionResponse {
                id: permission.id,
                approved: true,
                reason: None,
            })
            .await
            .unwrap();

        let completed = recv_completed(&mut session.events).await;
        assert_eq!(completed, "permission accepted");
    }

    #[tokio::test]
    async fn codex_provider_streams_completed_only_agent_messages() {
        let fixture = executable_fixture(
            "tests/fixtures/provider/codex_app_server_completed_only_fixture.sh",
        );
        let provider = CodexProvider::new(fixture);
        let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .unwrap();

        let mut saw_text_delta = false;
        let completed = loop {
            match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
                .await
                .expect("provider should emit completed-only text")
                .expect("provider event channel should stay open")
            {
                ProviderEvent::TextDelta { content } => {
                    assert_eq!(content, "Codex completed-only chunk");
                    saw_text_delta = true;
                }
                ProviderEvent::Completed { full_output, .. } => break full_output,
                ProviderEvent::StatusChanged(_)
                | ProviderEvent::Execution(_)
                | ProviderEvent::PermissionRequest(_)
                | ProviderEvent::ChoiceRequest(_)
                | ProviderEvent::ToolCall(_)
                | ProviderEvent::ToolResult(_) => {}
                ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
                ProviderEvent::ProtocolError { message, .. } => {
                    panic!("provider protocol error: {message}")
                }
                ProviderEvent::PermissionTimeout { permission_id } => {
                    panic!("provider permission timed out: {permission_id}")
                }
            }
        };

        assert!(saw_text_delta);
        assert_eq!(completed, "Codex completed-only chunk");
    }

    #[tokio::test]
    async fn codex_provider_bridges_request_user_input_and_completes() {
        let fixture =
            executable_fixture("tests/fixtures/provider/codex_app_server_user_input_fixture.sh");
        let provider = CodexProvider::new(fixture);
        let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .unwrap();

        let choice = loop {
            match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
                .await
                .expect("provider should emit a choice request")
                .expect("provider event channel should stay open")
            {
                ProviderEvent::ChoiceRequest(request) => break request,
                ProviderEvent::StatusChanged(_)
                | ProviderEvent::Execution(_)
                | ProviderEvent::TextDelta { .. }
                | ProviderEvent::PermissionRequest(_)
                | ProviderEvent::ToolCall(_)
                | ProviderEvent::ToolResult(_) => {}
                ProviderEvent::Completed { full_output, .. } => {
                    panic!("provider completed before choice request: {full_output}")
                }
                ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
                ProviderEvent::ProtocolError { message, .. } => {
                    panic!("provider protocol error: {message}")
                }
                ProviderEvent::PermissionTimeout { permission_id } => {
                    panic!("provider permission timed out: {permission_id}")
                }
            }
        };

        assert_eq!(choice.id, "77");
        assert_eq!(choice.prompt, "请选择复杂度");
        assert_eq!(choice.options[0].id, "O(n)");
        assert_eq!(choice.options[0].description.as_deref(), Some("线性复杂度"));

        session
            .commands
            .send(ProviderCommand::ChoiceResponse {
                id: choice.id,
                selected_option_ids: vec!["O(n)".to_string()],
                free_text: None,
            })
            .await
            .unwrap();

        let completed = recv_completed(&mut session.events).await;
        assert_eq!(completed, "Codex received O(n)");
    }

    #[tokio::test]
    async fn codex_provider_emits_command_execution_events_from_current_protocol() {
        let fixture =
            executable_fixture("tests/fixtures/provider/codex_app_server_current_fixture.sh");
        let provider = CodexProvider::new(fixture);
        let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .unwrap();

        let mut saw_started = false;
        let mut saw_completed = false;
        for _ in 0..20 {
            match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
                .await
                .expect("provider should emit execution events")
                .expect("provider event channel should stay open")
            {
                ProviderEvent::Execution(event)
                    if event.kind == ProviderExecutionEventKind::Command
                        && event.status == ProviderExecutionEventStatus::Started =>
                {
                    assert_eq!(event.event_id, "command_cmd_001");
                    assert_eq!(event.command.as_deref(), Some("pwd"));
                    assert!(event.cwd.is_some());
                    saw_started = true;
                }
                ProviderEvent::Execution(event)
                    if event.kind == ProviderExecutionEventKind::Command
                        && event.status == ProviderExecutionEventStatus::Completed =>
                {
                    assert_eq!(event.event_id, "command_cmd_001");
                    assert_eq!(event.command.as_deref(), Some("pwd"));
                    assert_eq!(event.exit_code, Some(0));
                    assert!(event.output.as_deref().unwrap_or_default().contains('/'));
                    saw_completed = true;
                }
                ProviderEvent::Completed { .. } if saw_started && saw_completed => return,
                ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
                _ => {}
            }
        }

        assert!(saw_started, "command started event was not emitted");
        assert!(saw_completed, "command completed event was not emitted");
    }

    #[tokio::test]
    async fn codex_provider_times_out_when_turn_stops_emitting_events() {
        let fixture =
            executable_fixture("tests/fixtures/provider/codex_app_server_hanging_turn_fixture.sh");
        let provider = CodexProvider::new(fixture);
        let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
        input.timeout_secs = 1;
        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .unwrap();

        loop {
            match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
                .await
                .expect("provider should emit timeout failure")
                .expect("provider event channel should stay open until failure")
            {
                ProviderEvent::Failed { message } => {
                    assert!(
                        message.contains("timed out") || message.contains("timeout"),
                        "unexpected failure message: {message}"
                    );
                    return;
                }
                ProviderEvent::StatusChanged(_)
                | ProviderEvent::Execution(_)
                | ProviderEvent::TextDelta { .. }
                | ProviderEvent::PermissionRequest(_)
                | ProviderEvent::ChoiceRequest(_)
                | ProviderEvent::ToolCall(_)
                | ProviderEvent::ToolResult(_) => {}
                ProviderEvent::Completed { full_output, .. } => {
                    panic!("provider completed unexpectedly: {full_output}")
                }
                ProviderEvent::ProtocolError { message, .. } => {
                    panic!("provider protocol error: {message}")
                }
                ProviderEvent::PermissionTimeout { permission_id } => {
                    panic!("provider permission timed out: {permission_id}")
                }
            }
        }
    }

    #[tokio::test]
    async fn codex_provider_reports_resume_stall_when_resumed_turn_emits_no_events() {
        let fixture = executable_fixture(
            "tests/fixtures/provider/codex_app_server_resume_hanging_turn_fixture.sh",
        );
        let provider = CodexProvider::new(fixture);
        let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
        input.resume_provider_session_id = Some("codex-thread-stale".to_string());
        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .unwrap();

        loop {
            match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
                .await
                .expect("provider should emit resume stall failure")
                .expect("provider event channel should stay open until failure")
            {
                ProviderEvent::Failed { message } => {
                    assert!(
                        message.contains("Codex resume stalled before provider progress"),
                        "unexpected failure message: {message}"
                    );
                    return;
                }
                ProviderEvent::StatusChanged(_)
                | ProviderEvent::Execution(_)
                | ProviderEvent::TextDelta { .. }
                | ProviderEvent::PermissionRequest(_)
                | ProviderEvent::ChoiceRequest(_)
                | ProviderEvent::ToolCall(_)
                | ProviderEvent::ToolResult(_) => {}
                ProviderEvent::Completed { full_output, .. } => {
                    panic!("provider completed unexpectedly: {full_output}")
                }
                ProviderEvent::ProtocolError { message, .. } => {
                    panic!("provider protocol error: {message}")
                }
                ProviderEvent::PermissionTimeout { permission_id } => {
                    panic!("provider permission timed out: {permission_id}")
                }
            }
        }
    }
}
