use crate::cross_cutting::adapter_compatibility::{
    AdapterCompatibilityEntry, CommandSpec, PromptInputMode, StructuredOutputMode,
};
use crate::cross_cutting::document_ops::compute_sha256;
use crate::cross_cutting::provider_adapter::{
    ProviderAdapter, ProviderAdapterError, parse_last_structured_output,
};
use crate::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
        if let Some(worktree_path) = &input.worktree_path {
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

    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ProviderAdapterError::command_missing(format!("{}: {error}", command_spec.program))
        } else {
            ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
        }
    })?;

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

    if let Some(timeout) = timeout {
        loop {
            if child
                .try_wait()
                .map_err(|error| {
                    ProviderAdapterError::execution_failed(
                        None,
                        String::new(),
                        error.to_string(),
                        0,
                    )
                })?
                .is_some()
            {
                break;
            }
            if started.elapsed() >= timeout {
                let _ = child.kill();
                let output = child.wait_with_output().map_err(|error| {
                    ProviderAdapterError::execution_failed(
                        None,
                        String::new(),
                        error.to_string(),
                        0,
                    )
                })?;
                return Err(ProviderAdapterError::timeout(
                    String::from_utf8_lossy(&output.stdout).to_string(),
                    String::from_utf8_lossy(&output.stderr).to_string(),
                    started.elapsed().as_millis() as u64,
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    let output = child.wait_with_output().map_err(|error| {
        ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
    })?;
    Ok(CapturedCommandOutput {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        duration_ms: started.elapsed().as_millis() as u64,
        timeout_status: TimeoutStatus::NotTimedOut,
    })
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
        .is_some_and(|name| name == ".git" || name == "target")
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
