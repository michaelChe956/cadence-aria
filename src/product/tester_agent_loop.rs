use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::cross_cutting::provider_adapter::DEFAULT_PROVIDER_TIMEOUT_SECS;
use crate::cross_cutting::streaming_provider::{ProviderToolCall, ProviderToolResult};
use crate::product::coding_models::{
    CodingExecutionAttempt, TestCommand, TestCommandStatus, TestPlan, TestPlanStep,
    TestingOverallStatus, TestingReport, TestingStepResult,
};
use crate::product::coding_workspace_engine::CodingExecutionContext;
use crate::product::test_executor::{
    TestCommandSpec, TestExecutorError, execute_test_command, infer_test_commands,
};

pub const TESTER_TOOL_FAILURE_LIMIT: usize = 3;

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
            timeout: Duration::from_secs(DEFAULT_PROVIDER_TIMEOUT_SECS),
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
    #[error("tester plan invalid: {0}")]
    Plan(String),
    #[error(transparent)]
    TestExecutor(#[from] TestExecutorError),
}

#[derive(Debug, Clone, Deserialize)]
struct ProviderTestPlanPayload {
    summary: String,
    #[serde(default)]
    context_warnings: Vec<String>,
    #[serde(default)]
    assumptions: Vec<String>,
    steps: Vec<TestPlanStep>,
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

pub fn build_tester_plan_prompt(
    attempt: &CodingExecutionAttempt,
    evaluation_context_json: &str,
) -> String {
    format!(
        "Tester Provider Runtime\n\
         Phase: plan_tests -> execute_test_plan\n\
         Project: {}\n\
         Issue: {}\n\
         Work Item: {}\n\
         Attempt: {}\n\
         Branch: {}\n\
         \n\
         [openspec_contract]\n\
         - 依据 Story Spec、Design Spec、Work Item、diff 与 project rules 设计验证计划。\n\
         - 如果 Story Spec、Design Spec、Work Item 之间存在冲突，必须 blocked 或请求人工澄清。\n\
         - 先输出 TestPlan JSON，不要直接声称测试通过。\n\
         \n\
         [superpowers_contract]\n\
         - 先证据后结论；不要用未执行的推断替代验证证据。\n\
         - 验证前置：执行 execute_test_plan 后，每个 required step 都必须有证据。\n\
         \n\
         工具与 step 绑定:\n\
         - plan_tests 阶段只生成 TestPlan。\n\
         - execute_test_plan 阶段调用 run_command/read_file/list_files/search_code 时必须在 input 中携带 step_id。\n\
         - 无 step_id 的工具结果只能进入 unplanned_commands 或 unplanned evidence，不能满足 required step。\n\
         - 不存在的 step_id 不能满足 required step。\n\
         \n\
         通用项目约束:\n\
         - Aria 是通用项目工作台，不要硬编码某种语言或包管理器。\n\
         - 不要默认 pnpm、cargo、pytest、npm 或任何单一生态；必须从上下文和仓库证据中决策。\n\
         \n\
         Evaluation Context JSON:\n\
         ```json\n{}\n```\n",
        attempt.project_id,
        attempt.issue_id,
        attempt.work_item_id,
        attempt.id,
        attempt.branch_name,
        evaluation_context_json
    )
}

pub fn build_tester_plan_repair_prompt(raw_output: &str, parse_error: &str) -> String {
    format!(
        "Tester Provider Runtime\n\
         Phase: plan_tests_repair\n\
         The previous plan_tests output could not be parsed as TestPlan JSON.\n\
         Parse error: {parse_error}\n\
         只返回合法 JSON。不要使用 Markdown 代码块，不要解释。\n\
         Required shape:\n\
         {{\"summary\":\"...\",\"context_warnings\":[],\"assumptions\":[],\"steps\":[{{\"id\":\"...\",\"title\":\"...\",\"intent\":\"...\",\"required\":true,\"tool\":\"run_command|read_file|list_files|search_code|provider_managed\",\"risk_level\":\"low|medium|high\",\"command_or_tool_input\":{{}},\"evidence_expectation\":\"...\"}}]}}\n\
         Previous output:\n\
         {raw_output}"
    )
}

pub fn build_tester_execute_repair_prompt(
    raw_output: &str,
    missing_required_steps: &[String],
) -> String {
    format!(
        "Tester Provider Runtime\n\
         Phase: execute_test_plan_repair\n\
         The previous execute_test_plan output did not provide valid step_results for every required step.\n\
         Missing required steps: {missing_required_steps:?}\n\
         Return only JSON: {{\"step_results\":[{{\"step_id\":\"...\",\"status\":\"passed|failed|blocked|skipped\",\"evidence_refs\":[\"...\"],\"provider_analysis\":\"...\"}}]}}\n\
         Previous output:\n\
         {raw_output}"
    )
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
        plan_id: None,
        plan_summary: None,
        steps: Vec::new(),
        unplanned_commands: Vec::new(),
        unplanned_evidence: Vec::new(),
        missing_required_steps: Vec::new(),
        skipped_required_steps: Vec::new(),
        context_warnings: Vec::new(),
        raw_provider_output_ref: None,
    }
}

pub fn parse_test_plan_payload(
    attempt_id: &str,
    plan_id: &str,
    raw_output: &str,
    raw_provider_output_ref: Option<String>,
) -> Result<TestPlan, TesterAgentError> {
    let json_text = extract_json_payload(raw_output)
        .ok_or_else(|| TesterAgentError::Plan("missing_json_object".to_string()))?;
    let payload: ProviderTestPlanPayload = serde_json::from_str(&json_text)
        .map_err(|error| TesterAgentError::Plan(format!("invalid_json: {error}")))?;
    validate_test_plan_payload(&payload)?;
    Ok(TestPlan {
        id: plan_id.to_string(),
        attempt_id: attempt_id.to_string(),
        summary: payload.summary,
        context_warnings: payload.context_warnings,
        assumptions: payload.assumptions,
        steps: payload.steps,
        created_at: Utc::now().to_rfc3339(),
        raw_provider_output_ref,
    })
}

pub fn build_plan_based_testing_report(
    report_id: &str,
    attempt_id: &str,
    plan: &TestPlan,
    steps: Vec<TestingStepResult>,
    unplanned_commands: Vec<TestCommand>,
    provider_claim: Option<Value>,
    raw_provider_output_ref: Option<String>,
) -> TestingReport {
    let mut missing_required_steps = Vec::new();
    let mut skipped_required_steps = Vec::new();
    let mut required_failed = false;
    let mut optional_failed = false;

    for plan_step in &plan.steps {
        let result = steps.iter().find(|result| result.step_id == plan_step.id);
        match (plan_step.required, result.map(|result| &result.status)) {
            (true, None) => missing_required_steps.push(plan_step.id.clone()),
            (true, Some(TestCommandStatus::Blocked)) => {
                skipped_required_steps.push(plan_step.id.clone());
            }
            (true, Some(TestCommandStatus::Failed | TestCommandStatus::TimedOut)) => {
                required_failed = true;
            }
            (
                false,
                Some(
                    TestCommandStatus::Failed
                    | TestCommandStatus::TimedOut
                    | TestCommandStatus::Blocked,
                ),
            ) => {
                optional_failed = true;
            }
            _ => {}
        }
    }

    let overall_status = if !missing_required_steps.is_empty() || !skipped_required_steps.is_empty()
    {
        TestingOverallStatus::Blocked
    } else if required_failed {
        TestingOverallStatus::Failed
    } else if !plan.context_warnings.is_empty() || optional_failed {
        TestingOverallStatus::PassedWithWarnings
    } else {
        TestingOverallStatus::Passed
    };

    TestingReport {
        id: report_id.to_string(),
        attempt_id: attempt_id.to_string(),
        commands: unplanned_commands.clone(),
        overall_status,
        provider_claim,
        backend_verified: true,
        started_at: Utc::now().to_rfc3339(),
        completed_at: Some(Utc::now().to_rfc3339()),
        plan_id: Some(plan.id.clone()),
        plan_summary: Some(plan.summary.clone()),
        steps,
        unplanned_commands,
        unplanned_evidence: Vec::new(),
        missing_required_steps,
        skipped_required_steps,
        context_warnings: plan.context_warnings.clone(),
        raw_provider_output_ref,
    }
}

fn extract_json_payload(raw_output: &str) -> Option<String> {
    let trimmed = raw_output.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed.to_string());
    }

    let mut in_json_fence = false;
    let mut fenced_lines = Vec::new();
    for line in raw_output.lines() {
        let trimmed_line = line.trim();
        if trimmed_line.starts_with("```") {
            if in_json_fence {
                return Some(fenced_lines.join("\n"));
            }
            let fence_label = trimmed_line.trim_start_matches('`').trim();
            if fence_label.is_empty() || fence_label.eq_ignore_ascii_case("json") {
                in_json_fence = true;
                fenced_lines.clear();
            }
            continue;
        }
        if in_json_fence {
            fenced_lines.push(line);
        }
    }

    let start = raw_output.find('{')?;
    let end = raw_output.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(raw_output[start..=end].to_string())
}

fn validate_test_plan_payload(payload: &ProviderTestPlanPayload) -> Result<(), TesterAgentError> {
    require_non_empty("summary", &payload.summary)?;
    if payload.steps.is_empty() {
        return Err(TesterAgentError::Plan("steps_empty".to_string()));
    }
    let mut seen_step_ids = std::collections::HashSet::new();
    for step in &payload.steps {
        require_non_empty("step.id", &step.id)?;
        if !seen_step_ids.insert(step.id.clone()) {
            return Err(TesterAgentError::Plan(format!(
                "duplicate_step_id: {}",
                step.id
            )));
        }
        require_non_empty("step.title", &step.title)?;
        require_non_empty("step.intent", &step.intent)?;
        require_non_empty("step.evidence_expectation", &step.evidence_expectation)?;
    }
    Ok(())
}

fn require_non_empty(field: &str, value: &str) -> Result<(), TesterAgentError> {
    if value.trim().is_empty() {
        return Err(TesterAgentError::Plan(format!("{field}_empty")));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::coding_models::{
        CodingAttemptStatus, CodingExecutionStage, TestPlan, TestPlanRiskLevel, TestPlanStep,
        TestPlanTool, TestingStepResult,
    };
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    fn test_attempt() -> CodingExecutionAttempt {
        CodingExecutionAttempt {
            id: "coding_attempt_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            attempt_no: 1,
            status: CodingAttemptStatus::Running,
            stage: CodingExecutionStage::Testing,
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            rework_count: 0,
            max_auto_rework: 2,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: "2026-06-10T00:00:00Z".to_string(),
            updated_at: "2026-06-10T00:00:00Z".to_string(),
            completed_at: None,
        }
    }

    #[test]
    fn tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools() {
        let prompt = build_tester_plan_prompt(
            &test_attempt(),
            r#"{"story_specs":[],"design_specs":[],"work_item":{}}"#,
        );

        assert!(prompt.contains("plan_tests"));
        assert!(prompt.contains("execute_test_plan"));
        assert!(prompt.contains("[openspec_contract]"));
        assert!(prompt.contains("[superpowers_contract]"));
        assert!(prompt.contains("Story Spec"));
        assert!(prompt.contains("Design Spec"));
        assert!(prompt.contains("Work Item"));
        assert!(prompt.contains("step_id"));
        assert!(prompt.contains("不要硬编码某种语言或包管理器"));
    }

    #[test]
    fn parses_test_plan_from_provider_json_and_blocks_missing_required_step() {
        let raw_output = r#"
Tester plan:

```json
{
  "summary": "unit and security checks",
  "context_warnings": [],
  "assumptions": [],
  "steps": [
    {
      "id": "unit",
      "title": "Unit tests",
      "intent": "verify unit behavior",
      "required": true,
      "tool": "run_command",
      "risk_level": "low",
      "command_or_tool_input": { "command": ["cargo", "test", "--locked", "--lib", "unit"] },
      "evidence_expectation": "exit 0"
    },
    {
      "id": "security",
      "title": "Security review",
      "intent": "verify sensitive output handling",
      "required": true,
      "tool": "provider_managed",
      "risk_level": "medium",
      "command_or_tool_input": { "check": "manual" },
      "evidence_expectation": "provider analysis with evidence"
    }
  ]
}
```
"#;

        let plan = parse_test_plan_payload(
            "coding_attempt_0001",
            "test_plan_0001",
            raw_output,
            Some("provider-raw/testing/plan_tests_0001.txt".to_string()),
        )
        .unwrap();

        assert_eq!(plan.attempt_id, "coding_attempt_0001");
        assert_eq!(plan.id, "test_plan_0001");
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].id, "unit");
        assert_eq!(plan.steps[1].id, "security");

        let report = build_plan_based_testing_report(
            "testing_report_0001",
            "coding_attempt_0001",
            &plan,
            vec![TestingStepResult {
                step_id: "unit".to_string(),
                status: TestCommandStatus::Passed,
                evidence_refs: vec!["unit.stdout.log".to_string()],
                command: Some(vec![
                    "cargo".to_string(),
                    "test".to_string(),
                    "--locked".to_string(),
                    "--lib".to_string(),
                    "unit".to_string(),
                ]),
                provider_analysis: None,
            }],
            Vec::new(),
            None,
            Some("provider-raw/testing/execute_tests_0001.txt".to_string()),
        );

        assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
        assert_eq!(report.plan_id.as_deref(), Some("test_plan_0001"));
        assert_eq!(report.missing_required_steps, vec!["security"]);
    }

    #[test]
    fn tester_plan_repair_prompt_includes_raw_output_and_schema_error() {
        let prompt = build_tester_plan_repair_prompt(
            "## 最终测试报告\n无法执行 cargo",
            "missing_json_object",
        );

        assert!(prompt.contains("Phase: plan_tests_repair"));
        assert!(prompt.contains("missing_json_object"));
        assert!(prompt.contains("## 最终测试报告"));
        assert!(prompt.contains("\"summary\""));
        assert!(prompt.contains("\"steps\""));
        assert!(prompt.contains("只返回合法 JSON"));
    }

    #[test]
    fn test_tool_call_without_step_id_is_unplanned_and_does_not_pass_required_step() {
        let plan = TestPlan {
            id: "test_plan_0001".to_string(),
            attempt_id: "coding_attempt_0001".to_string(),
            summary: "unit checks".to_string(),
            context_warnings: Vec::new(),
            assumptions: Vec::new(),
            steps: vec![TestPlanStep {
                id: "unit".to_string(),
                title: "Unit tests".to_string(),
                intent: "verify unit behavior".to_string(),
                required: true,
                tool: TestPlanTool::RunCommand,
                risk_level: TestPlanRiskLevel::Low,
                command_or_tool_input: serde_json::json!({"command": ["true"]}),
                evidence_expectation: "exit 0".to_string(),
                related_requirements: Vec::new(),
                related_design_constraints: Vec::new(),
                related_work_item_tasks: Vec::new(),
            }],
            created_at: "2026-06-10T00:00:00Z".to_string(),
            raw_provider_output_ref: None,
        };
        let unplanned_command = TestCommand {
            command: vec!["true".to_string()],
            cwd: PathBuf::from("/tmp/worktree"),
            exit_code: Some(0),
            duration_ms: 1,
            stdout_ref: "stdout.log".to_string(),
            stderr_ref: "stderr.log".to_string(),
            status: TestCommandStatus::Passed,
        };

        let report = build_plan_based_testing_report(
            "testing_report_0001",
            "coding_attempt_0001",
            &plan,
            Vec::new(),
            vec![unplanned_command],
            None,
            None,
        );

        assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
        assert_eq!(report.missing_required_steps, vec!["unit"]);
        assert!(report.steps.is_empty());
        assert_eq!(report.unplanned_commands.len(), 1);
    }
}
