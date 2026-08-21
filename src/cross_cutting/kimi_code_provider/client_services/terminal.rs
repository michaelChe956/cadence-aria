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
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

use super::sandbox::{build_bwrap_args, open_dir_no_follow_inherit, trusted_path_env};

pub const MAX_TERMINALS: usize = 4;
pub const MAX_TERMINAL_OUTPUT_BYTES: usize = 1_048_576;
pub const TERMINAL_COMMAND_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalIsolation {
    Bubblewrap { bwrap: PathBuf },
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct TerminalCommand {
    /// Full argv excluding the binary name (e.g. `["status"]` for git). The
    /// binary is resolved to a trusted absolute path by the caller.
    pub argv: Vec<String>,
    /// Absolute trusted path of the executable.
    pub binary: PathBuf,
    /// Authorized root (canonical) — read-only bind-mount inside bwrap.
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
pub enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    pub fn as_str(self) -> &'static str {
        match self {
            OutputStream::Stdout => "stdout",
            OutputStream::Stderr => "stderr",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TerminalOutputEvent {
    pub terminal_id: String,
    pub stream: OutputStream,
    pub output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalState {
    Created,
    Running,
    Finished,
    Released,
}

struct TerminalEntry {
    id: String,
    state: Mutex<TerminalState>,
    kill: CancellationToken,
    done: Arc<Notify>,
    result: Mutex<Option<TerminalResult>>,
    command: Mutex<Option<TerminalCommand>>,
}

impl TerminalEntry {
    fn new(id: String, command: TerminalCommand) -> Self {
        Self {
            id,
            state: Mutex::new(TerminalState::Created),
            kill: CancellationToken::new(),
            done: Arc::new(Notify::new()),
            result: Mutex::new(None),
            command: Mutex::new(Some(command)),
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
    output_tx: mpsc::Sender<TerminalOutputEvent>,
    timeout: Duration,
}

impl Clone for TerminalManager {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            output_tx: self.output_tx.clone(),
            timeout: self.timeout,
        }
    }
}

impl TerminalManager {
    pub fn new(output_tx: mpsc::Sender<TerminalOutputEvent>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManagerInner::default())),
            output_tx,
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
        inner.terminals.insert(
            id.clone(),
            Arc::new(TerminalEntry::new(id.clone(), command)),
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
    let mut child = match builder.spawn() {
        Ok(child) => {
            let pgid = child.id().and_then(|pid| i32::try_from(pid).ok());
            TerminalChild { child, pgid }
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
    };

    let stdout = child.child.stdout.take();
    let stderr = child.child.stderr.take();

    let budget = Arc::new(AtomicUsize::new(0));
    let truncated = Arc::new(AtomicBool::new(false));
    let stdout_task = stdout.map(|stream| {
        tokio::spawn(read_terminal_stream(
            stream,
            OutputStream::Stdout,
            entry.id.clone(),
            Arc::clone(&budget),
            Arc::clone(&truncated),
            manager.output_tx.clone(),
        ))
    });
    let stderr_task = stderr.map(|stream| {
        tokio::spawn(read_terminal_stream(
            stream,
            OutputStream::Stderr,
            entry.id.clone(),
            Arc::clone(&budget),
            Arc::clone(&truncated),
            manager.output_tx.clone(),
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
        TerminalIsolation::Bubblewrap { bwrap } => {
            let mut builder = Command::new(bwrap);
            let args = build_bwrap_args(
                &command.root,
                &command.cwd,
                anchor.map(AsRawFd::as_raw_fd),
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
    stream: OutputStream,
    terminal_id: String,
    budget: Arc<AtomicUsize>,
    truncated: Arc<AtomicBool>,
    output_tx: mpsc::Sender<TerminalOutputEvent>,
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
            if output_tx
                .send(TerminalOutputEvent {
                    terminal_id: terminal_id.clone(),
                    stream,
                    output,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager(timeout: Duration) -> (TerminalManager, mpsc::Receiver<TerminalOutputEvent>) {
        let (output_tx, output_rx) = mpsc::channel(64);
        let manager = TerminalManager::new(output_tx).with_timeout(timeout);
        (manager, output_rx)
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
        let (manager, _output) = manager(Duration::from_secs(5));
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
        let (manager, _output) = manager(Duration::from_secs(30));
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
        let (manager, _output) = manager(Duration::from_secs(30));
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
        let (manager, _output) = manager(Duration::from_secs(5));
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
        let (manager, _output) = manager(Duration::from_millis(200));
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        let result = manager.wait_for_exit(&id).await.expect("wait");
        assert!(result.timed_out);
        assert_eq!(result.exit_code, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn output_is_capped_and_truncation_flagged_once() {
        let dir = tempfile::tempdir().expect("dir");
        // Emit exactly MAX+1 bytes on a single stream (stdout) so the test is
        // deterministic: no cross-stream budget race, no timing dependence.
        // The cap must retain the first MAX bytes and flag truncation once.
        let script = format!(
            "head -c {} /dev/zero | tr '\\0' 'a'",
            MAX_TERMINAL_OUTPUT_BYTES + 1
        );
        let bin = write_executable(dir.path(), "big", &script);
        let (manager, mut output_rx) = manager(Duration::from_secs(30));
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        let (result, combined) = tokio::time::timeout(Duration::from_secs(10), async {
            let wait = manager.wait_for_exit(&id);
            tokio::pin!(wait);
            let mut combined = 0usize;
            let result = loop {
                tokio::select! {
                    result = &mut wait => break result.expect("wait"),
                    event = output_rx.recv() => {
                        combined += event.expect("output channel closed").output.len();
                    }
                }
            };
            while let Ok(event) = output_rx.try_recv() {
                combined += event.output.len();
            }
            (result, combined)
        })
        .await
        .expect("terminal output test timed out");
        assert!(result.truncated);
        assert_eq!(combined, MAX_TERMINAL_OUTPUT_BYTES);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn output_exactly_at_cap_is_not_truncated() {
        let dir = tempfile::tempdir().expect("dir");
        // Emit exactly MAX bytes on stdout: everything must be returned and
        // truncation must not be flagged (boundary case, MAX = 1048576).
        let script = format!(
            "head -c {} /dev/zero | tr '\\0' 'a'",
            MAX_TERMINAL_OUTPUT_BYTES
        );
        let bin = write_executable(dir.path(), "exact", &script);
        let (manager, mut output_rx) = manager(Duration::from_secs(30));
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        let (result, combined) = tokio::time::timeout(Duration::from_secs(10), async {
            let wait = manager.wait_for_exit(&id);
            tokio::pin!(wait);
            let mut combined = 0usize;
            let result = loop {
                tokio::select! {
                    result = &mut wait => break result.expect("wait"),
                    event = output_rx.recv() => {
                        combined += event.expect("output channel closed").output.len();
                    }
                }
            };
            while let Ok(event) = output_rx.try_recv() {
                combined += event.output.len();
            }
            (result, combined)
        })
        .await
        .expect("terminal output test timed out");
        assert_eq!(result.exit_code, Some(0));
        assert!(!result.truncated);
        assert_eq!(combined, MAX_TERMINAL_OUTPUT_BYTES);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_streams_share_budget_without_overdraw() {
        let dir = tempfile::tempdir().expect("dir");
        // Both stdout and stderr each emit MAX bytes concurrently. The shared
        // cap must still be honored exactly: total retained output == MAX and
        // truncation is flagged. With the old load/store race this could
        // overdraw; the atomic CAS reservation keeps it exact under any
        // interleaving.
        let script = format!(
            "head -c {} /dev/zero | tr '\\0' 'a' > /dev/stdout & head -c {} /dev/zero | tr '\\0' 'b' > /dev/stderr & wait",
            MAX_TERMINAL_OUTPUT_BYTES, MAX_TERMINAL_OUTPUT_BYTES
        );
        let bin = write_executable(dir.path(), "dual", &script);
        let (manager, mut output_rx) = manager(Duration::from_secs(30));
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        let (result, combined) = tokio::time::timeout(Duration::from_secs(15), async {
            let wait = manager.wait_for_exit(&id);
            tokio::pin!(wait);
            let mut combined = 0usize;
            let result = loop {
                tokio::select! {
                    result = &mut wait => break result.expect("wait"),
                    event = output_rx.recv() => {
                        combined += event.expect("output channel closed").output.len();
                    }
                }
            };
            while let Ok(event) = output_rx.try_recv() {
                combined += event.output.len();
            }
            (result, combined)
        })
        .await
        .expect("concurrent dual-stream test timed out");
        assert!(result.truncated);
        assert_eq!(combined, MAX_TERMINAL_OUTPUT_BYTES);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn wait_after_fast_exit_returns_immediately() {
        let dir = tempfile::tempdir().expect("dir");
        let bin = write_executable(dir.path(), "fast", "exit 0");
        let (manager, _output) = manager(Duration::from_secs(5));
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
        let (manager, _output) = manager(Duration::from_secs(30));
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
        let (manager, _output) = manager(Duration::from_secs(30));
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
        let (manager, _output) = manager(Duration::from_secs(30));
        let id = manager
            .create(command(&bin, dir.path(), &[]))
            .await
            .expect("create");
        manager.start(&id).expect("start");
        manager.cleanup_all();
        let result = manager.wait_for_exit(&id).await.expect("wait");
        assert!(result.killed);
    }
}
