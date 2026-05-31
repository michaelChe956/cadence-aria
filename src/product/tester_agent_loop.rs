use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::Duration;

use chrono::Utc;
use serde_json::{Value, json};
use thiserror::Error;

use crate::cross_cutting::streaming_provider::{ProviderToolCall, ProviderToolResult};
use crate::product::coding_models::{
    CodingExecutionAttempt, TestCommand, TestCommandStatus, TestingOverallStatus, TestingReport,
};
use crate::product::coding_workspace_engine::CodingExecutionContext;
use crate::product::test_executor::{
    TestCommandSpec, TestExecutorError, execute_test_command, infer_test_commands,
};

pub const TESTER_TOOL_FAILURE_LIMIT: usize = 3;

const TESTER_AGENT_TIMEOUT_SECS: u64 = 300;
const MAX_LISTED_FILES: usize = 200;
const MAX_SEARCH_MATCHES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TesterAgentOptions {
    pub timeout: Duration,
    pub failure_limit: usize,
}

impl Default for TesterAgentOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(TESTER_AGENT_TIMEOUT_SECS),
            failure_limit: TESTER_TOOL_FAILURE_LIMIT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TesterToolOutcome {
    pub result: ProviderToolResult,
    pub command: Option<TestCommand>,
}

#[derive(Debug, Error)]
pub enum TesterAgentError {
    #[error("tester tool failed: {0}")]
    Tool(String),
    #[error(transparent)]
    TestExecutor(#[from] TestExecutorError),
}

pub fn tester_allowed_tools() -> [&'static str; 4] {
    ["run_command", "read_file", "list_files", "search_code"]
}

pub fn build_tester_system_prompt(
    attempt: &CodingExecutionAttempt,
    context: &CodingExecutionContext,
    specs: &[TestCommandSpec],
) -> String {
    let inferred_specs = attempt
        .worktree_path
        .as_ref()
        .map(infer_test_commands)
        .unwrap_or_default();
    let prompt_specs = if specs.is_empty() {
        inferred_specs.as_slice()
    } else {
        specs
    };
    let changed_files = attempt
        .worktree_path
        .as_ref()
        .map(|path| detect_changed_files(path.as_path()))
        .unwrap_or_default();

    let mut prompt = format!(
        "Tester Agent Loop\n\
         你是 Coding Workspace tester。你只能验证和分析，不允许修改源码。\n\
         Project: {}\n\
         Issue: {}\n\
         Work Item: {}\n\
         Attempt: {}\n\
         Branch: {}\n",
        attempt.project_id, attempt.issue_id, attempt.work_item_id, attempt.id, attempt.branch_name
    );
    if let Some(worktree_path) = attempt.worktree_path.as_ref() {
        prompt.push_str(&format!("Worktree Path: {}\n", worktree_path.display()));
    }
    prompt.push_str("\n允许工具:\n");
    for tool in tester_allowed_tools() {
        prompt.push_str("- ");
        prompt.push_str(tool);
        prompt.push('\n');
    }
    prompt.push_str(
        "\n禁止工具:\n\
         - write_file\n\
         - edit_file\n\
         - delete_file\n",
    );
    prompt.push_str("\n可用测试命令:\n");
    if prompt_specs.is_empty() {
        prompt.push_str(
            "- 未推断到测试命令，请先使用 list_files/read_file/search_code 分析项目结构。\n",
        );
    } else {
        for spec in prompt_specs {
            prompt.push_str("- ");
            prompt.push_str(&spec.command.join(" "));
            prompt.push('\n');
        }
    }
    prompt.push_str("\n变更文件:\n");
    if changed_files.is_empty() {
        prompt.push_str("- 未检测到 git 变更文件。\n");
    } else {
        for file in changed_files {
            prompt.push_str("- ");
            prompt.push_str(&file);
            prompt.push('\n');
        }
    }
    if !context.verification_commands.is_empty() {
        prompt.push_str("\nWork Item 验证命令:\n");
        for command in &context.verification_commands {
            prompt.push_str("- ");
            prompt.push_str(command);
            prompt.push('\n');
        }
    }
    if let Some(markdown) = context.work_item_markdown.as_deref() {
        prompt.push_str("\n已确认 Work Item:\n````markdown\n");
        prompt.push_str(markdown.trim());
        prompt.push_str("\n````\n");
    }
    prompt.push_str(
        "\n输出要求:\n\
         - 优先调用 run_command 执行测试。\n\
         - 如发现失败，继续收集足够证据并在最终 JSON 中列出 bugs_found。\n\
         - 最终只输出 JSON：{\"summary\":\"...\",\"bugs_found\":[]}。\n",
    );
    prompt
}

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

pub fn build_testing_report(
    attempt_id: &str,
    commands: Vec<TestCommand>,
    provider_output: &str,
    blocked_summary: Option<String>,
) -> TestingReport {
    let provider_claim = parse_provider_claim(provider_output, blocked_summary.as_deref());
    let overall_status = if blocked_summary.is_some() || commands.is_empty() {
        TestingOverallStatus::Blocked
    } else if commands
        .iter()
        .all(|command| command.status == TestCommandStatus::Passed)
    {
        TestingOverallStatus::Passed
    } else {
        TestingOverallStatus::Failed
    };
    TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        commands,
        overall_status,
        provider_claim,
        backend_verified: true,
        started_at: Utc::now().to_rfc3339(),
        completed_at: Some(Utc::now().to_rfc3339()),
    }
}

fn parse_provider_claim(provider_output: &str, blocked_summary: Option<&str>) -> Option<Value> {
    if let Some(summary) = blocked_summary {
        return Some(json!({
            "summary": summary,
            "bugs_found": [],
            "warning": true
        }));
    }
    let trimmed = provider_output.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(trimmed)
        .ok()
        .or_else(|| Some(json!({"summary": trimmed, "bugs_found": []})))
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

fn read_file_tool(input: &Value, worktree_path: &Path) -> Result<String, String> {
    let path = input_path(input, "path", ".")?;
    let path = resolve_existing_worktree_path(worktree_path, &path)?;
    std::fs::read_to_string(&path)
        .map_err(|error| format!("读取文件失败 {}: {error}", path.display()))
}

fn list_files_tool(input: &Value, worktree_path: &Path) -> Result<String, String> {
    let path = input_path(input, "path", ".")?;
    let root = resolve_existing_worktree_path(worktree_path, &path)?;
    let mut files = Vec::new();
    collect_files(&root, worktree_path, &mut files, MAX_LISTED_FILES)?;
    Ok(json!({ "files": files }).to_string())
}

fn search_code_tool(input: &Value, worktree_path: &Path) -> Result<String, String> {
    let query = input
        .get("query")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "search_code 缺少 query 参数".to_string())?;
    let path = input_path(input, "path", ".")?;
    let root = resolve_existing_worktree_path(worktree_path, &path)?;
    let mut matches = Vec::new();
    search_files(
        &root,
        worktree_path,
        query,
        &mut matches,
        MAX_SEARCH_MATCHES,
    )?;
    Ok(json!({ "matches": matches }).to_string())
}

fn input_path(input: &Value, field: &str, default: &str) -> Result<PathBuf, String> {
    let value = input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(default);
    let path = PathBuf::from(value);
    if path.is_absolute() {
        return Err("工具路径必须是 worktree 内的相对路径".to_string());
    }
    Ok(path)
}

fn resolve_existing_worktree_path(
    worktree_path: &Path,
    relative_path: &Path,
) -> Result<PathBuf, String> {
    let root = worktree_path
        .canonicalize()
        .map_err(|error| format!("解析 worktree 路径失败: {error}"))?;
    let path = worktree_path
        .join(relative_path)
        .canonicalize()
        .map_err(|error| format!("解析工具路径失败 {}: {error}", relative_path.display()))?;
    if !path.starts_with(&root) {
        return Err("工具路径不能逃逸 worktree".to_string());
    }
    Ok(path)
}

fn collect_files(
    path: &Path,
    worktree_path: &Path,
    files: &mut Vec<String>,
    max_files: usize,
) -> Result<(), String> {
    if files.len() >= max_files || ignored_path(path) {
        return Ok(());
    }
    if path.is_file() {
        files.push(relative_display_path(path, worktree_path));
        return Ok(());
    }
    let entries = std::fs::read_dir(path)
        .map_err(|error| format!("列出目录失败 {}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("读取目录项失败: {error}"))?;
        collect_files(&entry.path(), worktree_path, files, max_files)?;
        if files.len() >= max_files {
            break;
        }
    }
    Ok(())
}

fn search_files(
    path: &Path,
    worktree_path: &Path,
    query: &str,
    matches: &mut Vec<Value>,
    max_matches: usize,
) -> Result<(), String> {
    if matches.len() >= max_matches || ignored_path(path) {
        return Ok(());
    }
    if path.is_dir() {
        let entries = std::fs::read_dir(path)
            .map_err(|error| format!("读取目录失败 {}: {error}", path.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("读取目录项失败: {error}"))?;
            search_files(&entry.path(), worktree_path, query, matches, max_matches)?;
            if matches.len() >= max_matches {
                break;
            }
        }
        return Ok(());
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    for (line_index, line) in content.lines().enumerate() {
        if !line.contains(query) {
            continue;
        }
        matches.push(json!({
            "path": relative_display_path(path, worktree_path),
            "line": line_index + 1,
            "text": line
        }));
        if matches.len() >= max_matches {
            break;
        }
    }
    Ok(())
}

fn ignored_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| matches!(name, ".git" | ".aria" | "target" | "node_modules"))
}

fn relative_display_path(path: &Path, worktree_path: &Path) -> String {
    path.strip_prefix(worktree_path)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn detect_changed_files(worktree_path: &Path) -> Vec<String> {
    let Ok(output) = StdCommand::new("git")
        .arg("-C")
        .arg(worktree_path)
        .arg("status")
        .arg("--short")
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.get(3..).map(str::trim))
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
        .collect()
}
