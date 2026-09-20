//! Terminal execution manager for the kimi `terminal/*` client services.
//!
//! Every terminal runs as `pipe` + process group + per-command timeout +
//! combined stdout/stderr output cap, with a concurrency limit of
//! `MAX_TERMINALS`. Lifecycle is `created -> running -> (exited|killed) ->
//! released`; `kill`/`release` are idempotent and unknown ids are
//! distinguishable errors. The caller resolves the trusted binary and builds
//! the argv through the closed grammar before reaching this module.

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

use super::sandbox::{build_bwrap_args, open_dir_no_follow_inherit, trusted_path_env};

pub const MAX_TERMINALS: usize = 4;
pub const MAX_TERMINAL_OUTPUT_BYTES: usize = 1_048_576;
pub const TERMINAL_COMMAND_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalIsolation {
    /// Run inside a bubblewrap sandbox. `writable_root` selects the mount
    /// mode of the authorized root (and the anchored cwd): the coding
    /// (Executor) role needs a read-write worktree to honour its TDD write
    /// path + commit responsibility (F-17), while every other role keeps the
    /// read-only mount. `writable_git_paths` carries git dir paths resolved
    /// outside the root (linked worktree) that are bound read-write with it.
    /// Everything else outside the authorized root stays read-only.
    Bubblewrap {
        bwrap: PathBuf,
        writable_root: bool,
        writable_git_paths: Vec<PathBuf>,
    },
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct TerminalCommand {
    /// Full argv excluding the binary name (e.g. `["status"]` for git). The
    /// binary is resolved to a trusted absolute path by the caller.
    pub argv: Vec<String>,
    /// Absolute trusted path of the executable.
    pub binary: PathBuf,
    /// Authorized root (canonical) — bind-mounted read-only (or read-write
    /// for the coding role, see [`TerminalIsolation::Bubblewrap`]) inside
    /// bwrap.
    pub root: PathBuf,
    /// Verified working directory (inside the authorized root).
    pub cwd: PathBuf,
    /// OS-level isolation selected by the permission policy.
    pub isolation: TerminalIsolation,
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
    fn new(command: TerminalCommand) -> Self {
        Self {
            state: Mutex::new(TerminalState::Created),
            kill: CancellationToken::new(),
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
pub struct TerminalManager {
    inner: Arc<Mutex<ManagerInner>>,
    timeout: Duration,
}

impl Clone for TerminalManager {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            timeout: self.timeout,
        }
    }
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManagerInner::default())),
            timeout: Duration::from_secs(TERMINAL_COMMAND_TIMEOUT_SECS),
        }
    }

    #[cfg(test)]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
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
        inner
            .terminals
            .insert(id.clone(), Arc::new(TerminalEntry::new(command)));
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
        let manager = self.clone();
        tokio::spawn(run_terminal(entry, command, manager));
        Ok(())
    }

    /// Wait for a terminal to exit (or be killed / time out).
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

    /// Kill every live terminal (session cancel / teardown).
    pub fn cleanup_all(&self) {
        let inner = self.inner.lock().expect("terminal manager lock");
        for entry in inner.terminals.values() {
            entry.kill.cancel();
        }
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

fn terminal_environment(root: &Path) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("PATH".to_string(), trusted_path_env());
    env.insert("HOME".to_string(), root.to_string_lossy().into_owned());
    env.insert("TMPDIR".to_string(), "/tmp".to_string());
    env.insert("LANG".to_string(), "C.UTF-8".to_string());
    env.insert("LC_ALL".to_string(), "C".to_string());
    env.insert("TERM".to_string(), "dumb".to_string());
    env.insert("GIT_CONFIG_NOSYSTEM".to_string(), "1".to_string());
    env.insert("GIT_CONFIG_GLOBAL".to_string(), "/dev/null".to_string());
    env.insert("GIT_CONFIG_SYSTEM".to_string(), "/dev/null".to_string());
    env.insert("GIT_CONFIG_COUNT".to_string(), "0".to_string());
    env
}

async fn run_terminal(
    entry: Arc<TerminalEntry>,
    command: TerminalCommand,
    manager: TerminalManager,
) {
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
    let env = terminal_environment(&command.root);
    let mut builder = build_terminal_command(&command, &env, &command.isolation, anchor.as_ref());
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
                *state = TerminalState::Finished;
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

    enum Completion {
        Exited(std::io::Result<std::process::ExitStatus>),
        TimedOut,
        Killed,
    }

    let completion = tokio::select! {
        status = child.wait() => Completion::Exited(status),
        _ = tokio::time::sleep(manager.timeout) => Completion::TimedOut,
        _ = entry.kill.cancelled() => Completion::Killed,
    };

    let (exit_code, timed_out, killed) = match completion {
        Completion::Exited(status) => (status.ok().and_then(|status| status.code()), false, false),
        Completion::TimedOut => {
            child.terminate().await;
            (None, true, false)
        }
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

fn build_terminal_command(
    command: &TerminalCommand,
    env: &BTreeMap<String, String>,
    isolation: &TerminalIsolation,
    anchor: Option<&OwnedFd>,
) -> Command {
    let mut builder = match isolation {
        TerminalIsolation::Bubblewrap {
            bwrap,
            writable_root,
            writable_git_paths,
        } => {
            let mut builder = Command::new(bwrap);
            let args = build_bwrap_args(
                &command.root,
                &command.cwd,
                anchor.map(AsRawFd::as_raw_fd),
                *writable_root,
                writable_git_paths,
                env,
                &command.binary,
                &command.argv,
            );
            builder.args(args);
            builder
        }
        TerminalIsolation::Unavailable => {
            let mut builder = Command::new(&command.binary);
            builder.args(&command.argv);
            if let Some(fd) = anchor {
                builder.current_dir(format!("/proc/self/fd/{}", fd.as_raw_fd()));
            } else {
                builder.current_dir(&command.cwd);
            }
            builder
        }
    };
    builder
        .env_clear()
        .envs(env.iter().map(|(key, value)| (key.clone(), value.clone())))
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

    fn manager(timeout: Duration) -> TerminalManager {
        TerminalManager::new().with_timeout(timeout)
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
    fn command(binary: &Path, cwd: &Path, argv: &[&str]) -> TerminalCommand {
        TerminalCommand {
            argv: argv.iter().map(|arg| arg.to_string()).collect(),
            binary: binary.to_path_buf(),
            root: cwd.to_path_buf(),
            cwd: cwd.to_path_buf(),
            isolation: TerminalIsolation::Unavailable,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn full_lifecycle_create_wait_release() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "emit", "printf 'out'; printf 'err' >&2; exit 3");
        let manager = manager(Duration::from_secs(5));
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
        let manager = manager(Duration::from_secs(30));
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
        let manager = manager(Duration::from_secs(30));
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
        let manager = manager(Duration::from_secs(5));
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

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_terminates_process_group() {
        let dir = tempfile::tempdir().expect("dir");
        // The child spawns a grandchild so process-group kill semantics are
        // exercised: killing only the direct child would leave the grandchild.
        let bin = write_executable(dir.path(), "slow-tree", "sleep 30 & sleep 30; exit 0");
        let manager = manager(Duration::from_millis(200));
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        let result = manager.wait_for_exit(&id).await.expect("wait");
        assert!(result.timed_out);
        assert_eq!(result.exit_code, None);
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
        let manager = manager(Duration::from_secs(60));
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
        let manager = manager(Duration::from_secs(30));
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
        let manager = manager(Duration::from_secs(5));
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
        let manager = manager(Duration::from_secs(30));
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

    // —— F-17:kimi coder 承包契约(TDD 写路径 + commit 责任)与只读终端
    // 沙箱的系统性冲突——授权根 ro-bind 下终端内 `git add/commit` 必死于
    // index.lock 只读,coder 只能判 plan defect 掷骰 blocked 门。修复 =
    // Executor 角色的授权根以 rw bind-mount 进入沙箱(宿主其余路径仍只
    // 读、网络/pid 隔离不变);其他角色保持只读挂载。真实 bwrap 端到端。
    #[cfg(unix)]
    fn bwrap_isolation(
        writable_root: bool,
        writable_git_paths: Vec<PathBuf>,
    ) -> Option<TerminalIsolation> {
        use super::super::sandbox::probe_bwrap;
        probe_bwrap().map(|bwrap| TerminalIsolation::Bubblewrap {
            bwrap,
            writable_root,
            writable_git_paths,
        })
    }

    #[cfg(unix)]
    fn host_git(args: &[&str], cwd: &Path) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("host git");
        assert!(
            status.success(),
            "host git {args:?} failed in {}",
            cwd.display()
        );
    }

    #[cfg(unix)]
    async fn run_bwrap_script(
        root: &Path,
        isolation: TerminalIsolation,
        script: &str,
    ) -> (TerminalResult, String) {
        use super::super::sandbox::resolve_trusted_binary;
        let manager = manager(Duration::from_secs(60));
        let command = TerminalCommand {
            argv: vec!["-c".to_string(), script.to_string()],
            binary: resolve_trusted_binary("bash").expect("trusted bash"),
            root: root.to_path_buf(),
            cwd: root.to_path_buf(),
            isolation,
        };
        let id = manager.create(command).await.expect("create");
        manager.start(&id).expect("start");
        // 60s 负载不敏感口径(与真实进程 smoke 一致)。
        let result = tokio::time::timeout(Duration::from_secs(60), manager.wait_for_exit(&id))
            .await
            .expect("sandbox script timed out (60s load-insensitive budget)")
            .expect("wait result");
        let output = manager.output(&id).expect("retained output");
        manager.release(&id).await.expect("release");
        (result, output)
    }

    /// 首轮红证据(标准仓形态):只读挂载下该脚本死于
    /// `index.lock: Read-only file system`。
    #[cfg(unix)]
    #[tokio::test]
    async fn executor_writable_root_sandbox_allows_git_commit_inside_root() {
        use super::super::sandbox::resolve_writable_git_paths;
        let dir = tempfile::tempdir().expect("worktree");
        let root = dir.path().canonicalize().expect("canonical root");
        let Some(isolation) = bwrap_isolation(true, resolve_writable_git_paths(&root)) else {
            return; // bubblewrap 不在时跳过真机隔离路径
        };
        let script = "git init -q \
                      && echo f17 > f17.txt \
                      && git add f17.txt \
                      && git -c user.name=f17 -c user.email=f17@example.com commit -qm f17 \
                      && echo COMMIT_OK";
        let (result, output) = run_bwrap_script(&root, isolation, script).await;
        assert_eq!(
            result.exit_code,
            Some(0),
            "coder contract: git add/commit must succeed inside the writable \
             worktree; output: {output}"
        );
        assert!(output.contains("COMMIT_OK"), "{output}");
        assert!(root.join(".git").is_dir(), "committed worktree");
        assert!(root.join("f17.txt").is_file());
    }

    /// fix round 1 主证据(P1/P2,生产形态):aria coding attempt 的
    /// worktree 是 git linked worktree——`.git` 是指针,gitdir/commondir
    /// 位于授权根外 `<repo>/.git/worktrees/<name>`,rw root bind 覆盖不到
    /// → git add/commit 仍死于 `index.lock: Read-only file system`
    /// (attempt 544a1b51 现场形态)。修复 = 服务端 rev-parse 解析根外
    /// git 路径(commondir 一条覆盖两者)随 rw root 一并 bind。
    /// 绑定前必红。
    #[cfg(unix)]
    #[tokio::test]
    async fn executor_writable_root_sandbox_allows_git_commit_in_linked_worktree() {
        use super::super::sandbox::resolve_writable_git_paths;
        let base = tempfile::tempdir().expect("workspace");
        let base = base.path().canonicalize().expect("canonical base");
        let main = base.join("main");
        host_git(&["init", "-q", "main"], &base);
        host_git(
            &[
                "-c",
                "user.name=f17",
                "-c",
                "user.email=f17@example.com",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "base",
            ],
            &main,
        );
        host_git(&["worktree", "add", "../wt", "-b", "f17wt"], &main);
        let root = main.parent().unwrap().join("wt");
        let root = root.canonicalize().expect("canonical worktree");
        assert!(root.join(".git").is_file(), "linked worktree shape");
        let git_paths = resolve_writable_git_paths(&root);
        assert_eq!(
            git_paths,
            vec![main.join(".git").canonicalize().expect("commondir")],
            "only the out-of-root common dir needs an extra rw bind"
        );
        let Some(isolation) = bwrap_isolation(true, git_paths) else {
            return; // bubblewrap 不在时跳过真机隔离路径
        };
        let script = "echo f17 > f17.txt \
                      && git add f17.txt \
                      && git -c user.name=f17 -c user.email=f17@example.com commit -qm f17 \
                      && echo WT_COMMIT_OK";
        let (result, output) = run_bwrap_script(&root, isolation, script).await;
        assert_eq!(
            result.exit_code,
            Some(0),
            "linked worktree: git add/commit must reach the out-of-root \
             gitdir/commondir; output: {output}"
        );
        assert!(output.contains("WT_COMMIT_OK"), "{output}");
        assert!(root.join("f17.txt").is_file());
        host_git(&["log", "--oneline", "-1", "f17wt"], &main);
    }

    /// 安全边界回归锁 1:非 Executor 挂载语义不变——授权根内写仍被拒。
    #[cfg(unix)]
    #[tokio::test]
    async fn read_only_root_sandbox_still_blocks_writes_inside_root() {
        let Some(isolation) = bwrap_isolation(false, Vec::new()) else {
            return;
        };
        let dir = tempfile::tempdir().expect("worktree");

        let root = dir.path().canonicalize().expect("canonical root");
        let (result, output) =
            run_bwrap_script(&root, isolation, "touch ro-probe.txt; echo done").await;
        assert_eq!(result.exit_code, Some(0), "{output}");
        assert!(
            !root.join("ro-probe.txt").exists(),
            "read-only mount must keep rejecting writes inside the root"
        );
    }

    /// 安全边界回归锁 2:可写模式下宿主其余路径仍只读(ro-bind / / 不变)。
    #[cfg(unix)]
    #[tokio::test]
    async fn writable_root_sandbox_keeps_host_outside_root_read_only() {
        let Some(isolation) = bwrap_isolation(true, Vec::new()) else {
            return;
        };
        let dir = tempfile::tempdir().expect("worktree");
        let root = dir.path().canonicalize().expect("canonical root");
        let (result, output) =
            run_bwrap_script(&root, isolation, "touch /etc/f17-must-fail 2>&1; true").await;
        assert_eq!(result.exit_code, Some(0), "{output}");
        assert!(
            output.contains("Read-only file system"),
            "host outside the authorized root must stay read-only; output: {output}"
        );
        assert!(!Path::new("/etc/f17-must-fail").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_start_frees_concurrency_quota() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "slow", "sleep 30");
        let manager = manager(Duration::from_secs(30));
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

    #[cfg(unix)]
    #[tokio::test]
    async fn cleanup_all_kills_live_terminals() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "slow", "sleep 30");
        let manager = manager(Duration::from_secs(30));
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        manager.cleanup_all();
        let result = manager.wait_for_exit(&id).await.expect("wait");
        assert!(result.killed);
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
