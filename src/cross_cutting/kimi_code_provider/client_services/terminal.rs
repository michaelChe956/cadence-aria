//! Terminal execution manager for the kimi `terminal/*` client services.
//!
//! Every terminal runs on the host as `pipe` + process group + combined
//! stdout/stderr output cap, with a concurrency limit of `MAX_TERMINALS`;
//! 批2 Task 7（Bug45）：命令时长无独立上限，唯一预算是受信 invocation
//! 总 deadline（经会话取消链 kill 收口）。Lifecycle is `created ->
//! running -> (exited|killed) -> released`; `kill`/`release` are idempotent
//! and unknown ids are distinguishable errors. 进程/目录先登记(`create`
//! 先于 `start`,spawn 前占住并发名额),spawn 失败立即释放名额。
//!
//! 批2 Task 6:全角色 Host 单路径——不再有 bwrap/netns/no-new-privs 执行
//! 隔离,也没有固定解释器/只读 grammar。子进程环境是冻结会话环境(与
//! provider 进程 spawn 消费的同一 env map):`env_clear` 后只注入该 map,
//! 裸名 program 经该环境的 `PATH` 解析(fail-closed,不回退宿主 PATH)。

use std::collections::{BTreeMap, HashMap};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::sandbox::open_dir_no_follow_inherit;

pub const MAX_TERMINALS: usize = 4;
pub const MAX_TERMINAL_OUTPUT_BYTES: usize = 1_048_576;

#[derive(Debug, Clone)]
pub struct TerminalCommand {
    /// Program exactly as the ACP server sent it: a bare name resolved
    /// through the frozen environment's `PATH`, or an absolute path. 原样
    /// argv——宿主不再施加固定解释器白名单,也不改写参数。
    pub program: String,
    /// Arguments passed verbatim (argv form) or the shell wrapper
    /// (`["-c", script]` for the legacy single-string form).
    pub args: Vec<String>,
    /// Verified working directory (canonical, anchored at the face root).
    pub cwd: PathBuf,
    /// 冻结会话环境(与 provider 进程 spawn 消费的同一 env map)。子进程
    /// `env_clear` 后只注入该 map;ACP 参数不参与构造(不能根据 ACP 参数
    /// 造 profile)。
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalResult {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub killed: bool,
    pub truncated: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalError {
    TooManyTerminals,
    UnknownTerminal(String),
}

impl std::fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TerminalError::TooManyTerminals => write!(
                formatter,
                "terminal concurrency limit of {MAX_TERMINALS} reached"
            ),
            TerminalError::UnknownTerminal(id) => write!(formatter, "unknown terminal: {id}"),
        }
    }
}

impl std::error::Error for TerminalError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalState {
    Created,
    Running,
    Finished,
    Released,
}

struct TerminalEntry {
    state: Mutex<TerminalState>,
    kill: CancellationToken,
    done: Arc<Notify>,
    result: Mutex<Option<TerminalResult>>,
    command: Mutex<Option<TerminalCommand>>,
    /// Combined stdout+stderr retained for the server's `terminal/output`
    /// request (kimi 0.38.0 asks for output as a request/response, after
    /// `terminal/wait_for_exit`), capped by `MAX_TERMINAL_OUTPUT_BYTES`.
    output: Arc<Mutex<String>>,
}

impl TerminalEntry {
    fn new(command: TerminalCommand, kill: CancellationToken) -> Self {
        Self {
            state: Mutex::new(TerminalState::Created),
            kill,
            done: Arc::new(Notify::new()),
            result: Mutex::new(None),
            command: Mutex::new(Some(command)),
            output: Arc::new(Mutex::new(String::new())),
        }
    }
}

#[derive(Default)]
struct ManagerInner {
    terminals: HashMap<String, Arc<TerminalEntry>>,
    next_id: u64,
}

/// The shared terminal manager. Cheap to clone; every clone shares state.
///
/// 批2 Task 7（Bug45）：不再有 `TERMINAL_COMMAND_TIMEOUT_SECS=120` 独立
/// 命令时长上限——命令的唯一预算是受信 invocation 总 deadline（经会话
/// 取消链 kill 收口），双层看门狗在总 deadline 内对真实在途 terminal
/// 暂停「无工作 stall」判定（见 `has_active_terminals`）。
///
/// 修复轮(F2)：`cleanup` 是 manager 级收编 token——每个 entry 的 kill
/// token 都是它的 child，`cleanup_all` 取消它即级联收割**当前与此后**
/// 登记的一切 terminal。这关闭「create/start 与 dispatcher Drop→
/// cleanup_all 并发」的逃逸窗口：晚于 cleanup_all 插入的登记/spawn 同样
/// 有界收口（120s 已删，孤儿不允许无时间上限）。
pub struct TerminalManager {
    inner: Arc<Mutex<ManagerInner>>,
    cleanup: CancellationToken,
}

impl Clone for TerminalManager {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            cleanup: self.cleanup.clone(),
        }
    }
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManagerInner::default())),
            cleanup: CancellationToken::new(),
        }
    }
    /// Register a new terminal. Returns the terminal id once the concurrency
    /// limit permits it. The process is not started until [`Self::start`] so
    /// the caller can send the `terminal/create` response first and no output
    /// notification can precede it.
    pub async fn create(&self, command: TerminalCommand) -> Result<String, TerminalError> {
        let mut inner = self.inner.lock().expect("terminal manager lock");
        let active = inner
            .terminals
            .values()
            .filter(|entry| {
                *entry.state.lock().expect("terminal state lock") != TerminalState::Released
            })
            .count();
        if active >= MAX_TERMINALS {
            return Err(TerminalError::TooManyTerminals);
        }
        inner.next_id += 1;
        let id = format!("term-{}", inner.next_id);
        inner.terminals.insert(
            id.clone(),
            Arc::new(TerminalEntry::new(command, self.cleanup.child_token())),
        );
        Ok(id)
    }

    /// Start the process for a previously created terminal.
    pub fn start(&self, id: &str) -> Result<(), TerminalError> {
        let entry = {
            let inner = self.inner.lock().expect("terminal manager lock");
            inner
                .terminals
                .get(id)
                .cloned()
                .ok_or_else(|| TerminalError::UnknownTerminal(id.to_string()))?
        };
        let command = {
            let mut command = entry.command.lock().expect("terminal command lock").take();
            if command.is_none() {
                // start cannot proceed (e.g. double start): release the slot
                // so the concurrency quota is not leaked.
                *entry.state.lock().expect("terminal state lock") = TerminalState::Released;
                return Err(TerminalError::UnknownTerminal(id.to_string()));
            }
            command.take().expect("command checked above")
        };
        tokio::spawn(run_terminal(entry, command));
        Ok(())
    }

    /// Wait for a terminal to exit (or be killed).
    ///
    /// `tokio::sync::Notify` stores no permit, so a terminal that finished
    /// before this call would never wake a late `notified()` registration.
    /// Register the future first, then check the result slot; repeat until
    /// the result is present.
    pub async fn wait_for_exit(&self, id: &str) -> Result<TerminalResult, TerminalError> {
        let entry = {
            let inner = self.inner.lock().expect("terminal manager lock");
            inner
                .terminals
                .get(id)
                .cloned()
                .ok_or_else(|| TerminalError::UnknownTerminal(id.to_string()))?
        };
        loop {
            let notified = entry.done.notified();
            if let Some(result) = entry.result.lock().expect("terminal result lock").clone() {
                return Ok(result);
            }
            notified.await;
        }
    }

    /// Combined retained output for a terminal (stdout + stderr, in read
    /// order). Unknown ids are errors; released terminals keep their output
    /// so a late `terminal/output` request after `terminal/release` still
    /// succeeds.
    pub fn output(&self, id: &str) -> Result<String, TerminalError> {
        let inner = self.inner.lock().expect("terminal manager lock");
        inner
            .terminals
            .get(id)
            .map(|entry| entry.output.lock().expect("terminal output lock").clone())
            .ok_or_else(|| TerminalError::UnknownTerminal(id.to_string()))
    }

    /// Kill a terminal's process group. Idempotent; unknown ids are errors.
    pub async fn kill(&self, id: &str) -> Result<(), TerminalError> {
        let entry = {
            let inner = self.inner.lock().expect("terminal manager lock");
            inner
                .terminals
                .get(id)
                .cloned()
                .ok_or_else(|| TerminalError::UnknownTerminal(id.to_string()))?
        };
        // Idempotent per spec: killing an already-released (or unknown-state)
        // terminal succeeds; only unknown ids are errors.
        entry.kill.cancel();
        Ok(())
    }

    /// Release a terminal. Idempotent; kills the process group if still
    /// running and marks the terminal released so later kill/release succeed.
    pub async fn release(&self, id: &str) -> Result<(), TerminalError> {
        let entry = {
            let inner = self.inner.lock().expect("terminal manager lock");
            inner
                .terminals
                .get(id)
                .cloned()
                .ok_or_else(|| TerminalError::UnknownTerminal(id.to_string()))?
        };
        {
            let mut state = entry.state.lock().expect("terminal state lock");
            if *state == TerminalState::Released {
                return Ok(());
            }
            *state = TerminalState::Released;
        }
        entry.kill.cancel();
        Ok(())
    }

    /// Kill every live terminal (session cancel / teardown)。取消 manager 级
    /// 收编 token：级联收割当前已登记与并发窗口内后续登记的 terminal
    /// （见结构体注释修复轮 F2）。
    pub fn cleanup_all(&self) {
        self.cleanup.cancel();
    }

    /// 批2 Task 7（Bug45）：真实活动 registry 读口——在途 = Created（已
    /// 登记，spawn 前）/ Running（spawn 后未收口）；Exited/Killed/
    /// Released 及 spawn 失败全部移除（与并发名额同一状态源）。ACP
    /// session 的 stall 判定与产品双层看门狗（经
    /// `streaming_provider::TerminalActivitySnapshot`）消费同一真实
    /// 进程状态——不是伪造心跳或模型自述。
    pub fn has_active_terminals(&self) -> bool {
        let inner = self.inner.lock().expect("terminal manager lock");
        inner.terminals.values().any(|entry| {
            matches!(
                *entry.state.lock().expect("terminal state lock"),
                TerminalState::Created | TerminalState::Running
            )
        })
    }
}

/// 批2 Task 7（Bug45）：terminal manager 即活动快照本体——产品双层
/// 看门狗经事件通道拿到的是同一 manager 的克隆句柄（共享 registry），
/// 读取的是真实进程生命周期，不引入第二份状态。
impl crate::cross_cutting::streaming_provider::TerminalActivitySnapshot for TerminalManager {
    fn has_active_terminals(&self) -> bool {
        TerminalManager::has_active_terminals(self)
    }
}

struct TerminalChild {
    child: Child,
    #[cfg(unix)]
    pgid: Option<i32>,
}

impl TerminalChild {
    fn start_kill(&mut self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid
            && pgid > 0
        {
            unsafe {
                let _ = libc::killpg(pgid, libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }

    async fn terminate(&mut self) {
        self.start_kill();
        let _ = self.child.wait().await;
    }

    async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }
}

impl Drop for TerminalChild {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid
            && pgid > 0
        {
            unsafe {
                let _ = libc::killpg(pgid, libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }
}

// 批2 Task 6:固定 HOME/GIT_CONFIG_GLOBAL 等手搓子进程环境已删——终端
// 子进程消费冻结会话环境(TerminalCommand::env,与 provider 进程同一
// env map),不再按授权根另行构造。

async fn run_terminal(entry: Arc<TerminalEntry>, command: TerminalCommand) {
    let started = Instant::now();
    {
        let mut state = entry.state.lock().expect("terminal state lock");
        if *state == TerminalState::Released {
            let mut result = entry.result.lock().expect("terminal result lock");
            *result = Some(TerminalResult {
                exit_code: None,
                timed_out: false,
                killed: true,
                truncated: false,
                duration_ms: 0,
            });
            entry.done.notify_waiters();
            return;
        }
        *state = TerminalState::Running;
    }

    let anchor: Option<OwnedFd> = open_dir_no_follow_inherit(&command.cwd, Path::new("")).ok();
    let mut builder = build_terminal_command(&command, anchor.as_ref());
    // ETXTBSY（目标正被写打开，含并发「写脚本→exec」压力与异步句柄释放
    // 延迟）是瞬态错误：有界退避重试，而不是把 spawn 失败伪装成
    // 「exit_code=None 的正常完成」——那会让调用方无法区分真实完成。
    const SPAWN_ETXTBSY_RETRY_BUDGET: Duration = Duration::from_secs(2);
    let mut spawn_backoff = Duration::from_millis(5);
    let mut child = loop {
        match builder.spawn() {
            Ok(child) => {
                let pgid = child.id().and_then(|pid| i32::try_from(pid).ok());
                break TerminalChild { child, pgid };
            }
            Err(error)
                if error.raw_os_error() == Some(libc::ETXTBSY)
                    && started.elapsed() + spawn_backoff < SPAWN_ETXTBSY_RETRY_BUDGET =>
            {
                tracing::debug!(
                    target: "kimi_code_provider",
                    attempt_after_ms = started.elapsed().as_millis(),
                    "terminal spawn hit transient ETXTBSY; backing off and retrying"
                );
                tokio::time::sleep(spawn_backoff).await;
                spawn_backoff = (spawn_backoff * 2).min(Duration::from_millis(100));
            }
            Err(error) => {
                tracing::debug!(target: "kimi_code_provider", %error, "terminal spawn failed");
                let mut result = entry.result.lock().expect("terminal result lock");
                *result = Some(TerminalResult {
                    exit_code: None,
                    timed_out: false,
                    killed: false,
                    truncated: false,
                    duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                });
                entry.done.notify_waiters();
                let mut state = entry.state.lock().expect("terminal state lock");
                // 批2 Task 6:spawn 失败释放并发名额——失败终端不得占住
                // 1/4 配额(登记已先行,失败即回收;结果/output 仍可查询)。
                *state = TerminalState::Released;
                return;
            }
        }
    };

    let stdout = child.child.stdout.take();
    let stderr = child.child.stderr.take();

    let budget = Arc::new(AtomicUsize::new(0));
    let truncated = Arc::new(AtomicBool::new(false));
    let stdout_task = stdout.map(|stream| {
        tokio::spawn(read_terminal_stream(
            stream,
            Arc::clone(&budget),
            Arc::clone(&truncated),
            Arc::clone(&entry.output),
        ))
    });
    let stderr_task = stderr.map(|stream| {
        tokio::spawn(read_terminal_stream(
            stream,
            Arc::clone(&budget),
            Arc::clone(&truncated),
            Arc::clone(&entry.output),
        ))
    });

    // 批2 Task 7（Bug45）：Completion 只剩 Exited/Killed——120s 独立超时
    // 臂已删，命令时长由受信 invocation 总 deadline 经 kill/cancel 收口。
    enum Completion {
        Exited(std::io::Result<std::process::ExitStatus>),
        Killed,
    }

    let completion = tokio::select! {
        status = child.wait() => Completion::Exited(status),
        _ = entry.kill.cancelled() => Completion::Killed,
    };

    let (exit_code, timed_out, killed) = match completion {
        Completion::Exited(status) => (status.ok().and_then(|status| status.code()), false, false),
        Completion::Killed => {
            child.terminate().await;
            (None, false, true)
        }
    };

    if let Some(task) = stdout_task {
        let _ = task.await;
    }
    if let Some(task) = stderr_task {
        let _ = task.await;
    }

    {
        let mut result = entry.result.lock().expect("terminal result lock");
        *result = Some(TerminalResult {
            exit_code,
            timed_out,
            killed,
            truncated: truncated.load(Ordering::Relaxed),
            duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        });
        entry.done.notify_waiters();
    }
    {
        let mut state = entry.state.lock().expect("terminal state lock");
        if *state != TerminalState::Released {
            *state = TerminalState::Finished;
        }
    }
}

fn build_terminal_command(command: &TerminalCommand, anchor: Option<&OwnedFd>) -> Command {
    let mut builder = Command::new(&command.program);
    builder.args(&command.args);
    if let Some(fd) = anchor {
        builder.current_dir(format!("/proc/self/fd/{}", fd.as_raw_fd()));
    } else {
        builder.current_dir(&command.cwd);
    }
    builder
        .env_clear()
        .envs(
            command
                .env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    builder.process_group(0);
    builder
}

async fn read_terminal_stream<R>(
    mut reader: R,
    budget: Arc<AtomicUsize>,
    truncated: Arc<AtomicBool>,
    retained_output: Arc<Mutex<String>>,
) where
    R: AsyncRead + Unpin,
{
    let mut chunk = [0_u8; 8192];
    while let Ok(read) = reader.read(&mut chunk).await {
        if read == 0 {
            break;
        }
        let retained = loop {
            let used = budget.load(Ordering::Acquire);
            if used >= MAX_TERMINAL_OUTPUT_BYTES {
                truncated.store(true, Ordering::Release);
                break 0;
            }
            let remaining = MAX_TERMINAL_OUTPUT_BYTES - used;
            let take = remaining.min(read);
            // Atomically reserve `take` bytes: a compare-exchange keeps
            // "read budget + deduct" race-free across concurrent stdout and
            // stderr reader tasks, so concurrent readers can never overdraw
            // or lose updates relative to the shared cap.
            match budget.compare_exchange(used, used + take, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    if take < read {
                        truncated.store(true, Ordering::Release);
                    }
                    break take;
                }
                // Another reader consumed budget concurrently; retry with
                // the up-to-date value the CAS returned.
                Err(_) => continue,
            }
        };
        if retained > 0 {
            let output = String::from_utf8_lossy(&chunk[..retained]).into_owned();
            retained_output
                .lock()
                .expect("terminal output lock")
                .push_str(&output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> TerminalManager {
        TerminalManager::new()
    }

    #[cfg(unix)]
    fn write_executable(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).expect("create script");
        file.write_all(format!("#!/bin/sh\n{body}\n").as_bytes())
            .expect("write script");
        file.set_permissions(std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");
        path
    }

    #[cfg(unix)]
    fn command(program: &Path, cwd: &Path, args: &[&str]) -> TerminalCommand {
        TerminalCommand {
            program: program.to_string_lossy().into_owned(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            cwd: cwd.to_path_buf(),
            env: test_env(),
        }
    }

    /// 测试用冻结会话环境:携带宿主 PATH(脚本内裸名工具经它解析),
    /// 其余键不伪造。
    fn test_env() -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        if let Some(path) = std::env::var_os("PATH") {
            env.insert("PATH".to_string(), path.to_string_lossy().into_owned());
        }
        env
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn full_lifecycle_create_wait_release() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "emit", "printf 'out'; printf 'err' >&2; exit 3");
        let manager = manager();
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        let result = manager.wait_for_exit(&id).await.expect("wait");
        assert_eq!(result.exit_code, Some(3));
        assert!(!result.timed_out);
        assert!(!result.killed);
        manager.release(&id).await.expect("release");
        manager.release(&id).await.expect("idempotent release");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_while_wait_pending_returns_killed() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "slow", "sleep 30");
        let manager = manager();
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        let wait = tokio::spawn({
            let manager = manager.clone();
            let id = id.clone();
            async move { manager.wait_for_exit(&id).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        manager.kill(&id).await.expect("kill");
        let result = wait.await.expect("wait task").expect("wait result");
        assert!(result.killed);
        assert_eq!(result.exit_code, None);
        manager.kill(&id).await.expect("idempotent kill");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fifth_terminal_is_rejected() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "slow", "sleep 30");
        let manager = manager();
        let mut ids = Vec::new();
        for _ in 0..MAX_TERMINALS {
            let id = manager
                .create(command(&bin, dir.path(), &[]))
                .await
                .expect("create within limit");
            ids.push(id);
        }
        let error = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect_err("fifth terminal must be rejected");
        assert_eq!(error, TerminalError::TooManyTerminals);
        manager.release(&ids[0]).await.expect("release");
        manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create after release");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unknown_id_is_distinguishable() {
        let manager = manager();
        assert_eq!(
            manager.wait_for_exit("term-missing").await,
            Err(TerminalError::UnknownTerminal("term-missing".to_string()))
        );
        assert_eq!(
            manager.kill("term-missing").await,
            Err(TerminalError::UnknownTerminal("term-missing".to_string()))
        );
        assert_eq!(
            manager.release("term-missing").await,
            Err(TerminalError::UnknownTerminal("term-missing".to_string()))
        );
    }

    /// 批2 Task 7（Bug45）：`TERMINAL_COMMAND_TIMEOUT_SECS=120` 独立上限
    /// 已删——命令时长的唯一收口是 kill/cancel 的进程组终止。本测试钉
    /// 进程组语义（直接子进程带孙进程；只杀直接子进程会留下孙进程，
    /// killpg 必须整组收割）；「无 120s 独立上限」由双层看门狗接线测试
    /// （kimi session_tests / coding provider_idle_stall）钉跨界行为。
    #[cfg(unix)]
    #[tokio::test]
    async fn kill_terminates_process_group_tree() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "slow-tree", "sleep 30 & sleep 30; exit 0");
        let manager = manager();
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        // 孙进程已起（组内两个 sleep）。
        tokio::time::sleep(Duration::from_millis(200)).await;
        manager.kill(&id).await.expect("kill");
        let result = tokio::time::timeout(Duration::from_secs(10), manager.wait_for_exit(&id))
            .await
            .expect("kill must bound the wait")
            .expect("wait result");
        assert!(result.killed);
        assert_eq!(result.exit_code, None);
        assert!(!result.timed_out);
    }

    // —— 唯一保留的真实进程端到端用例（DEF-7 桶收敛，D4）——
    // create→输出流→wait_for_exit→release 全链路 + cap 生效最小断言；
    // 预算 60s=机器负载不敏感口径（原三测试 10-15s 紧墙钟预算已废除）。
    #[cfg(unix)]
    #[tokio::test]
    async fn real_process_pipe_smoke_cap_end_to_end() {
        let dir = tempfile::tempdir().expect("dir");
        let script = format!(
            "head -c {} /dev/zero | tr '\\0' 'a'",
            MAX_TERMINAL_OUTPUT_BYTES + 1
        );
        let bin = write_executable(dir.path(), "smoke-cap", &script);
        let manager = manager();
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        let (result, combined) = tokio::time::timeout(Duration::from_secs(60), async {
            let result = manager.wait_for_exit(&id).await.expect("wait");
            let combined = manager.output(&id).expect("retained output").len();
            (result, combined)
        })
        .await
        .expect("real-process smoke timed out (60s load-insensitive budget)");
        assert_eq!(result.exit_code, Some(0));
        assert!(result.truncated);
        assert_eq!(combined, MAX_TERMINAL_OUTPUT_BYTES);
        manager.release(&id).await.expect("release");
        manager.release(&id).await.expect("idempotent release");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_retries_transient_etxtbsy_until_writer_releases() {
        let dir = tempfile::tempdir().expect("dir");
        // ETXTBSY（目标正被写打开）是瞬态错误：并发「写脚本→ exec」
        // 压力（实测定制内核/btrfs 组合下偶发）或异步句柄释放延迟都
        // 会命中。用真实写句柄确定性构造该状态，150ms 后异步释放；
        // run_terminal 必须退避重试到 spawn 成功，而不是把瞬态失败
        // 静默吞成「exit_code=None 的正常完成」。
        let bin = write_executable(dir.path(), "busy", "exit 0");
        let hold = std::fs::OpenOptions::new()
            .write(true)
            .open(&bin)
            .expect("hold writer open on script");
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            drop(hold);
        });
        let manager = manager();
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        let result = manager.wait_for_exit(&id).await.expect("wait");
        release.await.expect("release task");
        assert_eq!(
            result.exit_code,
            Some(0),
            "transient ETXTBSY must be retried, not swallowed: {result:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn wait_after_fast_exit_returns_immediately() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "fast", "exit 0");
        let manager = manager();
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        // Let the terminal finish long before the wait registers: Notify has
        // no permit, so wait_for_exit must observe the stored result instead
        // of hanging on notified().
        tokio::time::sleep(Duration::from_millis(300)).await;
        let result = tokio::time::timeout(Duration::from_secs(2), manager.wait_for_exit(&id))
            .await
            .expect("wait must not hang when the terminal already exited")
            .expect("wait result");
        assert_eq!(result.exit_code, Some(0));
        assert!(!result.timed_out);
        assert!(!result.killed);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_after_release_is_idempotent() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "slow", "sleep 30");
        let manager = manager();
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        manager.release(&id).await.expect("release");
        manager
            .kill(&id)
            .await
            .expect("kill after release must succeed (idempotent)");
        manager.release(&id).await.expect("idempotent release");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_start_frees_concurrency_quota() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "slow", "sleep 30");
        let manager = manager();
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("first start");
        let error = manager.start(&id).expect_err("second start must fail");
        assert_eq!(error, TerminalError::UnknownTerminal(id.clone()));
        manager
            .release(&id)
            .await
            .expect("release after failed start");
        // The quota consumed by the doomed create must be reusable.
        for _ in 0..MAX_TERMINALS {
            let id = manager
                .create(command(&bin, dir.path(), &[]))
                .await
                .expect("create within limit after failed start freed quota");
            manager.start(&id).expect("start");
        }
        manager.cleanup_all();
    }

    /// 批2 Task 6 红测:spawn 失败(如目标二进制不存在)必须释放并发
    /// 名额——失败终端不得永久占住 1/4 配额(现状:失败后状态停在
    /// Finished,未 Released,后续第 4 个 create 被顶死)。
    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_failure_releases_concurrency_quota() {
        let dir = tempfile::tempdir().expect("dir");
        let manager = manager();
        let id = manager
            .create(command(
                &dir.path().join("definitely-missing-bin"),
                dir.path(),
                &[],
            ))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        let result = manager.wait_for_exit(&id).await.expect("wait");
        assert_eq!(result.exit_code, None);
        assert!(!result.killed);

        // spawn 失败释放名额后:满额 4 个 create 必须全部成功。
        let bin = write_executable(dir.path(), "slow", "sleep 30");
        for _ in 0..MAX_TERMINALS {
            let id = manager
                .create(command(&bin, dir.path(), &[]))
                .await
                .expect("create within limit after spawn failure freed the slot");
            manager.start(&id).expect("start");
        }
        manager.cleanup_all();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cleanup_all_kills_live_terminals() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "slow", "sleep 30");
        let manager = manager();
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        manager.cleanup_all();
        let result = manager.wait_for_exit(&id).await.expect("wait");
        assert!(result.killed);
    }

    /// 批2 Task 7（Bug45）：真实活动 registry 生命周期钉——同一
    /// terminal_id 的 Started（登记）→ Running → 终态（Exited/Killed/
    /// Released/spawn 失败）全链在途性正确。双层看门狗消费的就是这份
    /// 状态（经 `TerminalActivitySnapshot`），不是事件日志。
    #[cfg(unix)]
    #[tokio::test]
    async fn activity_registry_tracks_lifecycle_states() {
        let dir = tempfile::tempdir().expect("dir");
        let fast = write_executable(dir.path(), "fast", "exit 0");
        let slow = write_executable(dir.path(), "slow", "sleep 30");
        let manager = manager();

        // Started（已登记，未 spawn）→ 在途。
        let fast_id = manager
            .create(command(&fast, dir.path(), &[]))
            .await
            .expect("create");
        assert!(manager.has_active_terminals(), "登记即 Started（在途）");

        // Running → 退出（Exited）→ 移除。
        manager.start(&fast_id).expect("start");
        assert!(manager.has_active_terminals(), "Running 在途");
        manager.wait_for_exit(&fast_id).await.expect("wait");
        assert!(
            !manager.has_active_terminals(),
            "Exited 移除在途状态（不留永久豁免）"
        );

        // Running → kill（Killed）→ 移除。
        let slow_id = manager
            .create(command(&slow, dir.path(), &[]))
            .await
            .expect("create slow");
        manager.start(&slow_id).expect("start slow");
        assert!(manager.has_active_terminals());
        manager.kill(&slow_id).await.expect("kill");
        manager.wait_for_exit(&slow_id).await.expect("wait slow");
        assert!(!manager.has_active_terminals(), "Killed 移除在途状态");

        // Running → release（Released）→ 移除（幂等重复 release 不复活）。
        let rel_id = manager
            .create(command(&slow, dir.path(), &[]))
            .await
            .expect("create rel");
        manager.start(&rel_id).expect("start rel");
        manager.release(&rel_id).await.expect("release");
        manager.release(&rel_id).await.expect("idempotent release");
        manager.wait_for_exit(&rel_id).await.expect("wait rel");
        assert!(!manager.has_active_terminals(), "Released 移除在途状态");

        // spawn 失败（创建失败）→ 移除（不占名额也不在途）。
        let fail_id = manager
            .create(command(
                &dir.path().join("definitely-missing-bin"),
                dir.path(),
                &[],
            ))
            .await
            .expect("create doomed");
        manager.start(&fail_id).expect("start doomed");
        manager.wait_for_exit(&fail_id).await.expect("wait doomed");
        assert!(!manager.has_active_terminals(), "创建失败移除在途状态");
    }

    /// 批2 Task 7 修复轮(F2)钉测：cleanup_all 之后才登记并 spawn 的
    /// terminal（「过门 create 与 dispatcher Drop 并发」的竞态形态）必须
    /// 被 manager 级收编 token 级联收割——无逃逸登记、无无上限孤儿进程。
    #[cfg(unix)]
    #[tokio::test]
    async fn cleanup_all_reaps_terminals_registered_after_it() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "late-slow", "sleep 30");
        let manager = manager();
        // dispatcher Drop 形态：先 cleanup_all（此刻 registry 为空）。
        manager.cleanup_all();
        // 竞态窗口内插入的登记 + spawn。
        let late = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("late create still registers");
        manager.start(&late).expect("late start");
        let result = tokio::time::timeout(Duration::from_secs(10), manager.wait_for_exit(&late))
            .await
            .expect("late terminal must be reaped by the cascade, not left unbounded")
            .expect("wait late");
        assert!(result.killed, "cleanup 级联必须收割晚登记进程: {result:?}");
        assert!(!manager.has_active_terminals(), "收割后不得残留在途登记");
    }
    // —— DEF-7 桶确定性边界族（D4/kimi-acp MODIFIED 确定性验证条款）——
    // 受控内存流（tokio::io::duplex）直接驱动 read_terminal_stream 的预算/截断
    // 逻辑：无真实进程、无墙钟预算，任何机器负载下结果恒定。真实进程端到端
    // 行为由下方唯一 smoke 覆盖（原三测试的真实进程形态已删除，不保留双份）。

    async fn drive_read_terminal_stream(
        payloads: &[Vec<u8>],
    ) -> (Arc<AtomicUsize>, Arc<AtomicBool>, String) {
        use tokio::io::AsyncWriteExt;

        let budget = Arc::new(AtomicUsize::new(0));
        let truncated = Arc::new(AtomicBool::new(false));
        let retained = Arc::new(Mutex::new(String::new()));
        let mut readers = Vec::new();
        let mut writers = Vec::new();
        for payload in payloads {
            let (mut writer, reader) = tokio::io::duplex(64 * 1024);
            readers.push(tokio::spawn(read_terminal_stream(
                reader,
                budget.clone(),
                truncated.clone(),
                retained.clone(),
            )));
            let payload = payload.clone();
            writers.push(tokio::spawn(async move {
                writer.write_all(&payload).await.expect("write payload");
                writer.shutdown().await.expect("shutdown writer");
            }));
        }
        for writer in writers {
            writer.await.expect("writer task");
        }
        for reader in readers {
            reader.await.expect("reader task");
        }
        let output = retained.lock().expect("terminal output lock").clone();
        (budget, truncated, output)
    }

    #[tokio::test]
    async fn cap_boundary_exactly_at_cap_retains_all_without_truncation_deterministic() {
        let (budget, truncated, output) =
            drive_read_terminal_stream(&[vec![b'a'; MAX_TERMINAL_OUTPUT_BYTES]]).await;
        assert_eq!(output.len(), MAX_TERMINAL_OUTPUT_BYTES);
        assert!(!truncated.load(Ordering::Acquire));
        assert_eq!(budget.load(Ordering::Acquire), MAX_TERMINAL_OUTPUT_BYTES);
    }

    #[tokio::test]
    async fn cap_boundary_over_cap_truncates_once_deterministic() {
        let (budget, truncated, output) =
            drive_read_terminal_stream(&[vec![b'a'; MAX_TERMINAL_OUTPUT_BYTES + 1]]).await;
        assert_eq!(output.len(), MAX_TERMINAL_OUTPUT_BYTES);
        assert!(output.bytes().all(|byte| byte == b'a'));
        assert!(truncated.load(Ordering::Acquire));
        assert_eq!(budget.load(Ordering::Acquire), MAX_TERMINAL_OUTPUT_BYTES);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cap_boundary_concurrent_streams_share_budget_without_overdraw_deterministic() {
        let (budget, truncated, output) = drive_read_terminal_stream(&[
            vec![b'a'; MAX_TERMINAL_OUTPUT_BYTES],
            vec![b'b'; MAX_TERMINAL_OUTPUT_BYTES],
        ])
        .await;
        // 交错不敏感不变式：合计恰为上限，不超额、不丢更新（CAS 预留语义）。
        assert_eq!(output.len(), MAX_TERMINAL_OUTPUT_BYTES);
        assert!(truncated.load(Ordering::Acquire));
        assert_eq!(budget.load(Ordering::Acquire), MAX_TERMINAL_OUTPUT_BYTES);
    }
}
