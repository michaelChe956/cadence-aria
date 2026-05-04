use crate::cross_cutting::adapter_compatibility::{
    AdapterCompatibilityEntry, CommandSpec, PromptInputMode, StructuredOutputMode,
};
use crate::cross_cutting::document_ops::compute_sha256;
use crate::cross_cutting::provider_adapter::{
    ProviderAdapter, ProviderAdapterError, STRUCTURED_OUTPUT_END, parse_last_structured_output,
};
use crate::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use std::collections::{BTreeMap, BTreeSet};
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

#[derive(Debug, Clone)]
pub struct CliAdapterConfig {
    pub compatibility: AdapterCompatibilityEntry,
    pub expected_artifact_kind: Option<String>,
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

        let raw = run_command_capture(
            &command,
            worktree_path,
            Some(&input.prompt),
            Some(Duration::from_secs(input.timeout)),
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
    let started = Instant::now();
    let mut command = Command::new(&command_spec.program);
    command.args(&command_spec.args);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
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

    let stdout_stream_path =
        provider_stream_path(current_dir, &command_spec.program, child.id(), "stdout");
    let stderr_stream_path =
        provider_stream_path(current_dir, &command_spec.program, child.id(), "stderr");
    let stdout_reader = child
        .stdout
        .take()
        .map(|reader| spawn_output_reader(reader, stdout_stream_path));
    let stderr_reader = child
        .stderr
        .take()
        .map(|reader| spawn_output_reader(reader, stderr_stream_path));

    if let Some(stdin_text) = stdin_text
        && let Some(mut stdin) = child.stdin.take()
    {
        stdin.write_all(stdin_text.as_bytes()).map_err(|error| {
            ProviderAdapterError::permission_denied(
                format!("write provider stdin: {error}"),
                String::new(),
                String::new(),
            )
        })?;
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
                    terminate_child_group(&mut child);
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
                terminate_child_group(&mut child);
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

struct OutputReader {
    buffer: Arc<Mutex<Vec<u8>>>,
    structured_output_complete: Arc<AtomicBool>,
    handle: JoinHandle<std::io::Result<()>>,
}

fn spawn_output_reader<R>(mut reader: R, stream_path: Option<PathBuf>) -> OutputReader
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

fn provider_stream_path(
    current_dir: Option<&Path>,
    program: &str,
    child_id: u32,
    stream_name: &str,
) -> Option<PathBuf> {
    let current_dir = current_dir?;
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
    Some(
        current_dir
            .join(".aria/runtime/provider-streams")
            .join(format!("{provider_name}-{child_id}-{stream_name}.log")),
    )
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

fn terminate_child_group(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        let _ = libc::killpg(child.id() as i32, libc::SIGKILL);
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
