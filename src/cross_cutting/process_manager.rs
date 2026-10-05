use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
#[cfg(test)]
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[cfg(windows)]
use command_group::{AsyncCommandGroup, AsyncGroupChild};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_boundary::{
    ProviderBoundaryError, ProviderBoundaryLauncher, ProviderBoundaryPlan,
};

const TRANSIENT_SPAWN_RETRY_COUNT: usize = 2;
const TRANSIENT_SPAWN_RETRY_DELAY: Duration = Duration::from_millis(10);

#[cfg(unix)]
fn unix_process_group_signal_target(spawn_time_leader_pid: Option<u32>, pgid: i32) -> Option<i32> {
    (spawn_time_leader_pid == u32::try_from(pgid).ok()).then_some(pgid)
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
    child: Option<tokio::process::Child>,
    #[cfg(windows)]
    child: AsyncGroupChild,
    #[cfg(unix)]
    pgid: Option<i32>,
    /// r19 根修:spawn 时冻结的进程组组长 pid(`process_group(0)` 使 child
    /// 即组长)。组长回收后组清理仍须可达——组内残留进程(CLI 工具孙进程)
    /// 持有 stdout/stderr 管道写端,会无限推迟 EOF(现场:sync 桥 epoll
    /// 空等 57min+)。以 spawn 时身份判定组归属,不随 child 回收失效。
    #[cfg(unix)]
    group_leader_pid: Option<u32>,
    #[cfg(all(unix, test))]
    drop_reaper_spawns: Arc<AtomicUsize>,
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
            let group_leader_pid = child.id();
            Ok(Self {
                child: Some(child),
                pgid,
                group_leader_pid,
                #[cfg(test)]
                drop_reaper_spawns: Arc::new(AtomicUsize::new(0)),
            })
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
            self.child
                .as_mut()
                .expect("managed process child is unavailable")
        }
        #[cfg(windows)]
        {
            self.child.inner()
        }
    }

    pub fn id(&self) -> Option<u32> {
        #[cfg(unix)]
        {
            self.child.as_ref().and_then(tokio::process::Child::id)
        }
        #[cfg(windows)]
        {
            self.child.id()
        }
    }

    pub fn start_kill(&mut self) -> std::io::Result<()> {
        // r19:以 spawn 时冻结的组长身份判定组归属——child 已退出/回收后
        // 组清理仍须可达(组内残留进程持有管道写端,推迟 EOF 使事件流
        // 无法终结)。
        #[cfg(unix)]
        if let Some(pgid) = self
            .pgid
            .and_then(|pgid| unix_process_group_signal_target(self.group_leader_pid, pgid))
        {
            let result = unsafe { libc::killpg(pgid, libc::SIGKILL) };
            if result == 0 {
                return Ok(());
            }
        }
        self.inner().start_kill()
    }

    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        // r19:成功回收后不再清空 pgid——组内残留进程(继承管道写端的工具
        // 孙进程)仍需后续 start_kill()/Drop 的组清理;组长身份以 spawn 时
        // 冻结的 group_leader_pid 判定,不受回收影响。
        self.inner().wait().await
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
        {
            if let Some(pgid) = self
                .pgid
                .take()
                .and_then(|pgid| unix_process_group_signal_target(self.group_leader_pid, pgid))
            {
                unsafe {
                    let _ = libc::killpg(pgid, libc::SIGKILL);
                }
            }
            if let Some(mut child) = self.child.take() {
                match child.try_wait() {
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => {
                        let _ = child.start_kill();
                        reap_child_after_drop(
                            child,
                            #[cfg(test)]
                            Arc::clone(&self.drop_reaper_spawns),
                        );
                    }
                }
            }
        }
    }
}

#[cfg(unix)]
fn reap_child_after_drop(
    mut child: tokio::process::Child,
    #[cfg(test)] reaper_spawns: Arc<AtomicUsize>,
) {
    #[cfg(test)]
    reaper_spawns.fetch_add(1, Ordering::Relaxed);
    let _ = std::thread::Builder::new()
        .name("aria-process-reaper".to_string())
        .spawn(move || {
            loop {
                match child.try_wait() {
                    Ok(Some(_)) | Err(_) => return,
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(5)),
                }
            }
        });
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
            .envs(env_vars);
        spawn_prepared(command, command_builder).await
    }

    /// Task 6a(冻结签名):产品写边界 spawn。validated launch 的 provider
    /// 进程必须经此入口携带不可伪造 `ProviderBoundaryPlan`;plan 非法或本机
    /// 无 bwrap/user namespace 时失败关闭,绝不回退无隔离 spawn。
    pub async fn spawn_with_boundary(
        command: &str,
        args: &[&str],
        working_dir: &Path,
        env_vars: &BTreeMap<String, String>,
        plan: &ProviderBoundaryPlan,
        _cancel: CancellationToken,
    ) -> Result<ManagedProcess, ProviderAdapterError> {
        Self::spawn_with_boundary_resolved(
            ProviderBoundaryLauncher::probe_environment(),
            command,
            args,
            working_dir,
            env_vars,
            plan,
            _cancel,
        )
        .await
    }

    /// 注入 launcher 的写边界 spawn(测试/装配 seam):行为与
    /// `spawn_with_boundary` 一致,仅 bwrap 解析可注入。
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_with_boundary_resolved(
        launcher: ProviderBoundaryLauncher,
        command: &str,
        args: &[&str],
        working_dir: &Path,
        env_vars: &BTreeMap<String, String>,
        plan: &ProviderBoundaryPlan,
        _cancel: CancellationToken,
    ) -> Result<ManagedProcess, ProviderAdapterError> {
        // 失败关闭:缺 bwrap/namespace 直接拒绝(判别码原样透传),调用方
        // 能力状态必须记 Unknown,不得回退 plain spawn。
        let bwrap = launcher.bwrap_path().ok_or_else(|| {
            ProviderAdapterError::provider_unavailable(
                ProviderBoundaryError::Unsupported(
                    "no usable bwrap/user namespace; refusing unisolated provider spawn".into(),
                )
                .to_string(),
            )
        })?;
        if !command_is_resolvable(command, working_dir, env_vars) {
            return Err(command_missing(command));
        }
        let argv = launcher
            .build_boundary_argv(command, args, working_dir, env_vars, plan)
            .map_err(|error| ProviderAdapterError::provider_unavailable(error.to_string()))?;
        let mut command_builder = Command::new(bwrap);
        command_builder
            .args(argv)
            .current_dir(working_dir)
            // 环境继承 + overlay:bwrap 不 --clearenv,保持既有配置发现。
            .envs(env_vars);
        spawn_prepared(command, command_builder).await
    }
}

/// 已构造好的 Command 统一走瞬态重试 + 管道提取(spawn 与写边界 spawn
/// 共用;`command` 仅用于错误归因)。
async fn spawn_prepared(
    command: &str,
    mut command_builder: Command,
) -> Result<ManagedProcess, ProviderAdapterError> {
    command_builder
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut retry_count = 0;
    let mut child = loop {
        match ManagedProcessChild::spawn(&mut command_builder) {
            Ok(child) => break child,
            Err(error)
                if is_retryable_spawn_error(&error)
                    && retry_count < TRANSIENT_SPAWN_RETRY_COUNT =>
            {
                retry_count += 1;
                tokio::time::sleep(TRANSIENT_SPAWN_RETRY_DELAY).await;
            }
            Err(error) => return Err(map_spawn_error(command, error)),
        }
    };

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

fn is_retryable_spawn_error(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(26)
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
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

    #[cfg(unix)]
    #[test]
    fn text_file_busy_is_retryable_but_not_a_missing_command() {
        let busy = std::io::Error::from_raw_os_error(26);
        assert!(is_retryable_spawn_error(&busy));
        assert!(!is_command_missing_error(&busy));
    }

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
