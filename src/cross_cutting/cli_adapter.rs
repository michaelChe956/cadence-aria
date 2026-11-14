use crate::cross_cutting::adapter_compatibility::{
    AdapterCompatibilityEntry, CommandSpec, PromptInputMode, StructuredOutputMode,
};
use crate::cross_cutting::document_ops::compute_sha256;
use crate::cross_cutting::process_manager::ProcessStartIdentity;
#[cfg(unix)]
use crate::cross_cutting::process_manager::signal_group_if_live_identity;
use crate::cross_cutting::provider_adapter::{
    ProviderAdapter, ProviderAdapterError, STRUCTURED_OUTPUT_END, parse_last_structured_output,
};
use crate::protocol::contracts::{AdapterInput, AdapterOutput, ProviderType, TimeoutStatus};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutputChunk {
    pub provider_type: ProviderType,
    pub output_schema: String,
    pub stream: String,
    pub text: String,
}

pub type ProviderOutputSink = Arc<dyn Fn(CliOutputChunk) + Send + Sync + 'static>;

#[derive(Clone)]
pub struct CliAdapterConfig {
    pub compatibility: AdapterCompatibilityEntry,
    pub expected_artifact_kind: Option<String>,
    pub output_sink: Option<ProviderOutputSink>,
}

impl fmt::Debug for CliAdapterConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CliAdapterConfig")
            .field("compatibility", &self.compatibility)
            .field("expected_artifact_kind", &self.expected_artifact_kind)
            .field("output_sink", &self.output_sink.as_ref().map(|_| "<sink>"))
            .finish()
    }
}

pub struct CliProviderAdapter {
    config: CliAdapterConfig,
}

impl CliProviderAdapter {
    pub fn new(config: CliAdapterConfig) -> Self {
        Self { config }
    }
}

impl ProviderAdapter for CliProviderAdapter {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        let mut command = self.config.compatibility.run_command.clone();
        if self.config.compatibility.pass_worktree_path_as_arg
            && let Some(worktree_path) = &input.worktree_path
        {
            command.args.push(worktree_path.clone());
        }

        let worktree_path = input.worktree_path.as_deref().map(Path::new);
        let before_files = match worktree_path {
            Some(path) => collect_file_hashes(path).unwrap_or_default(),
            None => BTreeMap::new(),
        };

        let stream_context = self
            .config
            .output_sink
            .as_ref()
            .map(|sink| OutputStreamContext {
                provider_type: input.provider_type.clone(),
                output_schema: input.output_schema.clone(),
                sink: Arc::clone(sink),
            });
        let stream_log_dir = input.provider_stream_log_dir.as_deref().map(Path::new);
        // A1-a(Bug45 批2 环境回退底座):同步 CLI 旁路不再构造显式空环境
        // ——`env_vars` 非空时作为调用方显式覆盖层注入,空时传 None(平台
        // 不构造、不覆盖),子进程直接继承宿主环境;绝不把宿主 env 抄进
        // BTreeMap 冒充继承(那会丢非 UTF-8 值)。
        let environment = if input.env_vars.is_empty() {
            None
        } else {
            Some(&input.env_vars)
        };
        let raw = run_command_capture_with_stream(
            &command,
            worktree_path,
            stream_log_dir,
            environment,
            Some(&input.prompt),
            Some(Duration::from_secs(input.timeout)),
            stream_context,
        )
        .map_err(|error| self.classify_error(error))?;

        let after_files = match worktree_path {
            Some(path) => collect_file_hashes(path).unwrap_or_default(),
            None => BTreeMap::new(),
        };
        let files_modified = diff_files(&before_files, &after_files);

        if raw.exit_code != Some(0) {
            return Err(self.classify_error(ProviderAdapterError::execution_failed(
                raw.exit_code,
                raw.stdout,
                raw.stderr,
                raw.duration_ms,
            )));
        }

        if self.config.compatibility.structured_output_mode != StructuredOutputMode::SentinelJson {
            return Err(ProviderAdapterError::incompatible_output(
                "unsupported structured output mode".to_string(),
                raw.stdout,
                raw.stderr,
            ));
        }
        let structured_output = parse_last_structured_output(&raw.stdout)?.ok_or_else(|| {
            ProviderAdapterError::parse_error(
                "missing structured output sentinel",
                raw.stdout.clone(),
                raw.stderr.clone(),
            )
        })?;
        if let Some(expected) =
            expected_artifact_kind(input, self.config.expected_artifact_kind.as_deref())
        {
            let got = structured_output
                .get("artifact_kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if got != expected {
                return Err(ProviderAdapterError::incompatible_output(
                    format!("expected artifact_kind {expected}, got {got}"),
                    raw.stdout,
                    raw.stderr,
                ));
            }
        }

        Ok(AdapterOutput {
            exit_code: raw.exit_code,
            stdout: raw.stdout,
            stderr: raw.stderr,
            structured_output: Some(structured_output),
            files_modified,
            duration_ms: raw.duration_ms,
            timeout_status: raw.timeout_status,
        })
    }
}

impl CliProviderAdapter {
    fn classify_error(&self, error: ProviderAdapterError) -> ProviderAdapterError {
        let combined = format!("{} {}", error.stdout, error.stderr).to_lowercase();
        if self
            .config
            .compatibility
            .unauthorized_patterns
            .iter()
            .any(|pattern| combined.contains(&pattern.to_lowercase()))
        {
            return ProviderAdapterError::unauthorized(error.details, error.stdout, error.stderr);
        }
        if self
            .config
            .compatibility
            .permission_denied_patterns
            .iter()
            .any(|pattern| combined.contains(&pattern.to_lowercase()))
        {
            return ProviderAdapterError::permission_denied(
                error.details,
                error.stdout,
                error.stderr,
            );
        }
        error
    }
}

#[derive(Debug, Clone)]
pub struct CapturedCommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timeout_status: TimeoutStatus,
}

pub fn run_command_capture(
    command_spec: &CommandSpec,
    current_dir: Option<&Path>,
    stdin_text: Option<&str>,
    timeout: Option<Duration>,
) -> Result<CapturedCommandOutput, ProviderAdapterError> {
    run_command_capture_with_stream(
        command_spec,
        current_dir,
        None,
        None,
        stdin_text,
        timeout,
        None,
    )
}

/// `current_dir` 是 provider 子进程的工作目录（目标仓库），`stream_log_dir` 是
/// 流日志落盘目录（Aria 侧）。两者语义独立：`stream_log_dir` 为 `None` 时不写
/// 流日志，且不得回退到 `current_dir`。
fn run_command_capture_with_stream(
    command_spec: &CommandSpec,
    current_dir: Option<&Path>,
    stream_log_dir: Option<&Path>,
    environment: Option<&BTreeMap<String, String>>,
    stdin_text: Option<&str>,
    timeout: Option<Duration>,
    stream_context: Option<OutputStreamContext>,
) -> Result<CapturedCommandOutput, ProviderAdapterError> {
    let started = Instant::now();
    let mut command = Command::new(&command_spec.program);
    command.args(&command_spec.args);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    // A1-a(Bug45 批2 环境回退底座):独立 env_clear 已删——`Some(env)` 只是
    // 调用方显式覆盖层(envs 覆盖同名键),`None` 为平台不构造不覆盖;两条
    // 路径下子进程都继承宿主环境。环境具体值不写入流日志/错误信息。
    if let Some(env) = environment {
        command.envs(env);
    }
    // 计划 2026-11-13 包 2.4:PATH overlay 合并点——`environment=None`
    // (继承宿主 env)分支同样必须显式写合并后的 PATH,不能只在
    // `Some(env)` 分支合并;overlay 为空时 effective_path 原样返回 base。
    let base_path = environment
        .and_then(|env| env.get("PATH"))
        .map(String::as_str);
    command.env(
        "PATH",
        crate::cross_cutting::path_overlay::effective_path(base_path),
    );
    // 计划 2026-11-13 §五 2.12:补入登录 shell 探测回的 agent 专属环境
    // 变量(缺失才补;进程 env/覆盖层已有的值优先),None 分支同样补。
    for (name, value) in crate::cross_cutting::path_overlay::shell_env_overrides(environment) {
        command.env(name, value);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin_text.is_some() {
        command.stdin(Stdio::piped());
    }
    #[cfg(unix)]
    {
        // Put the provider in its own process group so timeout/early-completion cleanup
        // also terminates shell grandchildren that may otherwise keep stdout pipes open.
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
    }

    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ProviderAdapterError::command_missing(format!("{}: {error}", command_spec.program))
        } else {
            ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
        }
    })?;
    // PID 复用防护:spawn 即冻结组长 (pid, /proc starttime)。超时/提前完成
    // 的组清理只在身份仍匹配时发射——pid 复用后裸 killpg 会打掉无关进程组。
    let child_identity = ProcessStartIdentity::freeze(child.id());

    let stdout_stream_path =
        provider_stream_path(stream_log_dir, &command_spec.program, child.id(), "stdout");
    let stderr_stream_path =
        provider_stream_path(stream_log_dir, &command_spec.program, child.id(), "stderr");
    let stdout_reader = child.stdout.take().map(|reader| {
        spawn_output_reader(reader, stdout_stream_path, "stdout", stream_context.clone())
    });
    let stderr_reader = child
        .stderr
        .take()
        .map(|reader| spawn_output_reader(reader, stderr_stream_path, "stderr", stream_context));

    if let Some(stdin_text) = stdin_text
        && let Some(mut stdin) = child.stdin.take()
    {
        // Provider 进程可能在我们写完之前就退出（例如 fixture 脚本立即报错），
        // 此时 write 会返回 BrokenPipe。BrokenPipe 不是错误本身，真正的错误来源
        // 在子进程的 stderr 与 exit_code 中，应让后续 classify_error 处理。
        if let Err(error) = stdin.write_all(stdin_text.as_bytes())
            && error.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                format!("write provider stdin: {error}"),
                0,
            ));
        }
    }

    let status = if let Some(timeout) = timeout {
        let mut structured_output_seen_at: Option<Instant> = None;
        loop {
            if let Some(status) = child.try_wait().map_err(|error| {
                ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
            })? {
                break status;
            }
            if structured_output_complete(&stdout_reader) {
                let first_seen = structured_output_seen_at.get_or_insert_with(Instant::now);
                if first_seen.elapsed() >= Duration::from_millis(500) {
                    terminate_child_group(&mut child, child_identity);
                    let _ = child.wait().map_err(|error| {
                        ProviderAdapterError::execution_failed(
                            None,
                            String::new(),
                            error.to_string(),
                            0,
                        )
                    })?;
                    let stdout = join_output_reader(stdout_reader)?;
                    let stderr = join_output_reader(stderr_reader)?;
                    return Ok(CapturedCommandOutput {
                        exit_code: Some(0),
                        stdout,
                        stderr,
                        duration_ms: started.elapsed().as_millis() as u64,
                        timeout_status: TimeoutStatus::NotTimedOut,
                    });
                }
            }
            if started.elapsed() >= timeout {
                terminate_child_group(&mut child, child_identity);
                let _ = child.wait().map_err(|error| {
                    ProviderAdapterError::execution_failed(
                        None,
                        String::new(),
                        error.to_string(),
                        0,
                    )
                })?;
                let stdout = join_output_reader(stdout_reader)?;
                let stderr = join_output_reader(stderr_reader)?;
                return Err(ProviderAdapterError::timeout(
                    stdout,
                    stderr,
                    started.elapsed().as_millis() as u64,
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    } else {
        child.wait().map_err(|error| {
            ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
        })?
    };

    let stdout = join_output_reader(stdout_reader)?;
    let stderr = join_output_reader(stderr_reader)?;
    Ok(CapturedCommandOutput {
        exit_code: status.code(),
        stdout,
        stderr,
        duration_ms: started.elapsed().as_millis() as u64,
        timeout_status: TimeoutStatus::NotTimedOut,
    })
}

#[derive(Clone)]
struct OutputStreamContext {
    provider_type: ProviderType,
    output_schema: String,
    sink: ProviderOutputSink,
}

struct OutputReader {
    buffer: Arc<Mutex<Vec<u8>>>,
    structured_output_complete: Arc<AtomicBool>,
    handle: JoinHandle<std::io::Result<()>>,
}

fn spawn_output_reader<R>(
    mut reader: R,
    stream_path: Option<PathBuf>,
    stream_name: &'static str,
    stream_context: Option<OutputStreamContext>,
) -> OutputReader
where
    R: Read + Send + 'static,
{
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let structured_output_complete = Arc::new(AtomicBool::new(false));
    let thread_buffer = Arc::clone(&buffer);
    let thread_structured_output_complete = Arc::clone(&structured_output_complete);
    let handle = std::thread::spawn(move || {
        let mut chunk = [0_u8; 8192];
        let sentinel = STRUCTURED_OUTPUT_END.as_bytes();
        let mut scan_tail = Vec::new();
        let mut stream_file = stream_path.as_ref().map(open_provider_stream).transpose()?;
        loop {
            let read = reader.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            let bytes = &chunk[..read];
            if !thread_structured_output_complete.load(Ordering::Relaxed) {
                let mut scan_window = scan_tail;
                scan_window.extend_from_slice(bytes);
                if contains_bytes(&scan_window, sentinel) {
                    thread_structured_output_complete.store(true, Ordering::Relaxed);
                }
                scan_tail = if scan_window.len() >= sentinel.len().saturating_sub(1) {
                    scan_window[scan_window
                        .len()
                        .saturating_sub(sentinel.len().saturating_sub(1))..]
                        .to_vec()
                } else {
                    scan_window
                };
            }
            if let Some(file) = stream_file.as_mut() {
                file.write_all(bytes)?;
            }
            if let Some(context) = stream_context.as_ref() {
                (context.sink)(CliOutputChunk {
                    provider_type: context.provider_type.clone(),
                    output_schema: context.output_schema.clone(),
                    stream: stream_name.to_string(),
                    text: String::from_utf8_lossy(bytes).to_string(),
                });
            }
            thread_buffer
                .lock()
                .expect("output buffer poisoned")
                .extend_from_slice(bytes);
        }
        Ok(())
    });
    OutputReader {
        buffer,
        structured_output_complete,
        handle,
    }
}

/// 生成流日志文件路径。`stream_log_dir` 由调用方提供 Aria 侧目录；为 `None`
/// 时返回 `None`（不写流日志），MUST NOT 回退到 provider 的工作目录，否则会在
/// 被开发的目标代码库中创建目录（change `fix-provider-stream-log-location`）。
fn provider_stream_path(
    stream_log_dir: Option<&Path>,
    program: &str,
    child_id: u32,
    stream_name: &str,
) -> Option<PathBuf> {
    let stream_log_dir = stream_log_dir?;
    // 非绝对路径（含空串）会把日志写到 Aria 进程的 cwd——后端若正好在用户仓库中
    // 启动，缺陷会以另一种形式复现。此处按缺省处理而不是猜测目录。
    if !stream_log_dir.is_absolute() {
        return None;
    }
    let provider_name = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    Some(stream_log_dir.join(format!("{provider_name}-{child_id}-{stream_name}.log")))
}

fn open_provider_stream(path: &PathBuf) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

fn join_output_reader(reader: Option<OutputReader>) -> Result<String, ProviderAdapterError> {
    let Some(reader) = reader else {
        return Ok(String::new());
    };
    let read_result = reader.handle.join().map_err(|_| {
        ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "provider output reader panicked".to_string(),
            0,
        )
    })?;
    read_result.map_err(|error| {
        ProviderAdapterError::execution_failed(
            None,
            String::new(),
            format!("read provider output: {error}"),
            0,
        )
    })?;
    let bytes = reader.buffer.lock().map_err(|_| {
        ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "provider output buffer poisoned".to_string(),
            0,
        )
    })?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn structured_output_complete(reader: &Option<OutputReader>) -> bool {
    let Some(reader) = reader else {
        return false;
    };
    reader.structured_output_complete.load(Ordering::Relaxed)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// 超时/提前完成的组清理：先按冻结身份核对再 killpg（pid 复用防护），
/// 随后回收直接子进程。身份不符只回收直接子进程，绝不向复用的 pgid 发信号。
fn terminate_child_group(child: &mut Child, identity: Option<ProcessStartIdentity>) {
    #[cfg(unix)]
    if let Ok(pgid) = i32::try_from(child.id()) {
        let _ = signal_group_if_live_identity(identity, pgid, libc::SIGKILL);
    }
    let _ = child.kill();
}

fn expected_artifact_kind<'a>(
    input: &'a AdapterInput,
    explicit: Option<&'a str>,
) -> Option<&'a str> {
    explicit.or_else(|| input.output_schema.split_once('.').map(|(kind, _)| kind))
}

fn collect_file_hashes(path: &Path) -> std::io::Result<BTreeMap<String, String>> {
    let mut hashes = BTreeMap::new();
    collect_file_hashes_inner(path, path, &mut hashes)?;
    Ok(hashes)
}

fn collect_file_hashes_inner(
    root: &Path,
    path: &Path,
    hashes: &mut BTreeMap<String, String>,
) -> std::io::Result<()> {
    if path
        .file_name()
        .is_some_and(|name| name == ".git" || name == ".aria" || name == "target")
    {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_file_hashes_inner(root, &path, hashes)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let content = std::fs::read(&path)?;
            hashes.insert(relative, compute_sha256(&content));
        }
    }
    Ok(())
}

fn diff_files(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Vec<String> {
    let all_paths = BTreeSet::from_iter(before.keys().chain(after.keys()).cloned());
    all_paths
        .into_iter()
        .filter(|path| before.get(path) != after.get(path))
        .collect()
}

#[allow(dead_code)]
fn _prompt_input_mode(_mode: PromptInputMode) {}

#[cfg(test)]
mod provider_stream_tests {
    use super::*;

    /// 追加语义只能在同一路径上验证：跨进程执行的 child_id 不同，落到不同文件，
    /// 因此集成测试无法区分 append 与 truncate。这里直接锁住写入模式。
    #[test]
    fn open_provider_stream_appends_instead_of_truncating() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("provider-1-stdout.log");

        {
            let mut file = open_provider_stream(&path).expect("first open");
            file.write_all(b"first run\n").expect("first write");
        }
        {
            let mut file = open_provider_stream(&path).expect("second open");
            file.write_all(b"second run\n").expect("second write");
        }

        let contents = std::fs::read_to_string(&path).expect("read stream log");
        assert!(
            contents.contains("first run"),
            "reopening must not truncate earlier content: {contents:?}"
        );
        assert!(contents.contains("second run"));
    }

    #[test]
    fn open_provider_stream_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested/deeper/provider-1-stdout.log");

        open_provider_stream(&path).expect("open must create parents");

        assert!(path.exists());
    }

    /// 未提供目录、或提供了非绝对路径（含空串）时都不得写流日志，更不得回退到
    /// provider 的工作目录或 Aria 进程的 cwd。
    #[test]
    fn provider_stream_path_rejects_missing_and_relative_dirs() {
        assert!(provider_stream_path(None, "claude", 1, "stdout").is_none());
        assert!(provider_stream_path(Some(Path::new("")), "claude", 1, "stdout").is_none());
        assert!(
            provider_stream_path(Some(Path::new("relative/logs")), "claude", 1, "stdout").is_none()
        );
    }

    #[test]
    fn provider_stream_path_sanitizes_provider_name_and_keeps_naming_shape() {
        let path = provider_stream_path(
            Some(Path::new("/tmp/aria-logs")),
            "/usr/bin/cl@ude",
            42,
            "stderr",
        )
        .expect("absolute dir yields a path");

        assert_eq!(path.parent(), Some(Path::new("/tmp/aria-logs")));
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("cl_ude-42-stderr.log")
        );
    }
}

/// A1-a(Bug45 批2 环境回退底座)绿证:同步 CLI 捕获通道的宿主继承语义。
#[cfg(all(test, unix))]
mod host_inheritance_tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::*;

    fn sh_spec(script: &str) -> CommandSpec {
        CommandSpec::new(
            "/bin/sh".to_string(),
            vec!["-c".to_string(), script.to_string()],
        )
    }

    /// 隔离父进程不构造 marker env(空 env_vars → None):真实子进程读到
    /// 宿主 PATH/HOME——子进程拿到的是宿主值,不是输入 map 快照。
    #[test]
    fn sync_capture_inherits_host_environment_without_explicit_map() {
        // overlay 是进程级共享状态:并行用例登记目录会让「子进程 PATH ==
        // 宿主 PATH」逐字断言失真 ⇒ 持锁清空后断言(计划 §6.1)。
        let _overlay_state = crate::cross_cutting::path_overlay::OverlayStateGuard::acquire();

        let host_home = std::env::var("HOME").expect("host HOME");
        let host_path = std::env::var("PATH").expect("host PATH");

        let output = run_command_capture_with_stream(
            &sh_spec("printf '%s\\n%s' \"$HOME\" \"$PATH\""),
            None,
            None,
            None,
            None,
            Some(Duration::from_secs(30)),
            None,
        )
        .expect("sync capture inherits host env");

        assert_eq!(output.exit_code, Some(0), "stderr: {}", output.stderr);
        assert_eq!(output.stdout, format!("{host_home}\n{host_path}"));
    }

    /// 显式 env map 是覆盖层而非快照:自定义 token 注入生效,map 外的
    /// 宿主键(HOME)仍从宿主继承到达子进程。
    #[test]
    fn sync_capture_env_map_overrides_host_without_replacing_it() {
        let host_home = std::env::var("HOME").expect("host HOME");
        let environment =
            BTreeMap::from([("ARIA_A1A_TOKEN".to_string(), "override-ok".to_string())]);

        let output = run_command_capture_with_stream(
            &sh_spec("printf '%s|%s' \"$ARIA_A1A_TOKEN\" \"$HOME\""),
            None,
            None,
            Some(&environment),
            None,
            Some(Duration::from_secs(30)),
            None,
        )
        .expect("sync capture applies env map as overrides");

        assert_eq!(output.exit_code, Some(0), "stderr: {}", output.stderr);
        assert_eq!(output.stdout, format!("override-ok|{host_home}"));
    }

    /// 计划 2026-11-13 包 2.4:同步捕获通道的 overlay 合并——
    /// `environment=None`(继承宿主 env)分支也必须显式下发合并后的
    /// PATH,不能只在 `Some(env)` 分支合并(首目录=overlay 目录)。
    #[test]
    fn sync_capture_merges_path_overlay_without_explicit_map() {
        let _overlay_state = crate::cross_cutting::path_overlay::OverlayStateGuard::acquire();

        let dir = tempfile::tempdir().expect("overlay dir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("create overlay bin dir");
        crate::cross_cutting::path_overlay::register(
            &bin_dir,
            crate::cross_cutting::path_overlay::OverlayPosition::Prepend,
        );

        let output = run_command_capture_with_stream(
            &sh_spec("printf '%s' \"$PATH\""),
            None,
            None,
            None,
            None,
            Some(Duration::from_secs(30)),
            None,
        )
        .expect("sync capture with overlay");

        assert_eq!(output.exit_code, Some(0), "stderr: {}", output.stderr);
        assert_eq!(
            std::env::split_paths(&output.stdout).next(),
            Some(bin_dir),
            "environment=None 分支的子进程 PATH 首目录必须是 overlay 目录: {}",
            output.stdout
        );
    }

    /// 计划 2026-11-13 §五 2.12：同步捕获通道补入登录 shell 探测回的
    /// agent 专属环境变量——`environment=None` 分支同样补；覆盖层已有值优先。
    #[test]
    fn sync_capture_injects_shell_probe_agent_env_without_explicit_map() {
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
        let script = format!("printf '%s' \"${free_var}\"");

        // 补入分支:environment=None、宿主缺该变量 ⇒ 子进程读到探测值。
        let output = run_command_capture_with_stream(
            &sh_spec(&script),
            None,
            None,
            None,
            None,
            Some(Duration::from_secs(30)),
            None,
        )
        .expect("sync capture with shell probe env");
        assert_eq!(output.exit_code, Some(0), "stderr: {}", output.stderr);
        assert_eq!(
            output.stdout, "/probe/agent-dir",
            "environment=None 分支也必须补入缺失的 agent 变量"
        );

        // 平台优先分支:显式覆盖层已有该变量 ⇒ 保留显式值。
        let environment =
            BTreeMap::from([(free_var.to_string(), "/explicit/agent-dir".to_string())]);
        let output = run_command_capture_with_stream(
            &sh_spec(&script),
            None,
            None,
            Some(&environment),
            None,
            Some(Duration::from_secs(30)),
            None,
        )
        .expect("sync capture keeps explicit env");
        assert_eq!(
            output.stdout, "/explicit/agent-dir",
            "覆盖层已有值不得被探测值覆盖"
        );
    }
}

/// PID 复用防护:超时/提前完成的组清理只在冻结的 (pid, starttime) 身份
/// 仍匹配时发射 `killpg`。身份不符只回收直接子进程——组内孙进程存活是
/// 「拒绝发射」的可观察证据(旧实现裸 killpg 会连带杀掉复用后的无关组)。
#[cfg(all(test, unix))]
mod pid_reuse_tests {
    use std::os::unix::process::CommandExt;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::*;

    fn spawn_group_holder(dir: &Path) -> (Child, PathBuf) {
        let grandchild_pid = dir.join("grandchild.pid");
        let mut command = Command::new("sh");
        command.args([
            "-c",
            &format!(
                "sleep 30 & printf '%s' \"$!\" > '{}'; wait",
                grandchild_pid.display()
            ),
        ]);
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        (command.spawn().expect("spawn group holder"), grandchild_pid)
    }

    fn read_pid(path: &Path) -> i32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(contents) = std::fs::read_to_string(path)
                && let Ok(pid) = contents.trim().parse::<i32>()
            {
                return pid;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("grandchild pid file never appeared: {}", path.display());
    }

    fn process_state(pid: i32) -> Option<char> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        stat.rsplit_once(") ")
            .and_then(|(_, tail)| tail.chars().next())
    }

    fn pid_alive(pid: i32) -> bool {
        matches!(process_state(pid), Some(state) if state != 'Z')
    }

    /// 有界轮询等待进程真实停止(僵尸/消失)。信号送达与 /proc 更新之间
    /// 有毫秒级窗口,不能直接断言。
    fn wait_until_stopped(pid: i32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if !pid_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn terminate_child_group_signals_group_when_identity_matches() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut child, grandchild_pid_file) = spawn_group_holder(dir.path());
        let child_pid = child.id();
        let identity = ProcessStartIdentity::freeze(child_pid).expect("freeze leader");
        let grandchild_pid = read_pid(&grandchild_pid_file);
        assert!(pid_alive(grandchild_pid), "孙进程必须在组内运行");

        terminate_child_group(&mut child, Some(identity));
        assert!(
            wait_until_stopped(grandchild_pid),
            "身份匹配时必须整组收口(孙进程不得存活)"
        );
        let _ = child.wait();
    }

    #[test]
    fn terminate_child_group_skips_group_signal_when_identity_is_stale() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut child, grandchild_pid_file) = spawn_group_holder(dir.path());
        let child_pid = child.id();
        let live = ProcessStartIdentity::freeze(child_pid).expect("freeze leader");
        let grandchild_pid = read_pid(&grandchild_pid_file);
        assert!(pid_alive(grandchild_pid), "孙进程必须在组内运行");

        // 「pgid 已被复用」等价状态:同一 pid 位置的 starttime 变了。
        let stale =
            ProcessStartIdentity::with_start_time_ticks(child_pid, live.start_time_ticks() + 1);
        terminate_child_group(&mut child, Some(stale));
        // 组信号若被误发,孙进程会在毫秒级内消失;观察窗口内持续存活
        // 即证明未发射组信号。
        for _ in 0..20 {
            assert!(
                pid_alive(grandchild_pid),
                "身份不符时不得 killpg(组内孙进程存活即未发组信号)"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.wait();
        unsafe {
            libc::kill(grandchild_pid, libc::SIGKILL);
        }
    }
}
