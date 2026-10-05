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

/// r19 续修(fix 轮 2,oracle 裁决):子进程退出 ⇒ 会话泵必然终结的排空/
/// join 有界窗口。正常完成的终态行已在管道缓冲内,秒级排空;窗口耗尽即
/// 以「子进程先于泵终态退出」fail-closed。测试环境缩短保持回归秒级。
#[cfg(not(test))]
pub(crate) const PROVIDER_CHILD_EXIT_STREAM_DRAIN: Duration = Duration::from_secs(2);
#[cfg(test)]
pub(crate) const PROVIDER_CHILD_EXIT_STREAM_DRAIN: Duration = Duration::from_secs(1);

/// `run_session_tail_with_exit_watch` 的结局:区分「泵自身终结」(其错误可能
/// 已由泵发终态事件——由各 provider 既有语义决定)与「子进程先退出且排空
/// 耗尽」(泵被截断,终态事件必然未发,调用方必须显式补发)。
#[derive(Debug)]
pub(crate) enum SessionTailExit<T, E> {
    /// 泵 future 自身返回(正常终态或泵内错误——终态事件发射责任归泵)。
    Stream(Result<T, E>),
    /// 子进程先退出且排空窗口耗尽:泵未终结、终态事件未发,调用方必须
    /// 以携带的错误显式补发终态 Failed。
    ChildExitBeforeTerminal { error: E },
}

/// r19 续修(fix 轮 2):会话泵与子进程退出的 exit-watch 竞速 supervisor
/// (claude 先例提炼,oracle 裁决三家同批收口)。
///
/// 契约(ProviderSession 终结保证,见 `StreamingProviderAdapter::start`):
/// 子进程退出 ⇒ 泵在有界时间内终结。泵 future 与 `child.wait()` 竞速;
/// 子进程先退出时先对进程组补 SIGKILL(释放持有 stdout/stderr 管道写端的
/// 组内残留进程,使 EOF 可达——EOF 本身可能被无限推迟,r19 现场),再给
/// 泵一个有界排空窗口;窗口耗尽即以合成错误终结,不得依赖消费方 timeout
/// 为唯一出口。
pub(crate) async fn run_session_tail_with_exit_watch<F, T, E>(
    stream: F,
    child: &mut ManagedProcessChild,
    drain_window: Duration,
    into_exit_error: impl FnOnce(String) -> E,
) -> SessionTailExit<T, E>
where
    F: Future<Output = Result<T, E>>,
{
    let mut stream = Box::pin(stream);
    tokio::select! {
        result = &mut stream => SessionTailExit::Stream(result),
        status = child.wait() => {
            let _ = child.start_kill();
            match tokio::time::timeout(drain_window, stream).await {
                Ok(result) => SessionTailExit::Stream(result),
                Err(_elapsed) => {
                    let status_note = match &status {
                        Ok(status) => format!("exit status: {status}"),
                        Err(error) => format!("wait error: {error}"),
                    };
                    SessionTailExit::ChildExitBeforeTerminal {
                        error: into_exit_error(format!(
                            "provider child exited before stream terminal event ({status_note})"
                        )),
                    }
                }
            }
        }
    }
}

/// r19 续修:终态 join 有界化(红线 a:终结条件已成立后的一切 join 有界)。
/// 先组清理(SIGKILL 后 wait 通常即刻返回),窗口耗尽放弃 join 返回 None。
pub(crate) async fn bounded_child_wait(
    child: &mut ManagedProcessChild,
    window: Duration,
) -> Option<ExitStatus> {
    let _ = child.start_kill();
    match tokio::time::timeout(window, child.wait()).await {
        Ok(Ok(status)) => Some(status),
        Ok(Err(_wait_error)) => None,
        Err(_elapsed) => None,
    }
}

/// r19 续修:后台任务 join 有界化——窗口耗尽 abort 后回收,不泄漏句柄。
pub(crate) async fn bounded_task_join<T>(
    mut task: tokio::task::JoinHandle<T>,
    window: Duration,
) -> Option<T> {
    match tokio::time::timeout(window, &mut task).await {
        Ok(Ok(output)) => Some(output),
        Ok(Err(_join_error)) => None,
        Err(_elapsed) => {
            task.abort();
            task.await.ok()
        }
    }
}

/// r19 续修:exit-watch 合成错误构造——`execution_failed` 的固定 details
/// 文案被覆写为携带子进程死因(调用方 Failed 事件可直接引用)。
pub(crate) fn provider_child_exit_error(message: String) -> ProviderAdapterError {
    let mut error = ProviderAdapterError::execution_failed(None, String::new(), String::new(), 0);
    error.details = message;
    error
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
