use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::process_manager::ProcessManager;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderPermissionMode, ProviderSession, RiskLevel, StreamChunk,
    StreamingProviderAdapter, StreamingProviderInput,
};
use crate::protocol::contracts::AdapterInput;

#[derive(Debug, Clone)]
pub struct CodexProvider {
    command: PathBuf,
}

impl CodexProvider {
    pub fn new(command: PathBuf) -> Self {
        Self { command }
    }

    fn build_args(&self) -> Vec<String> {
        vec!["app-server".to_string()]
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
            let status = child.wait().await;
            let _ = stderr_task.await;
            if let Err(error) = result {
                let stderr = combine_stderr(stderr_output.lock().await.clone(), error.stderr);
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
            session_id: None,
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
                    ProviderEvent::PermissionRequest(_) | ProviderEvent::StatusChanged(_) => {
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
        .request(json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "params": {},
        }))
        .await?;
    peer.send(json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {},
    }))
    .await?;

    let thread_response = peer
        .request(json!({
            "jsonrpc": "2.0",
            "method": "thread/start",
            "params": {
                "cwd": input.working_dir,
                "approvalPolicy": match input.permission_mode {
                    ProviderPermissionMode::Auto => "Never",
                    ProviderPermissionMode::Supervised => "OnRequest",
                },
                "ephemeral": true,
            },
        }))
        .await?;
    let thread_id = thread_response
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let turn_thread_id = thread_id.clone().unwrap_or_default();

    let _ = peer
        .request(json!({
            "jsonrpc": "2.0",
            "method": "turn/start",
            "params": {
                "threadId": turn_thread_id,
                "input": [
                    {
                        "type": "text",
                        "text": input.prompt,
                    }
                ],
            },
        }))
        .await?;

    let mut full_output = String::new();
    loop {
        let incoming = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(provider_error("Codex provider cancelled"));
            }
            incoming = peer.next_incoming() => incoming.ok_or_else(|| {
                provider_error("Codex app-server stream ended before completion")
            })?,
        };

        if let Some(content) = parse_text_delta(&incoming) {
            full_output.push_str(&content);
            send_provider_event(&event_tx, ProviderEvent::TextDelta { content }, &cancel).await?;
            continue;
        }

        if let Some(request) = parse_approval_request(&incoming) {
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

        if is_turn_completed(&incoming) {
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

fn parse_text_delta(value: &Value) -> Option<String> {
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

    if content.is_empty() {
        None
    } else {
        Some(content)
    }
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
        "method": "item/commandExecution/requestApproval",
        "result": {
            "decision": decision,
        },
        "response": {
            "decision": decision,
        },
    }))
    .await
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
        ProviderCommand, ProviderEvent, ProviderPermissionMode, StreamingProviderAdapter,
        StreamingProviderInput,
    };
    use crate::protocol::contracts::{AdapterRole, ProviderType};

    use super::CodexProvider;

    const TEST_TIMEOUT: Duration = Duration::from_secs(2);

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
            session_id: None,
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
                | ProviderEvent::TextDelta { .. }
                | ProviderEvent::PermissionRequest(_) => {}
                ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            }
        }
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
}
