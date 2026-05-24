use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
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
    if worktree_path.join("Cargo.toml").is_file() {
        return vec![TestCommandSpec {
            id: "rust".to_string(),
            command: vec![
                "cargo".to_string(),
                "test".to_string(),
                "--locked".to_string(),
                "-j".to_string(),
                "1".to_string(),
            ],
        }];
    }
    if worktree_path.join("pyproject.toml").is_file() || worktree_path.join("setup.py").is_file() {
        return vec![TestCommandSpec {
            id: "python".to_string(),
            command: vec!["uv".to_string(), "run".to_string(), "pytest".to_string()],
        }];
    }
    if worktree_path.join("package.json").is_file() {
        return vec![TestCommandSpec {
            id: "node".to_string(),
            command: vec!["pnpm".to_string(), "test".to_string()],
        }];
    }
    Vec::new()
}

pub async fn execute_test_command(
    spec: &TestCommandSpec,
    worktree_path: impl AsRef<Path>,
) -> Result<TestCommand, TestExecutorError> {
    execute_test_command_with_timeout(spec, worktree_path, DEFAULT_TEST_TIMEOUT).await
}

async fn execute_test_command_with_timeout(
    spec: &TestCommandSpec,
    worktree_path: impl AsRef<Path>,
    timeout_duration: Duration,
) -> Result<TestCommand, TestExecutorError> {
    if spec.command.is_empty() {
        return Err(TestExecutorError::EmptyCommand);
    }
    validate_command_id(&spec.id)?;
    let worktree_path = worktree_path.as_ref();
    let started = std::time::Instant::now();
    let stdout_ref = artifact_ref(&spec.id, "stdout");
    let stderr_ref = artifact_ref(&spec.id, "stderr");
    let stdout_path = worktree_path.join(&stdout_ref);
    let stderr_path = worktree_path.join(&stderr_ref);
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
                stdout_ref: stdout_ref.to_string_lossy().to_string(),
                stderr_ref: stderr_ref.to_string_lossy().to_string(),
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
                stdout_ref: stdout_ref.to_string_lossy().to_string(),
                stderr_ref: stderr_ref.to_string_lossy().to_string(),
                status: TestCommandStatus::TimedOut,
            })
        }
    }
}

pub async fn run_all_tests(
    attempt_id: &str,
    worktree_path: impl AsRef<Path>,
    specs: &[TestCommandSpec],
) -> Result<TestingReport, TestExecutorError> {
    let started_at = Utc::now().to_rfc3339();
    let worktree_path = worktree_path.as_ref();
    let mut commands = Vec::with_capacity(specs.len());
    for spec in specs {
        commands.push(execute_test_command(spec, worktree_path).await?);
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
    })
}

fn artifact_ref(command_id: &str, stream: &str) -> PathBuf {
    PathBuf::from(".aria")
        .join("coding-artifacts")
        .join("test-output")
        .join(format!("{command_id}.{stream}.log"))
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
