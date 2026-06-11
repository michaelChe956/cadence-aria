use std::path::{Component, Path};
use std::time::Duration;

use chrono::Utc;
use serde_json::Value;
use thiserror::Error;
use tokio::process::Command;
use tokio::time::timeout;

use crate::product::coding_models::{
    TestCommand, TestCommandStatus, TestingOverallStatus, TestingReport,
};

const DEFAULT_TEST_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCommandSpec {
    pub id: String,
    pub command: Vec<String>,
}

#[derive(Debug, Error)]
pub enum TestExecutorError {
    #[error("test command is empty")]
    EmptyCommand,
    #[error("invalid test command id: {0}")]
    InvalidCommandId(String),
    #[error("test executor io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn discover_test_commands(worktree_path: impl AsRef<Path>) -> Vec<TestCommandSpec> {
    let worktree_path = worktree_path.as_ref();
    let mut specs = Vec::new();
    if worktree_path.join("Cargo.toml").is_file() {
        specs.push(TestCommandSpec {
            id: "rust".to_string(),
            command: vec![
                "cargo".to_string(),
                "test".to_string(),
                "--locked".to_string(),
            ],
        });
    }
    if worktree_path.join("pyproject.toml").is_file() || worktree_path.join("setup.py").is_file() {
        specs.push(TestCommandSpec {
            id: "python".to_string(),
            command: vec!["uv".to_string(), "run".to_string(), "pytest".to_string()],
        });
    }
    specs.extend(package_test_commands(worktree_path, false));
    specs
}

pub fn infer_test_commands(worktree_path: impl AsRef<Path>) -> Vec<TestCommandSpec> {
    let worktree_path = worktree_path.as_ref();
    let mut specs = Vec::new();
    if worktree_path.join("Cargo.toml").is_file() {
        specs.push(TestCommandSpec {
            id: "inferred_rust".to_string(),
            command: vec!["cargo".to_string(), "test".to_string()],
        });
    }
    if worktree_path.join("pytest.ini").is_file()
        || pyproject_declares_pytest(&worktree_path.join("pyproject.toml"))
    {
        specs.push(TestCommandSpec {
            id: "inferred_python".to_string(),
            command: vec!["pytest".to_string()],
        });
    }
    specs.extend(package_test_commands(worktree_path, true));
    specs
}

pub fn planned_test_commands_from_markdown(markdown: &str) -> Vec<TestCommandSpec> {
    let mut commands = Vec::new();
    let mut in_verification_block = false;
    let mut in_command_fence: Option<String> = None;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(fence) = in_command_fence.as_deref() {
            if is_closing_fence(trimmed, fence) {
                in_command_fence = None;
                continue;
            }
            let command = trimmed.strip_prefix("$ ").unwrap_or(trimmed).to_string();
            if let Some(command) = normalize_planned_command(&command)
                && !commands.contains(&command)
            {
                commands.push(command);
            }
            continue;
        }
        if trimmed.starts_with('#') {
            in_verification_block = trimmed.contains("验证命令");
            continue;
        }
        if is_verification_label(trimmed) {
            in_verification_block = true;
            continue;
        }
        if in_verification_block && is_non_verification_label(trimmed) {
            in_verification_block = false;
            continue;
        }
        if !in_verification_block {
            continue;
        }
        if let Some(fence) = code_fence_marker(trimmed) {
            in_command_fence = Some(fence);
            continue;
        }
        for command in inline_code_spans(trimmed) {
            let Some(command) = normalize_planned_command(&command) else {
                continue;
            };
            if commands.contains(&command) {
                continue;
            }
            commands.push(command);
        }
    }

    commands
        .into_iter()
        .enumerate()
        .map(|(index, command)| TestCommandSpec {
            id: format!("planned_{:03}", index + 1),
            command,
        })
        .collect()
}

pub async fn execute_test_command(
    spec: &TestCommandSpec,
    worktree_path: impl AsRef<Path>,
    artifact_output_root: impl AsRef<Path>,
) -> Result<TestCommand, TestExecutorError> {
    execute_test_command_with_timeout(
        spec,
        worktree_path,
        artifact_output_root,
        DEFAULT_TEST_TIMEOUT,
    )
    .await
}

async fn execute_test_command_with_timeout(
    spec: &TestCommandSpec,
    worktree_path: impl AsRef<Path>,
    artifact_output_root: impl AsRef<Path>,
    timeout_duration: Duration,
) -> Result<TestCommand, TestExecutorError> {
    if spec.command.is_empty() {
        return Err(TestExecutorError::EmptyCommand);
    }
    validate_command_id(&spec.id)?;
    let worktree_path = worktree_path.as_ref();
    let artifact_output_root = artifact_output_root.as_ref();
    let started = std::time::Instant::now();
    let stdout_ref = artifact_ref(&spec.id, "stdout");
    let stderr_ref = artifact_ref(&spec.id, "stderr");
    let stdout_path = artifact_output_root.join(&stdout_ref);
    let stderr_path = artifact_output_root.join(&stderr_ref);
    if let Some(parent) = stdout_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut command = Command::new(&spec.command[0]);
    command
        .args(&spec.command[1..])
        .current_dir(worktree_path)
        .kill_on_drop(true);
    let result = timeout(timeout_duration, command.output()).await;
    let duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;

    match result {
        Ok(output_result) => {
            let output = output_result?;
            tokio::fs::write(&stdout_path, &output.stdout).await?;
            tokio::fs::write(&stderr_path, &output.stderr).await?;
            let exit_code = output.status.code();
            let status = if output.status.success() {
                TestCommandStatus::Passed
            } else {
                TestCommandStatus::Failed
            };
            Ok(TestCommand {
                command: spec.command.clone(),
                cwd: worktree_path.to_path_buf(),
                exit_code,
                duration_ms,
                stdout_ref,
                stderr_ref,
                status,
            })
        }
        Err(_) => {
            tokio::fs::write(&stdout_path, b"").await?;
            tokio::fs::write(&stderr_path, b"test command timed out").await?;
            Ok(TestCommand {
                command: spec.command.clone(),
                cwd: worktree_path.to_path_buf(),
                exit_code: None,
                duration_ms,
                stdout_ref,
                stderr_ref,
                status: TestCommandStatus::TimedOut,
            })
        }
    }
}

pub async fn run_all_tests(
    attempt_id: &str,
    worktree_path: impl AsRef<Path>,
    artifact_output_root: impl AsRef<Path>,
    specs: &[TestCommandSpec],
) -> Result<TestingReport, TestExecutorError> {
    let started_at = Utc::now().to_rfc3339();
    let worktree_path = worktree_path.as_ref();
    let artifact_output_root = artifact_output_root.as_ref();
    let mut commands = Vec::with_capacity(specs.len());
    for spec in specs {
        commands.push(execute_test_command(spec, worktree_path, artifact_output_root).await?);
    }
    let overall_status = if commands.is_empty() {
        TestingOverallStatus::Blocked
    } else if commands
        .iter()
        .all(|command| command.status == TestCommandStatus::Passed)
    {
        TestingOverallStatus::Passed
    } else {
        TestingOverallStatus::Failed
    };

    Ok(TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        commands,
        overall_status,
        provider_claim: None,
        backend_verified: true,
        started_at,
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
    })
}

fn artifact_ref(command_id: &str, stream: &str) -> String {
    format!("{command_id}.{stream}.log")
}

fn package_test_commands(worktree_path: &Path, inferred: bool) -> Vec<TestCommandSpec> {
    let mut specs = Vec::new();
    push_package_test_command(worktree_path, None, inferred, &mut specs);

    let Ok(entries) = std::fs::read_dir(worktree_path) else {
        return specs;
    };
    let mut child_dirs = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            (!ignored_test_discovery_dir(&name)).then_some((name, entry.path()))
        })
        .collect::<Vec<_>>();
    child_dirs.sort_by(|left, right| left.0.cmp(&right.0));

    for (relative_dir, path) in child_dirs {
        push_package_test_command(&path, Some(&relative_dir), inferred, &mut specs);
    }
    specs
}

fn push_package_test_command(
    package_dir: &Path,
    relative_dir: Option<&str>,
    inferred: bool,
    specs: &mut Vec<TestCommandSpec>,
) {
    if !package_json_has_test_script(&package_dir.join("package.json")) {
        return;
    }
    let prefix = if inferred { "inferred_node" } else { "node" };
    let id = relative_dir
        .map(|dir| format!("{prefix}_{}", sanitize_command_id_fragment(dir)))
        .unwrap_or_else(|| prefix.to_string());
    let command = relative_dir
        .map(|dir| {
            vec![
                "pnpm".to_string(),
                "-C".to_string(),
                dir.to_string(),
                "test".to_string(),
            ]
        })
        .unwrap_or_else(|| vec!["pnpm".to_string(), "test".to_string()]);
    specs.push(TestCommandSpec { id, command });
}

fn ignored_test_discovery_dir(name: &str) -> bool {
    matches!(
        name,
        ".aria" | ".git" | ".worktrees" | "node_modules" | "target" | "dist"
    )
}

fn sanitize_command_id_fragment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn package_json_has_test_script(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return false;
    };
    value
        .get("scripts")
        .and_then(|scripts| scripts.get("test"))
        .and_then(Value::as_str)
        .is_some_and(|script| !script.trim().is_empty())
}

fn pyproject_declares_pytest(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "[tool.pytest]" || trimmed == "[tool.pytest.ini_options]"
    })
}

fn validate_command_id(command_id: &str) -> Result<(), TestExecutorError> {
    if command_id.is_empty()
        || !command_id
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || value == '_' || value == '-')
    {
        return Err(TestExecutorError::InvalidCommandId(command_id.to_string()));
    }
    Ok(())
}

fn is_verification_label(line: &str) -> bool {
    line.starts_with("验证命令")
        || line.starts_with("主验证命令")
        || line.starts_with("辅助检查命令")
}

fn is_non_verification_label(line: &str) -> bool {
    line.ends_with('：')
        && !line.starts_with('-')
        && !line.starts_with('*')
        && !line.contains("命令")
        && !is_verification_label(line)
}

fn inline_code_spans(line: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('`') else {
            break;
        };
        let value = after_start[..end].trim();
        if !value.is_empty() {
            spans.push(value.to_string());
        }
        rest = &after_start[end + 1..];
    }
    spans
}

fn normalize_planned_command(command: &str) -> Option<Vec<String>> {
    let parts = split_simple_command(command)?;

    if let Some(parts) = normalize_cd_pnpm_command(&parts) {
        return Some(parts);
    }

    allowed_planned_command_parts(&parts).then_some(parts)
}

fn normalize_cd_pnpm_command(parts: &[String]) -> Option<Vec<String>> {
    if parts.len() < 4
        || parts.first().map(String::as_str) != Some("cd")
        || parts.get(2).map(String::as_str) != Some("&&")
        || parts.get(3).map(String::as_str) != Some("pnpm")
    {
        return None;
    }
    let package_dir = parts.get(1)?;
    if !is_safe_package_dir_argument(package_dir) {
        return None;
    }
    let mut normalized = vec![
        "pnpm".to_string(),
        "-C".to_string(),
        package_dir.to_string(),
    ];
    normalized.extend(parts.iter().skip(4).cloned());
    allowed_planned_command_parts(&normalized).then_some(normalized)
}

fn allowed_planned_command_parts(parts: &[String]) -> bool {
    match parts.first().map(String::as_str) {
        Some("cargo" | "uv" | "pnpm" | "node" | "python" | "python3" | "pytest") => true,
        Some("git") => parts.get(1).is_some_and(|subcommand| subcommand == "diff"),
        _ => false,
    }
}

fn is_safe_package_dir_argument(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !value.starts_with('-')
        && path.is_relative()
        && path.components().all(|component| match component {
            Component::Normal(value) => value
                .to_string_lossy()
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')),
            _ => false,
        })
}

fn split_simple_command(command: &str) -> Option<Vec<String>> {
    let parts: Vec<String> = command
        .split_whitespace()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect();
    (!parts.is_empty()).then_some(parts)
}

fn code_fence_marker(line: &str) -> Option<String> {
    let first = line.as_bytes().first().copied()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let len = line
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == first)
        .count();
    (len >= 3).then(|| std::iter::repeat_n(char::from(first), len).collect())
}

fn is_closing_fence(line: &str, fence: &str) -> bool {
    line.starts_with(fence) && line[fence.len()..].trim().is_empty()
}
