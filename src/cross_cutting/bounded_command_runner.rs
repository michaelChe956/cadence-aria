use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::process_manager::{ManagedProcess, ManagedProcessChild, ProcessManager};
use crate::protocol::provider_errors::ProviderErrorCode;

#[derive(Debug, Clone)]
pub struct BoundedCommandRequest {
    pub executable: String,
    pub argv: Vec<String>,
    pub working_dir: PathBuf,
    pub timeout: Duration,
    pub cancellation: CancellationToken,
    pub environment: BTreeMap<String, String>,
    pub stdout_limit: usize,
    pub stderr_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedCommandResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BoundedCommandError {
    #[error("command missing: {executable}: {details}")]
    CommandMissing { executable: String, details: String },
    #[error("command I/O error: {details}")]
    Io { details: String },
}

#[async_trait::async_trait]
pub trait BoundedCommandRunner: Send + Sync {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TokioBoundedCommandRunner;

impl TokioBoundedCommandRunner {
    pub async fn run_inherited(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        self.run_with_environment(request, true).await
    }

    async fn run_with_environment(
        &self,
        request: BoundedCommandRequest,
        inherit_environment: bool,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let started = Instant::now();
        let argv = request.argv.iter().map(String::as_str).collect::<Vec<_>>();
        let process = if inherit_environment {
            ProcessManager::spawn(
                &request.executable,
                &argv,
                &request.working_dir,
                &request.environment,
                request.cancellation.clone(),
            )
            .await
        } else {
            ProcessManager::spawn_isolated(
                &request.executable,
                &argv,
                &request.working_dir,
                &request.environment,
                request.cancellation.clone(),
            )
            .await
        }
        .map_err(|error| {
            if error.code == ProviderErrorCode::ProviderCommandMissing {
                BoundedCommandError::CommandMissing {
                    executable: request.executable.clone(),
                    details: error.details,
                }
            } else {
                BoundedCommandError::Io {
                    details: error.stderr,
                }
            }
        })?;

        run_spawned_process(process, request, started).await
    }
}

#[async_trait::async_trait]
impl BoundedCommandRunner for TokioBoundedCommandRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        self.run_with_environment(request, false).await
    }
}

async fn run_spawned_process(
    process: ManagedProcess,
    request: BoundedCommandRequest,
    started: Instant,
) -> Result<BoundedCommandResult, BoundedCommandError> {
    let ManagedProcess {
        stdin,
        stdout,
        stderr,
        mut child,
    } = process;
    drop(stdin);

    let stdout_task = tokio::spawn(read_limited(stdout, request.stdout_limit));
    let stderr_task = tokio::spawn(read_limited(stderr, request.stderr_limit));

    enum Completion {
        Exited(std::io::Result<std::process::ExitStatus>),
        TimedOut,
        Cancelled,
    }

    let completion = tokio::select! {
        status = child.wait() => Completion::Exited(status),
        _ = tokio::time::sleep(request.timeout) => Completion::TimedOut,
        _ = request.cancellation.cancelled() => Completion::Cancelled,
    };

    let (exit_code, timed_out, cancelled) = match completion {
        Completion::Exited(status) => (
            status
                .map_err(|error| BoundedCommandError::Io {
                    details: error.to_string(),
                })?
                .code(),
            false,
            false,
        ),
        Completion::TimedOut => {
            terminate_process(&mut child).await;
            (None, true, false)
        }
        Completion::Cancelled => {
            terminate_process(&mut child).await;
            (None, false, true)
        }
    };

    let stdout = join_reader(stdout_task).await?;
    let stderr = join_reader(stderr_task).await?;

    Ok(BoundedCommandResult {
        exit_code,
        stdout: String::from_utf8_lossy(&stdout.bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
        timed_out,
        cancelled,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
        duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    })
}

async fn terminate_process(child: &mut ManagedProcessChild) {
    child.terminate().await;
}

struct LimitedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn read_limited<R>(mut reader: R, limit: usize) -> std::io::Result<LimitedOutput>
where
    R: AsyncRead + Unpin,
{
    let mut bytes = Vec::with_capacity(limit.min(8192));
    let mut truncated = false;
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(bytes.len());
        let retained = remaining.min(read);
        bytes.extend_from_slice(&chunk[..retained]);
        truncated |= retained < read;
    }
    Ok(LimitedOutput { bytes, truncated })
}

async fn join_reader(
    task: tokio::task::JoinHandle<std::io::Result<LimitedOutput>>,
) -> Result<LimitedOutput, BoundedCommandError> {
    task.await
        .map_err(|error| BoundedCommandError::Io {
            details: error.to_string(),
        })?
        .map_err(|error| BoundedCommandError::Io {
            details: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::Duration;

    use tokio_util::sync::CancellationToken;

    use super::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandRunner, TokioBoundedCommandRunner,
    };

    #[cfg(unix)]
    fn write_executable(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write command fixture");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fixture");
        path
    }

    #[cfg(unix)]
    fn request(executable: &Path, working_dir: &Path) -> BoundedCommandRequest {
        BoundedCommandRequest {
            executable: executable.to_string_lossy().to_string(),
            argv: Vec::new(),
            working_dir: working_dir.to_path_buf(),
            timeout: Duration::from_secs(2),
            cancellation: CancellationToken::new(),
            environment: BTreeMap::new(),
            stdout_limit: 1024,
            stderr_limit: 1024,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_command_runner_reports_command_missing() {
        let working_dir = tempfile::tempdir().expect("working dir");
        let command = working_dir.path().join("missing-provider");

        let error = TokioBoundedCommandRunner
            .run(request(&command, working_dir.path()))
            .await
            .expect_err("missing command must fail");

        assert!(matches!(error, BoundedCommandError::CommandMissing { .. }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_command_runner_collects_successful_stdout() {
        let working_dir = tempfile::tempdir().expect("working dir");
        let command = write_executable(
            working_dir.path(),
            "stdout-provider",
            "printf 'claude 1.2.3\\n'",
        );

        let result = TokioBoundedCommandRunner
            .run(request(&command, working_dir.path()))
            .await
            .expect("command succeeds");

        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout, "claude 1.2.3\n");
        assert!(result.stderr.is_empty());
        assert!(!result.timed_out);
        assert!(!result.cancelled);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_command_runner_collects_version_written_to_stderr() {
        let working_dir = tempfile::tempdir().expect("working dir");
        let command = write_executable(
            working_dir.path(),
            "stderr-provider",
            "printf 'codex 0.124.0\\n' >&2",
        );

        let result = TokioBoundedCommandRunner
            .run(request(&command, working_dir.path()))
            .await
            .expect("command succeeds");

        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stderr, "codex 0.124.0\n");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_command_runner_preserves_non_zero_exit() {
        let working_dir = tempfile::tempdir().expect("working dir");
        let command = write_executable(
            working_dir.path(),
            "failing-provider",
            "printf 'nope' >&2\nexit 7",
        );

        let result = TokioBoundedCommandRunner
            .run(request(&command, working_dir.path()))
            .await
            .expect("spawn succeeds");

        assert_eq!(result.exit_code, Some(7));
        assert_eq!(result.stderr, "nope");
        assert!(!result.timed_out);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_command_runner_kills_process_on_timeout() {
        let working_dir = tempfile::tempdir().expect("working dir");
        let command = write_executable(working_dir.path(), "slow-provider", "sleep 5");
        let mut command_request = request(&command, working_dir.path());
        command_request.timeout = Duration::from_millis(30);

        let result = TokioBoundedCommandRunner
            .run(command_request)
            .await
            .expect("timeout is an execution result");

        assert!(result.timed_out);
        assert!(!result.cancelled);
        assert_eq!(result.exit_code, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_command_runner_kills_process_on_cancellation() {
        let working_dir = tempfile::tempdir().expect("working dir");
        let command = write_executable(working_dir.path(), "cancel-provider", "sleep 5");
        let cancellation = CancellationToken::new();
        let mut command_request = request(&command, working_dir.path());
        command_request.cancellation = cancellation.clone();

        let run = tokio::spawn(async move { TokioBoundedCommandRunner.run(command_request).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancellation.cancel();
        let result = run.await.expect("runner task").expect("cancel result");

        assert!(result.cancelled);
        assert!(!result.timed_out);
        assert_eq!(result.exit_code, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_command_runner_truncates_output_while_draining_process() {
        let working_dir = tempfile::tempdir().expect("working dir");
        let command = write_executable(
            working_dir.path(),
            "verbose-provider",
            "printf 'abcdefghij'; printf 'wxyz123456' >&2",
        );
        let mut command_request = request(&command, working_dir.path());
        command_request.stdout_limit = 4;
        command_request.stderr_limit = 4;

        let result = TokioBoundedCommandRunner
            .run(command_request)
            .await
            .expect("command succeeds");

        assert_eq!(result.stdout, "abcd");
        assert_eq!(result.stderr, "wxyz");
        assert!(result.stdout_truncated);
        assert!(result.stderr_truncated);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_command_runner_passes_argv_without_shell_interpretation() {
        let working_dir = tempfile::tempdir().expect("working dir");
        let command = write_executable(working_dir.path(), "argv-provider", "printf '%s' \"$1\"");
        let marker = working_dir.path().join("shell-injection-marker");
        let payload = format!("; touch {}", marker.display());
        let mut command_request = request(&command, working_dir.path());
        command_request.argv = vec![payload.clone()];

        let result = TokioBoundedCommandRunner
            .run(command_request)
            .await
            .expect("command succeeds");

        assert_eq!(result.stdout, payload);
        assert!(!marker.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_command_runner_exposes_only_allowlisted_environment() {
        let working_dir = tempfile::tempdir().expect("working dir");
        let command = write_executable(
            working_dir.path(),
            "env-provider",
            "printf '%s:%s' \"${ALLOWED-unset}\" \"${HOME-unset}\"",
        );
        let mut command_request = request(&command, working_dir.path());
        command_request
            .environment
            .insert("ALLOWED".to_string(), "visible".to_string());

        let result = TokioBoundedCommandRunner
            .run(command_request)
            .await
            .expect("command succeeds");

        assert_eq!(result.stdout, "visible:unset");
    }
}
