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

const TRANSIENT_SPAWN_RETRY_COUNT: usize = 2;
const TRANSIENT_SPAWN_RETRY_DELAY: Duration = Duration::from_millis(10);

/// 批2 Task 5:读取 `/proc/<pid>/stat` 的 starttime(第 22 字段,时钟 ticks)——
/// 与 PID 组成 PID-start 真实身份,防 PID 复用误杀。进程消失/不可读返回 None。
#[cfg(unix)]
pub(crate) fn process_start_time_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, tail) = stat.rsplit_once(") ")?;
    // tail 从第 3 字段(state)开始:starttime=第 22 字段 → nth(19)。
    tail.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(not(unix))]
pub(crate) fn process_start_time_ticks(_pid: u32) -> Option<u64> {
    None
}

/// PID-start 真实身份：pid + `/proc/<pid>/stat` starttime（时钟 ticks）。
///
/// starttime 随进程回收消失、随 pid 复用必然改变——它是「同一 pid 位置
/// 是否仍是原主」的唯一可靠判据。信号发射前一律以此核对（PID 复用误杀
/// 防护）：只看 pid 会把「复用后占位的新进程」当成自己人打掉。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProcessStartIdentity {
    pid: u32,
    start_time_ticks: u64,
}

impl ProcessStartIdentity {
    /// spawn 后立即冻结。`/proc` 不可读（进程已消失/无 procfs）→ None；
    /// 调用方按「身份不可证」处理，绝不凭 pid 裸发信号。
    pub(crate) fn freeze(pid: u32) -> Option<Self> {
        Some(Self {
            pid,
            start_time_ticks: process_start_time_ticks(pid)?,
        })
    }

    /// 测试专用:构造「同一 pid 位置换了主人」的冻结身份(PID 复用场景)。
    #[cfg(test)]
    pub(crate) fn with_start_time_ticks(pid: u32, start_time_ticks: u64) -> Self {
        Self {
            pid,
            start_time_ticks,
        }
    }

    #[cfg(test)]
    pub(crate) fn start_time_ticks(self) -> u64 {
        self.start_time_ticks
    }

    /// 该 pid 位置仍是冻结时的同一进程。僵尸仍占用 pid（回收前不会被
    /// 复用），向它的进程组发信号是安全的，因此只比 starttime、不看 state。
    pub(crate) fn still_same_process(self) -> bool {
        process_start_time_ticks(self.pid) == Some(self.start_time_ticks)
    }
}

/// 身份核对后的进程组信号——组信号发射的唯一出口（PID 复用误杀防护）。
///
/// 仅当冻结身份仍是同一进程、且 `pgid` 就是该身份的 pid（`process_group(0)`
/// / `setpgid(0,0)` 使组长 pid 即 pgid）时才 `killpg`。已回收、已复用、
/// 身份不可证 → 不发信号并返回 false：拒绝发射是可审计结果，不是静默。
/// 组内残留清理的代价必须让位于「绝不误杀复用后的无关进程组」。
pub(crate) fn signal_group_if_live_identity(
    identity: Option<ProcessStartIdentity>,
    pgid: i32,
    signal: i32,
) -> bool {
    let Ok(pgid_pid) = u32::try_from(pgid) else {
        return false;
    };
    let Some(identity) = identity.filter(|identity| identity.pid == pgid_pid) else {
        tracing::debug!(
            target: "process_manager",
            pgid,
            "process group signal skipped: no frozen leader identity"
        );
        return false;
    };
    if !identity.still_same_process() {
        tracing::warn!(
            target: "process_manager",
            pgid,
            start_time_ticks = identity.start_time_ticks,
            "process group signal skipped: frozen leader identity is reaped or pid-reused"
        );
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: killpg 只作用于已核对身份的正整数 pgid（组长 pid）。
        unsafe { libc::killpg(pgid, signal) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = signal;
        false
    }
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
    /// 即组长)。spawn 时身份判定组归属,不随 child 回收失效。
    #[cfg(unix)]
    group_leader_pid: Option<u32>,
    /// PID 复用防护:spawn 时冻结的组长 (pid, `/proc` starttime) 真实身份。
    /// 每次发射组信号前必须核对——组长已回收/pid 已被复用 → 绝不 killpg
    /// (现场:并行测试中误杀无关进程组的 SIGKILL)。
    #[cfg(unix)]
    leader_identity: Option<ProcessStartIdentity>,
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
            let leader_identity = group_leader_pid.and_then(ProcessStartIdentity::freeze);
            Ok(Self {
                child: Some(child),
                pgid,
                group_leader_pid,
                leader_identity,
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

    /// 批2 Task 5:spawn 时冻结的进程组组长 PID——资源归属真实身份的
    /// PID 维(与 `/proc/<pid>/stat` starttime 组成 PID-start identity,
    /// 防 PID 复用误杀;`process_group(0)` 使 child 即组长)。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn spawn_leader_pid(&self) -> Option<u32> {
        #[cfg(unix)]
        {
            self.group_leader_pid
        }
        #[cfg(windows)]
        {
            self.id()
        }
    }

    pub fn start_kill(&mut self) -> std::io::Result<()> {
        // PID 复用防护:组信号只在 spawn 时冻结的组长身份仍匹配时发射。
        // 组长已回收/pid 已被复用 → 拒绝 killpg(否则会打掉复用后的无关
        // 进程组),仅回收直接子进程。
        #[cfg(unix)]
        if let Some(pgid) = self.pgid
            && signal_group_if_live_identity(self.leader_identity, pgid, libc::SIGKILL)
        {
            return Ok(());
        }
        self.inner().start_kill()
    }

    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        // r19:成功回收后不清空 pgid——组内残留进程(继承管道写端的工具
        // 孙进程)仍需后续 start_kill()/Drop 的组信号;是否发射由冻结的
        // leader_identity 核对决定,不因 child 句柄状态而放行。
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
            if let Some(pgid) = self.pgid.take() {
                // 与 start_kill 同一口径:身份不符即拒绝发射(可审计 warn)。
                let _ = signal_group_if_live_identity(self.leader_identity, pgid, libc::SIGKILL);
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
    /// A1-a(Bug45 批2 环境回退底座):spawn 宿主继承——子进程继承宿主
    /// 环境,`env_vars` 只是调用方显式覆盖层(envs 覆盖同名键),空 map
    /// 即纯继承;裸名命令在 env_vars 无 PATH 时回退宿主 PATH 解析
    /// (`path_env`),流式/同步/终端三路同一姿态。
    pub async fn spawn(
        command: &str,
        args: &[&str],
        working_dir: &Path,
        env_vars: &BTreeMap<String, String>,
        _cancel: CancellationToken,
    ) -> Result<ManagedProcess, ProviderAdapterError> {
        Self::spawn_with_environment(command, args, working_dir, env_vars).await
    }

    /// 兼容别名:与 spawn 同一宿主继承语义(既有调用方零迁移;A1-a 起
    /// 两入口不再有 env_clear 差异)。
    pub async fn spawn_isolated(
        command: &str,
        args: &[&str],
        working_dir: &Path,
        env_vars: &BTreeMap<String, String>,
        _cancel: CancellationToken,
    ) -> Result<ManagedProcess, ProviderAdapterError> {
        Self::spawn_with_environment(command, args, working_dir, env_vars).await
    }

    async fn spawn_with_environment(
        command: &str,
        args: &[&str],
        working_dir: &Path,
        env_vars: &BTreeMap<String, String>,
    ) -> Result<ManagedProcess, ProviderAdapterError> {
        if !command_is_resolvable(command, working_dir, env_vars) {
            return Err(command_missing(command));
        }

        let mut command_builder = Command::new(command);
        // A1-a(Bug45 批2 环境回退底座):env_clear 已删——子进程继承宿主
        // 环境,env_vars 仅作显式覆盖层(envs 覆盖同名键)。
        command_builder
            .args(args)
            .current_dir(working_dir)
            .envs(env_vars);
        // 计划 2026-11-13 包 2.4:子进程 PATH 显式下发 overlay 合并值——
        // env_vars 含 PATH 时覆盖为合并值,不含时也显式写,使 overlay 目录
        // 到达子进程;overlay 为空时与宿主 PATH 等值,继承语义不变。
        if let Some(merged_path) = path_env(env_vars) {
            command_builder.env("PATH", merged_path);
        }
        // 计划 2026-11-13 §五 2.12:补入登录 shell 探测回的 agent 专属
        // 环境变量(缺失才补;进程 env/覆盖层已有的值优先)。
        for (name, value) in crate::cross_cutting::path_overlay::shell_env_overrides(Some(env_vars))
        {
            command_builder.env(name, value);
        }
        spawn_prepared(command, command_builder).await
    }

    // 批2 Task 10(旧物理 launcher 退役):provider 进程统一经
    // `spawn`/`spawn_isolated` 受信 Host 启动(A1-a 起宿主继承 + 显式
    // 覆盖层);`spawn_with_boundary[_resolved]`(bwrap 写边界 spawn)已删除,
    // 不再存在「无 bwrap 不可用」的启动前置——物理隔离档不保留可选回退路径。
}

/// 已构造好的 Command 统一走瞬态重试 + 管道提取(spawn 共用;
/// `command` 仅用于错误归因)。
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
    // A1-a(Bug45 批2 环境回退底座):env_vars 无 PATH 时回退宿主 PATH
    // (std::env)——裸名 CLI 解析不断链;显式 PATH 仍优先(覆盖层)。
    // 计划 2026-11-13 包 2.4:可执行性判定与下发的 PATH 都改为 overlay
    // 合并后的值(effective_path 在 overlay 为空时原样返回 base,零行为变化)。
    let base = env_vars.get("PATH").map(String::as_str);
    if base.is_none() && std::env::var_os("PATH").is_none() {
        return None;
    }
    Some(OsString::from(
        crate::cross_cutting::path_overlay::effective_path(base),
    ))
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

    /// A1-a(Bug45 批2 环境回退底座):spawn 传空 env map 时子进程继承
    /// 宿主环境——隔离父进程不构造 marker env,真实子进程读到宿主
    /// PATH/HOME(只读宿主键,不 set_var 污染并行测试)。
    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_inherits_host_environment_when_env_map_is_empty() {
        use tokio::io::AsyncReadExt;
        // overlay 是进程级共享状态：并行用例登记目录会让「子进程 PATH ==
        // 宿主 PATH」的逐字断言失真 ⇒ 持锁清空后断言（计划 §6.1）。
        let _overlay_state = crate::cross_cutting::path_overlay::OverlayStateGuard::acquire();

        let host_home = std::env::var("HOME").expect("host HOME");
        let host_path = std::env::var("PATH").expect("host PATH");

        let mut process = ProcessManager::spawn(
            "/bin/sh",
            &["-c", "printf '%s\\n%s\\n' \"$HOME\" \"$PATH\""],
            &std::env::current_dir().expect("cwd"),
            &BTreeMap::new(),
            CancellationToken::new(),
        )
        .await
        .expect("spawn host env printer");

        let mut stdout = String::new();
        let mut reader = std::pin::pin!(process.stdout);
        reader.read_to_string(&mut stdout).await.expect("read env");
        let _ = process.child.wait().await.expect("wait fixture");
        let mut lines = stdout.lines();
        assert_eq!(
            lines.next(),
            Some(host_home.as_str()),
            "子进程必须读到宿主 HOME(不在输入 env map 中)",
        );
        assert_eq!(
            lines.next(),
            Some(host_path.as_str()),
            "子进程必须读到宿主 PATH(不在输入 env map 中)",
        );
    }

    /// A1-a(Bug45 批2 环境回退底座):env_vars 无 PATH 时回退宿主
    /// PATH——裸名命令经宿主 PATH 解析成功,不再显式缺工具。
    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_without_explicit_path_resolves_via_host_path() {
        let mut process = ProcessManager::spawn(
            "sh",
            &["-c", "exit 0"],
            &std::env::current_dir().expect("cwd"),
            &BTreeMap::new(),
            CancellationToken::new(),
        )
        .await
        .expect("裸名命令必须经宿主 PATH 解析成功");

        let status = process.child.wait().await.expect("wait fixture");
        assert!(status.success());
    }

    /// A1-a(Bug45 批2 环境回退底座):显式 env map 是覆盖层而非快照——
    /// 自定义 token 经 envs 覆盖注入生效,map 外的宿主键(HOME)仍从
    /// 宿主继承到达子进程;断言子进程拿到宿主值,不是输入 map 快照。
    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_env_map_overrides_host_without_replacing_it() {
        use tokio::io::AsyncReadExt;

        let host_home = std::env::var("HOME").expect("host HOME");
        let mut process = ProcessManager::spawn(
            "/bin/sh",
            &["-c", "printf '%s|%s' \"$ARIA_A1A_TOKEN\" \"$HOME\""],
            &std::env::current_dir().expect("cwd"),
            &BTreeMap::from([("ARIA_A1A_TOKEN".to_string(), "override-ok".to_string())]),
            CancellationToken::new(),
        )
        .await
        .expect("spawn token+home printer");

        let mut stdout = String::new();
        let mut reader = std::pin::pin!(process.stdout);
        reader.read_to_string(&mut stdout).await.expect("read env");
        let _ = process.child.wait().await.expect("wait fixture");
        assert_eq!(
            stdout,
            format!("override-ok|{host_home}"),
            "token 来自输入 env map,HOME 必须来自宿主(非输入 map 快照)",
        );
    }

    /// 批2 Task 5:spawn 即冻结 PID-start 真实身份(PID 与 /proc starttime)。
    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_freezes_process_start_identity() {
        let working_dir = tempfile::tempdir().expect("working dir");
        let sleeper = working_dir.path().join("identity-sleeper");
        std::fs::write(&sleeper, "#!/bin/sh\nsleep 30\n").expect("write fixture");
        std::fs::set_permissions(&sleeper, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fixture");

        let mut process = ProcessManager::spawn(
            &sleeper.to_string_lossy(),
            &[],
            working_dir.path(),
            &BTreeMap::new(),
            CancellationToken::new(),
        )
        .await
        .expect("spawn identity fixture");

        let pid = process.child.spawn_leader_pid().expect("spawn leader pid");
        assert_eq!(Some(pid), process.child.id());
        let start_ticks =
            process_start_time_ticks(pid).expect("/proc starttime readable for live child");
        assert!(start_ticks > 0);

        process.child.terminate().await;
        assert_eq!(process_start_time_ticks(pid), None);
    }

    /// 计划 2026-11-13 包 2.4：PATH overlay 合并点——spawn 的可执行性判定
    /// 与子进程 env 的 PATH 都必须用合并后的 PATH：overlay 内的裸名命令可
    /// 解析，且子进程 PATH 首目录即 overlay 目录（Prepend 语义）。
    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_merges_path_overlay_into_resolvability_and_child_env() {
        use tokio::io::AsyncReadExt;

        let _overlay_state = crate::cross_cutting::path_overlay::OverlayStateGuard::acquire();

        let dir = tempfile::tempdir().expect("overlay dir");
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create overlay bin dir");
        let probe = bin_dir.join("aria-overlay-probe");
        fs::write(&probe, "#!/bin/sh\nprintf '%s' \"$PATH\"\n").expect("write probe fixture");
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o755))
            .expect("chmod probe fixture");
        crate::cross_cutting::path_overlay::register(
            &bin_dir,
            crate::cross_cutting::path_overlay::OverlayPosition::Prepend,
        );

        let mut process = ProcessManager::spawn(
            "aria-overlay-probe",
            &[],
            dir.path(),
            &BTreeMap::new(),
            CancellationToken::new(),
        )
        .await
        .expect("overlay 内的裸名命令必须经合并后的 PATH 解析成功");

        let mut stdout = String::new();
        let mut reader = std::pin::pin!(process.stdout);
        reader.read_to_string(&mut stdout).await.expect("read PATH");
        let _ = process.child.wait().await.expect("wait probe fixture");
        assert_eq!(
            std::env::split_paths(&stdout).next(),
            Some(bin_dir),
            "子进程 PATH 首目录必须是 overlay 目录(Prepend): {stdout}"
        );
    }

    /// 计划 2026-11-13 §五 2.12：spawn 构造子进程 env 时补入登录 shell 探测
    /// 回的 agent 专属环境变量——缺失才补（进程 env / 显式覆盖层已有的值
    /// 优先，不被探测值覆盖）。
    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_injects_shell_probe_agent_env_when_platform_lacks_it() {
        use tokio::io::AsyncReadExt;

        let _overlay_state = crate::cross_cutting::path_overlay::OverlayStateGuard::acquire();

        let Some(free_var) = ["PI_CODING_AGENT_DIR", "CODEX_HOME", "CLAUDE_CONFIG_DIR"]
            .into_iter()
            .find(|name| std::env::var_os(name).is_none())
        else {
            panic!("宿主 env 已含全部 agent 变量,无法验证补入分支(环境不满足)");
        };
        crate::cross_cutting::path_overlay::set_shell_probe_for_test(
            None,
            vec![(free_var.to_string(), "/probe/agent-dir".to_string())],
        );

        // 补入分支:env_vars 为空、宿主缺该变量 ⇒ 子进程读到探测值。
        let script = format!("printf '%s' \"${free_var}\"");
        let mut process = ProcessManager::spawn(
            "/bin/sh",
            &["-c", &script],
            &std::env::current_dir().expect("cwd"),
            &BTreeMap::new(),
            CancellationToken::new(),
        )
        .await
        .expect("spawn env printer");
        let mut stdout = String::new();
        let mut reader = std::pin::pin!(process.stdout);
        reader.read_to_string(&mut stdout).await.expect("read env");
        let _ = process.child.wait().await;
        assert_eq!(
            stdout, "/probe/agent-dir",
            "缺失的 agent 变量必须被探测值补入"
        );

        // 平台优先分支:显式覆盖层已有该变量 ⇒ 保留显式值。
        let mut process = ProcessManager::spawn(
            "/bin/sh",
            &["-c", &script],
            &std::env::current_dir().expect("cwd"),
            &BTreeMap::from([(free_var.to_string(), "/explicit/agent-dir".to_string())]),
            CancellationToken::new(),
        )
        .await
        .expect("spawn explicit env printer");
        let mut stdout = String::new();
        let mut reader = std::pin::pin!(process.stdout);
        reader.read_to_string(&mut stdout).await.expect("read env");
        let _ = process.child.wait().await;
        assert_eq!(
            stdout, "/explicit/agent-dir",
            "覆盖层已有值不得被探测值覆盖"
        );
    }
}

#[cfg(all(test, unix))]
mod drop_tests;
#[cfg(all(test, windows))]
mod windows_tests;
