use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;

use command_group::AsyncGroupChild;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::{ApprovalBridge, ChoiceDecision};
use crate::cross_cutting::process_manager::ProcessManager;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestData, ChoiceRequestSource, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, ProviderSession, ProviderStatus, RiskLevel, StreamingProviderAdapter,
    StreamingProviderInput,
};

mod ask_user_question;
mod stream;
mod tool;

#[cfg(test)]
pub mod tests;

const TOOL_RESULT_PREVIEW_MAX_BYTES: usize = 500;
#[derive(Debug, Clone)]
struct ClaudePermissionRequest {
    request_id: String,
    tool_use_id: Option<String>,
    tool_name: String,
    description: String,
    input: Value,
}

#[derive(Debug, Clone)]
struct ResolvedAskUserQuestion {
    input: Value,
    answers: serde_json::Map<String, Value>,
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
    is_error: bool,
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

    fn parse_stream_text_delta(value: &Value) -> Option<String> {
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
        }
        None
    }

    fn parse_assistant_text(value: &Value) -> Option<String> {
        if value.get("type")?.as_str()? != "assistant" {
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

    fn assistant_text_delta(assistant_text: &str, emitted_text: &str) -> Option<String> {
        if assistant_text.is_empty() || assistant_text == emitted_text {
            return None;
        }
        if emitted_text.is_empty() {
            return Some(assistant_text.to_string());
        }
        if let Some(suffix) = assistant_text.strip_prefix(emitted_text) {
            if suffix.is_empty() {
                return None;
            }
            return Some(suffix.to_string());
        }
        if emitted_text.ends_with(assistant_text) {
            return None;
        }
        Some(assistant_text.to_string())
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
                let is_error = item
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
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
                    is_error,
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
            tool_use_id: value
                .get("tool_use_id")
                .or_else(|| request.get("tool_use_id"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
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
        let payload = Self::control_response_payload(
            request_id,
            json!({
                "behavior": behavior,
                "message": reason,
            }),
        );
        tool::write_json_line(stdin, &payload).await
    }

    async fn write_choice_control_response(
        stdin: &Arc<Mutex<ChildStdin>>,
        request_id: &str,
        original_input: &Value,
        answers: serde_json::Map<String, Value>,
    ) -> Result<(), ProviderAdapterError> {
        eprintln!(
            "[aria-choice-diag] claude writing control_response request_id={} answer_keys={:?}",
            request_id,
            answers.keys().cloned().collect::<Vec<_>>()
        );
        let mut updated_input = original_input.clone();
        if let Some(obj) = updated_input.as_object_mut() {
            obj.insert("answers".to_string(), Value::Object(answers));
        }
        let payload = Self::control_response_payload(
            request_id,
            json!({
                "behavior": "allow",
                "updatedInput": updated_input,
            }),
        );
        tool::write_json_line(stdin, &payload).await
    }

    async fn write_tool_result(
        stdin: &Arc<Mutex<ChildStdin>>,
        tool_use_id: &str,
        input: &Value,
        answers: &serde_json::Map<String, Value>,
    ) -> Result<(), ProviderAdapterError> {
        eprintln!(
            "[aria-choice-diag] claude writing AskUserQuestion tool_result tool_use_id={} answer_keys={:?}",
            tool_use_id,
            answers.keys().cloned().collect::<Vec<_>>()
        );
        let content = ask_user_question::ask_user_question_tool_result_content(input, answers);
        let payload = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": content,
                }],
            },
        });
        tool::write_json_line(stdin, &payload).await
    }

    fn control_response_payload(request_id: &str, response: Value) -> Value {
        json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": response,
            }
        })
    }

    async fn write_initial_messages(
        stdin: &Arc<Mutex<ChildStdin>>,
        input: &StreamingProviderInput,
    ) -> Result<(), ProviderAdapterError> {
        tool::write_json_line(
            stdin,
            &json!({
                "type": "control_request",
                "request": {
                    "subtype": "initialize",
                },
            }),
        )
        .await?;
        tool::write_json_line(
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
        tool::write_json_line(
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
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

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
                let stderr = tool::combine_stderr(stderr_output.lock().await.clone(), error.stderr);
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
                        message: tool::format_exit_failure(status, stderr),
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

            let result =
                stream::read_claude_stream(stdout, stdin, bridge, event_tx.clone(), cancel).await;
            match result {
                Ok(ClaudeStreamOutcome::Aborted) => {
                    stderr_task.abort();
                    stream::terminate_aborted_child(&mut child).await;
                    let _ = stderr_task.await;
                }
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
                                message: tool::format_exit_failure(status, stderr),
                            })
                            .await;
                    }
                }
                Err(error) => {
                    let _ = child.start_kill();
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
}
