use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{ExitStatus, Stdio};

#[cfg(windows)]
use command_group::{AsyncCommandGroup, AsyncGroupChild};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;

#[cfg(unix)]
fn unix_process_group_signal_target(current_child_id: Option<u32>, pgid: i32) -> Option<i32> {
    (current_child_id == u32::try_from(pgid).ok()).then_some(pgid)
}

#[derive(Debug)]
pub struct ManagedProcess {
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
    pub child: ManagedProcessChild,
}

pub struct ManagedProcessChild {
    #[cfg(unix)]
    child: tokio::process::Child,
    #[cfg(windows)]
    child: AsyncGroupChild,
    #[cfg(unix)]
    pgid: Option<i32>,
}

impl std::fmt::Debug for ManagedProcessChild {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedProcessChild")
            .field("id", &self.id())
            .finish()
    }
}

impl ManagedProcessChild {
    pub fn spawn(command: &mut Command) -> std::io::Result<Self> {
        command.kill_on_drop(true);
        #[cfg(unix)]
        {
            command.process_group(0);
            let child = command.spawn()?;
            let pgid = child.id().and_then(|pid| i32::try_from(pid).ok());
            Ok(Self { child, pgid })
        }
        #[cfg(windows)]
        {
            let mut group = command.group();
            group.kill_on_drop(true);
            group.spawn().map(|child| Self { child })
        }
    }

    pub fn inner(&mut self) -> &mut tokio::process::Child {
        #[cfg(unix)]
        {
            &mut self.child
        }
        #[cfg(windows)]
        {
            self.child.inner()
        }
    }

    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn start_kill(&mut self) -> std::io::Result<()> {
        #[cfg(unix)]
        if let Some(pgid) = self
            .pgid
            .and_then(|pgid| unix_process_group_signal_target(self.child.id(), pgid))
        {
            let result = unsafe { libc::killpg(pgid, libc::SIGKILL) };
            if result == 0 {
                return Ok(());
            }
        }
        self.child.start_kill()
    }

    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        let status = self.child.wait().await;
        if status.is_ok() {
            #[cfg(unix)]
            {
                self.pgid = None;
            }
        }
        status
    }

    pub async fn terminate(&mut self) {
        let _ = self.start_kill();
        let _ = self.inner().start_kill();
        let _ = self.wait().await;
    }
}

impl Drop for ManagedProcessChild {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pgid) = self
            .pgid
            .take()
            .and_then(|pgid| unix_process_group_signal_target(self.child.id(), pgid))
        {
            unsafe {
                let _ = libc::killpg(pgid, libc::SIGKILL);
            }
            let _ = self.child.start_kill();
        }
    }
}

pub struct ProcessManager;

impl ProcessManager {
    pub async fn spawn(
        command: &str,
        args: &[&str],
        working_dir: &Path,
        env_vars: &BTreeMap<String, String>,
        _cancel: CancellationToken,
    ) -> Result<ManagedProcess, ProviderAdapterError> {
        Self::spawn_with_environment(command, args, working_dir, env_vars, true).await
    }

    pub async fn spawn_isolated(
        command: &str,
        args: &[&str],
        working_dir: &Path,
        env_vars: &BTreeMap<String, String>,
        _cancel: CancellationToken,
    ) -> Result<ManagedProcess, ProviderAdapterError> {
        Self::spawn_with_environment(command, args, working_dir, env_vars, false).await
    }

    async fn spawn_with_environment(
        command: &str,
        args: &[&str],
        working_dir: &Path,
        env_vars: &BTreeMap<String, String>,
        inherit_environment: bool,
    ) -> Result<ManagedProcess, ProviderAdapterError> {
        if !command_is_resolvable(command, working_dir, env_vars) {
            return Err(command_missing(command));
        }

        let mut command_builder = Command::new(command);
        if !inherit_environment {
            command_builder.env_clear();
        }
        command_builder
            .args(args)
            .current_dir(working_dir)
            .envs(env_vars)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = ManagedProcessChild::spawn(&mut command_builder)
            .map_err(|error| map_spawn_error(command, error))?;

        let stdin = child.inner().stdin.take().ok_or_else(missing_stdin_pipe)?;
        let stdout = child
            .inner()
            .stdout
            .take()
            .ok_or_else(missing_stdout_pipe)?;
        let stderr = child
            .inner()
            .stderr
            .take()
            .ok_or_else(missing_stderr_pipe)?;

        Ok(ManagedProcess {
            stdin,
            stdout,
            stderr,
            child,
        })
    }
}

fn map_spawn_error(command: &str, error: std::io::Error) -> ProviderAdapterError {
    if is_command_missing_error(&error) {
        command_missing(command)
    } else {
        ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
    }
}

fn command_is_resolvable(
    command: &str,
    working_dir: &Path,
    env_vars: &BTreeMap<String, String>,
) -> bool {
    let command_path = Path::new(command);
    if command_path.is_absolute() {
        return command_path.exists();
    }
    if command_path.components().count() > 1 {
        return working_dir.join(command_path).exists();
    }

    path_env(env_vars)
        .map(|paths| {
            std::env::split_paths(&paths).any(|directory| directory.join(command).exists())
        })
        .unwrap_or(false)
}

fn path_env(env_vars: &BTreeMap<String, String>) -> Option<OsString> {
    env_vars
        .get("PATH")
        .map(OsString::from)
        .or_else(|| std::env::var_os("PATH"))
}

fn command_missing(command: &str) -> ProviderAdapterError {
    ProviderAdapterError::command_missing(format!("provider command not found: {command}"))
}

fn is_command_missing_error(error: &std::io::Error) -> bool {
    let error_text = error.to_string().to_lowercase();
    error.kind() == ErrorKind::NotFound
        || error.raw_os_error() == Some(2)
        || error_text.contains("no such file or directory")
}

fn missing_stdin_pipe() -> ProviderAdapterError {
    ProviderAdapterError::execution_failed(None, String::new(), "provider stdin pipe missing", 0)
}

fn missing_stdout_pipe() -> ProviderAdapterError {
    ProviderAdapterError::execution_failed(None, String::new(), "provider stdout pipe missing", 0)
}

fn missing_stderr_pipe() -> ProviderAdapterError {
    ProviderAdapterError::execution_failed(None, String::new(), "provider stderr pipe missing", 0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::protocol::provider_errors::ProviderErrorCode;

    #[tokio::test]
    async fn process_manager_reports_missing_command() {
        let result = ProcessManager::spawn(
            "__aria_missing_provider_command__",
            &[],
            &std::env::current_dir().unwrap(),
            &BTreeMap::new(),
            CancellationToken::new(),
        )
        .await;

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().code,
            ProviderErrorCode::ProviderCommandMissing
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_manager_resolves_relative_command_from_working_dir() {
        let working_dir = tempfile::tempdir().expect("working dir");
        let command_path = working_dir.path().join("provider-fixture");
        fs::write(&command_path, "#!/bin/sh\nexit 0\n").expect("write fixture");
        fs::set_permissions(&command_path, fs::Permissions::from_mode(0o755))
            .expect("chmod fixture");

        let mut process = ProcessManager::spawn(
            "./provider-fixture",
            &[],
            working_dir.path(),
            &BTreeMap::new(),
            CancellationToken::new(),
        )
        .await
        .expect("spawn relative provider fixture");

        let status = process.child.wait().await.expect("wait fixture");
        assert!(status.success());
    }
}

#[cfg(all(test, unix))]
mod drop_tests;
#[cfg(all(test, windows))]
mod windows_tests;
