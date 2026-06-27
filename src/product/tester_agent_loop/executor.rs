use std::path::Path;

use serde_json::{Value, json};

use crate::cross_cutting::streaming_provider::{ProviderToolCall, ProviderToolResult};
use crate::product::coding_models::TestCommandStatus;
use crate::product::test_executor::{TestCommandSpec, execute_test_command};

use super::prompts::tester_allowed_tools;
use super::tools::{list_files_tool, read_file_tool, search_code_tool};
use super::types::{TesterAgentError, TesterToolOutcome};

pub async fn execute_tester_tool_call(
    call: &ProviderToolCall,
    worktree_path: impl AsRef<Path>,
    artifact_output_root: impl AsRef<Path>,
) -> Result<TesterToolOutcome, TesterAgentError> {
    let worktree_path = worktree_path.as_ref();
    let artifact_output_root = artifact_output_root.as_ref();
    if !tester_allowed_tools().contains(&call.tool_name.as_str()) {
        return Ok(error_outcome(call, "Tester 不允许修改文件或调用未授权工具"));
    }
    match call.tool_name.as_str() {
        "run_command" => run_command_tool(call, worktree_path, artifact_output_root).await,
        "read_file" => Ok(text_tool_outcome(
            call,
            read_file_tool(&call.input, worktree_path),
        )),
        "list_files" => Ok(text_tool_outcome(
            call,
            list_files_tool(&call.input, worktree_path),
        )),
        "search_code" => Ok(text_tool_outcome(
            call,
            search_code_tool(&call.input, worktree_path),
        )),
        _ => Ok(error_outcome(call, "Tester 不允许修改文件或调用未授权工具")),
    }
}

async fn run_command_tool(
    call: &ProviderToolCall,
    worktree_path: &Path,
    artifact_output_root: &Path,
) -> Result<TesterToolOutcome, TesterAgentError> {
    let command = match command_parts_from_input(&call.input) {
        Ok(command) => command,
        Err(message) => return Ok(error_outcome(call, &message)),
    };
    let spec = TestCommandSpec {
        id: command_id_for_tool_call(&call.id),
        command,
    };
    let command = execute_test_command(&spec, worktree_path, artifact_output_root).await?;
    Ok(TesterToolOutcome {
        result: ProviderToolResult {
            tool_use_id: call.id.clone(),
            output: serde_json::to_string(&json!({
                "command": command.command,
                "exit_code": command.exit_code,
                "status": command.status,
                "stdout_ref": command.stdout_ref,
                "stderr_ref": command.stderr_ref,
                "duration_ms": command.duration_ms
            }))
            .expect("serialize command result"),
            is_error: command.status != TestCommandStatus::Passed,
        },
        command: Some(command),
    })
}

fn text_tool_outcome(call: &ProviderToolCall, output: Result<String, String>) -> TesterToolOutcome {
    match output {
        Ok(output) => TesterToolOutcome {
            result: ProviderToolResult {
                tool_use_id: call.id.clone(),
                output,
                is_error: false,
            },
            command: None,
        },
        Err(message) => error_outcome(call, &message),
    }
}

fn error_outcome(call: &ProviderToolCall, message: &str) -> TesterToolOutcome {
    TesterToolOutcome {
        result: ProviderToolResult {
            tool_use_id: call.id.clone(),
            output: message.to_string(),
            is_error: true,
        },
        command: None,
    }
}

fn command_parts_from_input(input: &Value) -> Result<Vec<String>, String> {
    let command_value = input
        .get("command")
        .ok_or_else(|| "run_command 缺少 command 参数".to_string())?;
    let parts = if let Some(parts) = command_value.as_array() {
        parts
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(ToString::to_string)
                    .ok_or_else(|| "run_command command 数组只能包含字符串".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?
    } else if let Some(command) = command_value.as_str() {
        command
            .split_whitespace()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    } else {
        return Err("run_command command 必须是字符串数组或字符串".to_string());
    };
    if parts.is_empty() {
        return Err("run_command command 不能为空".to_string());
    }
    Ok(parts)
}

fn command_id_for_tool_call(id: &str) -> String {
    let mut value = id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        value = "tool_call".to_string();
    }
    value
}
