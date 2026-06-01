use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::process_manager::ProcessManager;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestData, ProviderEvent, ProviderExecutionEvent,
    ProviderExecutionEventKind, ProviderExecutionEventStatus, ProviderPermissionMode,
    ProviderSession, ProviderStatus, RiskLevel, StreamChunk, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::protocol::contracts::AdapterInput;

const TOOL_RESULT_PREVIEW_MAX_BYTES: usize = 500;

#[derive(Debug, Clone)]
struct ClaudePermissionRequest {
    request_id: String,
    tool_name: String,
    description: String,
    input: Value,
}

#[derive(Debug, Clone)]
struct ToolUseBlock {
    id: String,
    name: String,
    input: Value,
}

#[derive(Debug, Clone)]
struct ToolResultBlock {
    tool_use_id: String,
    output: String,
}

#[derive(Debug, Clone)]
pub struct ClaudeCodeProvider {
    command: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeStreamOutcome {
    TerminalEventEmitted,
    Aborted,
    EofWithoutResult,
}

impl ClaudeCodeProvider {
    pub fn new(command: PathBuf) -> Self {
        Self { command }
    }

    fn build_args(
        &self,
        mode: ProviderPermissionMode,
        resume_provider_session_id: Option<&str>,
    ) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            "--verbose".to_string(),
            "--output-format=stream-json".to_string(),
            "--input-format=stream-json".to_string(),
            "--include-partial-messages".to_string(),
            "--replay-user-messages".to_string(),
        ];

        if let Some(session_id) = resume_provider_session_id
            .map(str::trim)
            .filter(|session_id| !session_id.is_empty())
        {
            args.push("--resume".to_string());
            args.push(session_id.to_string());
        }

        if mode == ProviderPermissionMode::Supervised {
            args.push("--permission-prompt-tool=stdio".to_string());
        }

        args
    }

    fn parse_text_delta(value: &Value, received_stream_events: bool) -> Option<String> {
        if value.get("type")?.as_str()? == "stream_event" {
            let event = value.get("event")?;
            if event.get("type")?.as_str()? == "content_block_delta" {
                let delta = event.get("delta")?;
                if delta.get("type")?.as_str()? == "text_delta" {
                    let text = delta.get("text")?.as_str()?;
                    if !text.is_empty() {
                        return Some(text.to_string());
                    }
                }
            }
            return None;
        }

        if value.get("type")?.as_str()? != "assistant" {
            return None;
        }

        if received_stream_events {
            return None;
        }

        let content = value.get("message")?.get("content")?.as_array()?;
        let text = content
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");

        if text.is_empty() { None } else { Some(text) }
    }

    fn parse_tool_use_from_assistant(value: &Value) -> Option<Vec<ToolUseBlock>> {
        if value.get("type")?.as_str()? != "assistant" {
            return None;
        }
        let content = value.get("message")?.get("content")?.as_array()?;
        let tool_uses: Vec<ToolUseBlock> = content
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
            .filter_map(|item| {
                Some(ToolUseBlock {
                    id: item.get("id")?.as_str()?.to_string(),
                    name: item.get("name")?.as_str()?.to_string(),
                    input: item.get("input").cloned().unwrap_or(Value::Null),
                })
            })
            .collect();
        if tool_uses.is_empty() {
            None
        } else {
            Some(tool_uses)
        }
    }

    fn parse_tool_result(value: &Value) -> Option<Vec<ToolResultBlock>> {
        if value.get("type")?.as_str()? != "user" {
            return None;
        }
        let content = value.get("message")?.get("content")?.as_array()?;
        let results: Vec<ToolResultBlock> = content
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_result"))
            .filter_map(|item| {
                let tool_use_id = item.get("tool_use_id")?.as_str()?.to_string();
                let output = match item.get("content") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Array(arr)) => arr
                        .iter()
                        .filter_map(|block| block.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => String::new(),
                };
                Some(ToolResultBlock {
                    tool_use_id,
                    output,
                })
            })
            .collect();
        if results.is_empty() {
            None
        } else {
            Some(results)
        }
    }

    fn parse_control_request(value: &Value) -> Option<ClaudePermissionRequest> {
        if value.get("type")?.as_str()? != "control_request" {
            return None;
        }

        let request = value.get("request")?;
        if request.get("subtype")?.as_str()? != "can_use_tool" {
            return None;
        }

        let input = request.get("input").unwrap_or(&Value::Null);
        let command = input.get("command").and_then(Value::as_str);
        let description = input
            .get("description")
            .and_then(Value::as_str)
            .or(command)
            .unwrap_or("Claude Code tool request");

        Some(ClaudePermissionRequest {
            request_id: value.get("request_id")?.as_str()?.to_string(),
            tool_name: request
                .get("tool_name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            description: description.to_string(),
            input: input.clone(),
        })
    }

    async fn write_control_response(
        stdin: &Arc<Mutex<ChildStdin>>,
        request_id: &str,
        approved: bool,
        reason: Option<String>,
    ) -> Result<(), ProviderAdapterError> {
        let behavior = if approved { "allow" } else { "deny" };
        let payload = json!({
            "type": "control_response",
            "request_id": request_id,
            "response": {
                "behavior": behavior,
                "message": reason,
            }
        });
        write_json_line(stdin, &payload).await
    }

    async fn write_choice_control_response(
        stdin: &Arc<Mutex<ChildStdin>>,
        request_id: &str,
        original_input: &Value,
        answers: serde_json::Map<String, Value>,
    ) -> Result<(), ProviderAdapterError> {
        let mut updated_input = original_input.clone();
        if let Some(obj) = updated_input.as_object_mut() {
            obj.insert("answers".to_string(), Value::Object(answers));
        }
        let payload = json!({
            "type": "control_response",
            "request_id": request_id,
            "response": {
                "behavior": "allow",
                "updatedInput": updated_input,
            }
        });
        write_json_line(stdin, &payload).await
    }

    async fn write_initial_messages(
        stdin: &Arc<Mutex<ChildStdin>>,
        input: &StreamingProviderInput,
    ) -> Result<(), ProviderAdapterError> {
        write_json_line(
            stdin,
            &json!({
                "type": "control_request",
                "request": {
                    "subtype": "initialize",
                },
            }),
        )
        .await?;
        write_json_line(
            stdin,
            &json!({
                "type": "control_request",
                "request": {
                    "subtype": "set_permission_mode",
                    "mode": match input.permission_mode {
                        ProviderPermissionMode::Auto => "auto",
                        ProviderPermissionMode::Supervised => "supervised",
                    },
                },
            }),
        )
        .await?;
        write_json_line(
            stdin,
            &json!({
                "type": "user",
                "message": {
                    "role": "user",
                    "content": input.prompt,
                },
            }),
        )
        .await
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ClaudeCodeProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let args = self.build_args(
            input.permission_mode.clone(),
            input.resume_provider_session_id.as_deref(),
        );
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

        let stdin = Arc::new(Mutex::new(process.stdin));
        let stdout = process.stdout;
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
                title: "Claude Code provider started".to_string(),
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

            if let Err(error) = Self::write_initial_messages(&stdin, &input).await {
                let _ = child.start_kill();
                let status = child.wait().await;
                let _ = stderr_task.await;
                let stderr = combine_stderr(stderr_output.lock().await.clone(), error.stderr);
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "provider".to_string(),
                        kind: ProviderExecutionEventKind::Provider,
                        status: ProviderExecutionEventStatus::Failed,
                        title: "Claude Code provider failed".to_string(),
                        detail: Some(error.details),
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
                        message: format_exit_failure(status, stderr),
                    })
                    .await;
                return;
            }
            let _ = event_tx
                .send(ProviderEvent::StatusChanged(ProviderStatus::Running))
                .await;
            let _ = event_tx
                .send(ProviderEvent::Execution(ProviderExecutionEvent {
                    event_id: "turn".to_string(),
                    kind: ProviderExecutionEventKind::Turn,
                    status: ProviderExecutionEventStatus::Started,
                    title: "Turn started".to_string(),
                    detail: None,
                    command: None,
                    cwd: Some(input.working_dir.display().to_string()),
                    output: None,
                    exit_code: None,
                }))
                .await;

            let result = read_claude_stream(stdout, stdin, bridge, event_tx.clone(), cancel).await;
            if matches!(result, Ok(ClaudeStreamOutcome::Aborted) | Err(_)) {
                let _ = child.start_kill();
            }
            match result {
                Ok(outcome) => {
                    let status = child.wait().await;
                    let _ = stderr_task.await;
                    if outcome == ClaudeStreamOutcome::EofWithoutResult {
                        let stderr = stderr_output.lock().await.clone();
                        let _ = event_tx
                            .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                            .await;
                        let _ = event_tx
                            .send(ProviderEvent::Execution(ProviderExecutionEvent {
                                event_id: "provider".to_string(),
                                kind: ProviderExecutionEventKind::Provider,
                                status: ProviderExecutionEventStatus::Failed,
                                title: "Claude Code provider failed".to_string(),
                                detail: Some("exited without result".to_string()),
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
                                message: format_exit_failure(status, stderr),
                            })
                            .await;
                    }
                }
                Err(error) => {
                    let _ = event_tx
                        .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                        .await;
                    let _ = event_tx
                        .send(ProviderEvent::Execution(ProviderExecutionEvent {
                            event_id: "provider".to_string(),
                            kind: ProviderExecutionEventKind::Provider,
                            status: ProviderExecutionEventStatus::Failed,
                            title: "Claude Code provider failed".to_string(),
                            detail: Some(error.details.clone()),
                            command: None,
                            cwd: None,
                            output: None,
                            exit_code: None,
                        }))
                        .await;
                    let _ = event_tx
                        .send(ProviderEvent::Failed {
                            message: error.details,
                        })
                        .await;
                    let _ = child.wait().await;
                    let _ = stderr_task.await;
                }
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

async fn read_claude_stream(
    stdout: tokio::process::ChildStdout,
    stdin: Arc<Mutex<ChildStdin>>,
    bridge: ApprovalBridge,
    event_tx: mpsc::Sender<ProviderEvent>,
    cancel: CancellationToken,
) -> Result<ClaudeStreamOutcome, ProviderAdapterError> {
    use std::collections::HashMap;

    let mut lines = BufReader::new(stdout).lines();
    let mut pending_tool_uses: HashMap<String, ToolUseBlock> = HashMap::new();
    let mut received_stream_events = false;

    loop {
        let line = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Aborted))
                    .await;
                return Ok(ClaudeStreamOutcome::Aborted);
            }
            line = lines.next_line() => line.map_err(|error| {
                ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
            })?,
        };
        let Some(line) = line else {
            return Ok(ClaudeStreamOutcome::EofWithoutResult);
        };
        if line.trim().is_empty() {
            continue;
        }

        let value = serde_json::from_str::<Value>(&line).map_err(|error| {
            ProviderAdapterError::parse_error(
                format!("invalid Claude stream JSON: {error}"),
                line.clone(),
                String::new(),
            )
        })?;

        if let Some(content) = ClaudeCodeProvider::parse_text_delta(&value, received_stream_events)
        {
            if value.get("type").and_then(Value::as_str) == Some("stream_event") {
                received_stream_events = true;
            }
            send_provider_event(&event_tx, ProviderEvent::TextDelta { content }, &cancel).await?;
            continue;
        }

        if let Some(request) = ClaudeCodeProvider::parse_control_request(&value) {
            if request.tool_name == "AskUserQuestion" {
                let choice_request =
                    parse_ask_user_question_from_input(&request.input, &request.request_id);
                let choice_decision = bridge
                    .request_choice(choice_request, cancel.clone())
                    .await?;
                let mut answers = serde_json::Map::new();
                if let Some(text) = &choice_decision.free_text {
                    let questions = request.input.get("questions").and_then(Value::as_array);
                    if let Some(questions) = questions
                        && let Some(first_q) = questions.first()
                    {
                        let question_text = first_q
                            .get("question")
                            .and_then(Value::as_str)
                            .unwrap_or("question");
                        answers.insert(question_text.to_string(), Value::String(text.clone()));
                    }
                } else if !choice_decision.selected_option_ids.is_empty() {
                    let questions = request.input.get("questions").and_then(Value::as_array);
                    if let Some(questions) = questions
                        && let Some(first_q) = questions.first()
                    {
                        let question_text = first_q
                            .get("question")
                            .and_then(Value::as_str)
                            .unwrap_or("question");
                        let options = first_q.get("options").and_then(Value::as_array);
                        let selected_label = options
                            .and_then(|opts| {
                                choice_decision.selected_option_ids.first().and_then(|id| {
                                    let idx = id.strip_prefix("opt_")?.parse::<usize>().ok()?;
                                    opts.get(idx)?.get("label")?.as_str().map(String::from)
                                })
                            })
                            .unwrap_or_else(|| choice_decision.selected_option_ids.join(", "));
                        answers.insert(question_text.to_string(), Value::String(selected_label));
                    }
                }
                ClaudeCodeProvider::write_choice_control_response(
                    &stdin,
                    &request.request_id,
                    &request.input,
                    answers,
                )
                .await?;
            } else {
                let decision = bridge
                    .request_tool(
                        &request.tool_name,
                        &request.description,
                        RiskLevel::High,
                        cancel.clone(),
                    )
                    .await?;
                ClaudeCodeProvider::write_control_response(
                    &stdin,
                    &request.request_id,
                    decision.approved,
                    decision.reason,
                )
                .await?;
            }
            continue;
        }

        if let Some(tool_uses) = ClaudeCodeProvider::parse_tool_use_from_assistant(&value) {
            for tool_use in tool_uses {
                if tool_use.name == "AskUserQuestion" {
                    continue;
                } else {
                    let description = tool_use_description(&tool_use);
                    send_provider_event(
                        &event_tx,
                        ProviderEvent::Execution(ProviderExecutionEvent {
                            event_id: tool_use.id.clone(),
                            kind: ProviderExecutionEventKind::Command,
                            status: ProviderExecutionEventStatus::Started,
                            title: tool_use.name.clone(),
                            detail: Some(description),
                            command: tool_use_command(&tool_use),
                            cwd: None,
                            output: None,
                            exit_code: None,
                        }),
                        &cancel,
                    )
                    .await?;
                    pending_tool_uses.insert(tool_use.id.clone(), tool_use);
                }
            }
            continue;
        }

        if let Some(results) = ClaudeCodeProvider::parse_tool_result(&value) {
            for result in results {
                if let Some(tool_use) = pending_tool_uses.remove(&result.tool_use_id) {
                    let output_preview =
                        output_preview(&result.output, TOOL_RESULT_PREVIEW_MAX_BYTES);
                    let command = tool_use_command(&tool_use);
                    send_provider_event(
                        &event_tx,
                        ProviderEvent::Execution(ProviderExecutionEvent {
                            event_id: tool_use.id,
                            kind: ProviderExecutionEventKind::Command,
                            status: ProviderExecutionEventStatus::Completed,
                            title: tool_use.name,
                            detail: None,
                            command,
                            cwd: None,
                            output: Some(output_preview),
                            exit_code: Some(0),
                        }),
                        &cancel,
                    )
                    .await?;
                }
            }
            continue;
        }

        if value.get("type").and_then(Value::as_str) == Some("result") {
            let is_error = value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if is_error {
                send_provider_event(
                    &event_tx,
                    ProviderEvent::Failed {
                        message: value
                            .get("result")
                            .and_then(Value::as_str)
                            .unwrap_or("Claude Code provider failed")
                            .to_string(),
                    },
                    &cancel,
                )
                .await?;
                return Ok(ClaudeStreamOutcome::TerminalEventEmitted);
            }

            let full_output = value
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let provider_session_id = value
                .get("session_id")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            send_provider_event(
                &event_tx,
                ProviderEvent::Execution(ProviderExecutionEvent {
                    event_id: "turn".to_string(),
                    kind: ProviderExecutionEventKind::Turn,
                    status: ProviderExecutionEventStatus::Completed,
                    title: "Turn completed".to_string(),
                    detail: None,
                    command: None,
                    cwd: None,
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
                    provider_session_id,
                },
                &cancel,
            )
            .await?;
            return Ok(ClaudeStreamOutcome::TerminalEventEmitted);
        }
    }
}

fn parse_ask_user_question_from_input(input: &Value, request_id: &str) -> ChoiceRequestData {
    let questions = input.get("questions").and_then(Value::as_array);

    let (prompt, options, allow_multiple) = if let Some(questions) = questions {
        if let Some(first_question) = questions.first() {
            let prompt = first_question
                .get("question")
                .and_then(Value::as_str)
                .unwrap_or("请选择")
                .to_string();
            let multi = first_question
                .get("multiSelect")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let opts = first_question
                .get("options")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .enumerate()
                        .filter_map(|(idx, opt)| {
                            let label = opt.get("label")?.as_str()?.to_string();
                            let description = opt
                                .get("description")
                                .and_then(Value::as_str)
                                .map(String::from);
                            Some(ChoiceOptionData {
                                id: format!("opt_{idx}"),
                                label,
                                description,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (prompt, opts, multi)
        } else {
            ("请选择".to_string(), vec![], false)
        }
    } else {
        ("请选择".to_string(), vec![], false)
    };

    ChoiceRequestData {
        id: request_id.to_string(),
        prompt,
        options,
        allow_multiple,
        allow_free_text: true,
    }
}

fn tool_use_description(tool_use: &ToolUseBlock) -> String {
    match tool_use.name.as_str() {
        "Bash" => tool_use
            .input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        "Read" => tool_use
            .input
            .get("file_path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        "Edit" | "Write" => tool_use
            .input
            .get("file_path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => tool_use
            .input
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    }
}

fn tool_use_command(tool_use: &ToolUseBlock) -> Option<String> {
    match tool_use.name.as_str() {
        "Bash" => tool_use
            .input
            .get("command")
            .and_then(Value::as_str)
            .map(String::from),
        "Read" => Some(format!(
            "read {}",
            tool_use
                .input
                .get("file_path")
                .and_then(Value::as_str)
                .unwrap_or("?")
        )),
        "Edit" => Some(format!(
            "edit {}",
            tool_use
                .input
                .get("file_path")
                .and_then(Value::as_str)
                .unwrap_or("?")
        )),
        "Write" => Some(format!(
            "write {}",
            tool_use
                .input
                .get("file_path")
                .and_then(Value::as_str)
                .unwrap_or("?")
        )),
        _ => None,
    }
}

fn output_preview(output: &str, max_bytes: usize) -> String {
    if output.len() <= max_bytes {
        return output.to_string();
    }

    let truncate_at = output
        .char_indices()
        .map(|(idx, _)| idx)
        .take_while(|idx| *idx <= max_bytes)
        .last()
        .unwrap_or(0);
    format!("{}...", &output[..truncate_at])
}

fn combine_stderr(process_stderr: String, write_stderr: String) -> String {
    match (process_stderr.trim(), write_stderr.trim()) {
        ("", "") => String::new(),
        (process, "") => process.to_string(),
        ("", write_error) => write_error.to_string(),
        (process, write_error) => format!("{process}\n{write_error}"),
    }
}

fn format_exit_failure(status: Result<ExitStatus, std::io::Error>, stderr: String) -> String {
    let status_text = match status {
        Ok(status) => format!("exit status: {status}"),
        Err(error) => format!("failed to wait for process: {error}"),
    };
    if stderr.trim().is_empty() {
        format!("Claude Code provider exited without result ({status_text})")
    } else {
        format!(
            "Claude Code provider exited without result ({status_text}); stderr: {}",
            stderr.trim()
        )
    }
}

async fn write_json_line(
    stdin: &Arc<Mutex<ChildStdin>>,
    value: &Value,
) -> Result<(), ProviderAdapterError> {
    let mut stdin = stdin.lock().await;
    let line = serde_json::to_string(value).map_err(|error| {
        ProviderAdapterError::parse_error(
            format!("invalid Claude control JSON: {error}"),
            String::new(),
            String::new(),
        )
    })?;
    stdin.write_all(line.as_bytes()).await.map_err(|error| {
        ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
    })?;
    stdin.write_all(b"\n").await.map_err(|error| {
        ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
    })?;
    stdin.flush().await.map_err(|error| {
        ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
    })
}

async fn send_provider_event(
    event_tx: &mpsc::Sender<ProviderEvent>,
    event: ProviderEvent,
    cancel: &CancellationToken,
) -> Result<(), ProviderAdapterError> {
    tokio::select! {
        _ = cancel.cancelled() => Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "Claude Code provider cancelled",
            0,
        )),
        result = event_tx.send(event) => result.map_err(|_| {
            ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "provider event receiver closed",
                0,
            )
        }),
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
        ProviderCommand, ProviderEvent, ProviderExecutionEventStatus, ProviderPermissionMode,
        ProviderSession, ProviderStatus, StreamingProviderAdapter, StreamingProviderInput,
    };
    use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

    use super::ClaudeCodeProvider;

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn claude_args_include_resume_when_provider_session_is_available() {
        let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
        let args = provider.build_args(
            ProviderPermissionMode::Supervised,
            Some("claude-session-123"),
        );

        assert!(args.contains(&"--resume".to_string()));
        assert!(args.contains(&"claude-session-123".to_string()));
        assert!(!args.contains(&"--continue".to_string()));
        assert!(!args.contains(&"--fork-session".to_string()));
    }

    #[test]
    fn claude_args_do_not_include_resume_without_provider_session() {
        let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
        let args = provider.build_args(ProviderPermissionMode::Supervised, None);

        assert!(!args.contains(&"--resume".to_string()));
        assert!(!args.contains(&"--continue".to_string()));
    }

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
            prompt: "Run the fixture provider".to_string(),
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

    async fn wait_for_receiver_closed<T>(rx: &mpsc::Receiver<T>) {
        for _ in 0..200 {
            if rx.is_closed() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("receiver did not close after cancellation");
    }

    async fn wait_for_buffer_len<T>(rx: &mpsc::Receiver<T>, expected_len: usize) {
        for _ in 0..1000 {
            if rx.len() >= expected_len {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!(
            "receiver buffer did not reach {expected_len} items; actual len is {}",
            rx.len()
        );
    }

    #[cfg(target_os = "linux")]
    async fn wait_for_process_absent(pid: u32) {
        let proc_path = PathBuf::from(format!("/proc/{pid}"));
        for _ in 0..200 {
            if !proc_path.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("process {pid} was not reaped after cancellation");
    }

    fn adapter_input(prompt: &str) -> AdapterInput {
        AdapterInput {
            provider_type: ProviderType::ClaudeCode,
            role: AdapterRole::Orchestrator,
            worktree_path: Some(
                std::env::current_dir()
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
            ),
            prompt: prompt.to_string(),
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 60,
            max_retries: 0,
        }
    }

    fn write_fixture(relative_path: &str, body: &str) -> PathBuf {
        let path = tempfile::tempdir()
            .expect("fixture dir")
            .keep()
            .join(relative_path);
        std::fs::write(&path, body).expect("write fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).expect("chmod fixture");
        }
        path
    }

    #[tokio::test]
    async fn claude_provider_bridges_permission_and_completes() {
        let fixture = executable_fixture("tests/fixtures/provider/claude_stream_json_fixture.sh");
        let provider = ClaudeCodeProvider::new(fixture);
        let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);

        let mut session: ProviderSession = provider
            .start(input, CancellationToken::new())
            .await
            .unwrap();

        assert!(matches!(
            session.events.recv().await.unwrap(),
            ProviderEvent::StatusChanged(ProviderStatus::Starting)
        ));

        let mut saw_text = false;
        let permission_id = loop {
            match session.events.recv().await.unwrap() {
                ProviderEvent::TextDelta { content } => {
                    saw_text = content.contains("Claude fixture chunk");
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
        assert_eq!(completed, "Claude fixture chunk done");
    }

    #[tokio::test]
    async fn claude_provider_reports_failure_when_process_exits_without_result() {
        let fixture = write_fixture(
            "claude_fail_fixture.sh",
            "#!/usr/bin/env bash\nset -euo pipefail\nexec 0<&-\necho 'not authenticated' >&2\nexit 7\n",
        );
        let provider = ClaudeCodeProvider::new(fixture);
        let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Auto);

        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .expect("start provider");

        let mut failed = None;
        while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit failure")
        {
            if let ProviderEvent::Failed { message } = event {
                failed = Some(message);
                break;
            }
        }

        let failed = failed.expect("provider should emit failed event");
        assert!(
            failed.contains("exited without result") || failed.contains("exit status"),
            "unexpected failure message: {failed}"
        );
        assert!(
            failed.contains("not authenticated"),
            "unexpected failure message: {failed}"
        );
    }

    #[tokio::test]
    async fn claude_provider_truncates_multibyte_tool_result_preview_without_panicking() {
        let long_output = "通".repeat(180);
        let tool_use_line = serde_json::json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_utf8",
                    "name": "Bash",
                    "input": { "command": "printf unicode output" }
                }]
            },
            "session_id": "claude_fixture_session"
        })
        .to_string();
        let tool_result_line = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_utf8",
                    "content": long_output
                }]
            },
            "session_id": "claude_fixture_session"
        })
        .to_string();
        let result_line = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "result": "done",
            "session_id": "claude_fixture_session"
        })
        .to_string();
        let body = format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"user\"'* ]]; then\n    printf '%s\\n' '{tool_use_line}'\n    printf '%s\\n' '{tool_result_line}'\n    printf '%s\\n' '{result_line}'\n    exit 0\n  fi\ndone\n"
        );
        let fixture = write_fixture("claude_utf8_tool_result_fixture.sh", &body);
        let provider = ClaudeCodeProvider::new(fixture);
        let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Auto);

        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .expect("start provider");
        let mut preview = None;
        let mut completed = None;

        while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should not hang while handling utf8 tool result")
        {
            match event {
                ProviderEvent::Execution(event) => {
                    if event.event_id == "toolu_utf8"
                        && event.status == ProviderExecutionEventStatus::Completed
                    {
                        preview = event.output;
                    }
                }
                ProviderEvent::Completed { full_output, .. } => {
                    completed = Some(full_output);
                    break;
                }
                ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
                ProviderEvent::ProtocolError { message, .. } => {
                    panic!("provider protocol error: {message}")
                }
                ProviderEvent::PermissionTimeout { permission_id } => {
                    panic!("provider permission timed out: {permission_id}")
                }
                ProviderEvent::StatusChanged(_)
                | ProviderEvent::TextDelta { .. }
                | ProviderEvent::PermissionRequest(_)
                | ProviderEvent::ChoiceRequest(_)
                | ProviderEvent::ToolCall(_)
                | ProviderEvent::ToolResult(_) => {}
            }
        }

        assert_eq!(completed.as_deref(), Some("done"));
        let preview = preview.expect("tool result preview");
        assert!(preview.ends_with("..."), "preview should be truncated");
        assert!(
            preview.len() <= 503,
            "preview should be capped to 500 bytes plus suffix, got {} bytes",
            preview.len()
        );
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn claude_provider_cancel_kills_and_reaps_hanging_process() {
        let fixture = write_fixture(
            "claude_hanging_after_output_fixture.sh",
            "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$$\" > \"$ARIA_FIXTURE_PID\"\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"user\"'* ]]; then\n    echo '{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"started\"}]},\"session_id\":\"claude_fixture_session\"}'\n    sleep 30\n  fi\ndone\n",
        );
        let provider = ClaudeCodeProvider::new(fixture);
        let pid_path = tempfile::NamedTempFile::new()
            .expect("pid file")
            .into_temp_path();
        let mut input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Auto);
        input.env_vars.insert(
            "ARIA_FIXTURE_PID".to_string(),
            pid_path.to_string_lossy().to_string(),
        );
        let cancel = CancellationToken::new();

        let mut session = provider
            .start(input, cancel.clone())
            .await
            .expect("start provider");
        loop {
            match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
                .await
                .expect("provider should emit startup event")
                .expect("provider channel should stay open until cancellation")
            {
                ProviderEvent::TextDelta { content } if content == "started" => break,
                ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
                ProviderEvent::ProtocolError { message, .. } => {
                    panic!("provider protocol error: {message}")
                }
                ProviderEvent::PermissionTimeout { permission_id } => {
                    panic!("provider permission timed out: {permission_id}")
                }
                ProviderEvent::StatusChanged(_)
                | ProviderEvent::Execution(_)
                | ProviderEvent::TextDelta { .. }
                | ProviderEvent::PermissionRequest(_)
                | ProviderEvent::ChoiceRequest(_)
                | ProviderEvent::ToolCall(_)
                | ProviderEvent::ToolResult(_)
                | ProviderEvent::Completed { .. } => {}
            }
        }

        let pid = std::fs::read_to_string(&pid_path)
            .expect("fixture pid")
            .trim()
            .parse::<u32>()
            .expect("fixture pid number");
        cancel.cancel();
        drop(session);
        wait_for_process_absent(pid).await;
    }

    #[tokio::test]
    async fn claude_provider_run_streaming_cancel_closes_backpressured_bridge() {
        let mut body = String::from("#!/usr/bin/env bash\nset -euo pipefail\n");
        body.push_str("while IFS= read -r line; do\n");
        body.push_str("  if [[ \"$line\" == *'\"user\"'* ]]; then\n");
        for index in 0..80 {
            body.push_str(&format!(
                "    echo '{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"chunk {index}\"}}]}},\"session_id\":\"backpressure\"}}'\n"
            ));
        }
        body.push_str("    sleep 5\n");
        body.push_str("  fi\n");
        body.push_str("done\n");

        let fixture = write_fixture("claude_backpressure_fixture.sh", &body);
        let provider = ClaudeCodeProvider::new(fixture);
        let cancel = CancellationToken::new();
        let rx = provider
            .run_streaming(&adapter_input("trigger backpressure"), cancel.clone())
            .await
            .expect("run streaming");

        wait_for_buffer_len(&rx, 32).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
        tokio::time::timeout(TEST_TIMEOUT, wait_for_receiver_closed(&rx))
            .await
            .expect("stream receiver should close after cancellation");
    }
}
