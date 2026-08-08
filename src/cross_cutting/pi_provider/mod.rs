use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

use sha2::{Digest, Sha256};

use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandRequest, TokioBoundedCommandRunner,
};
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::process_manager::ProcessManager;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderExecutionEvent, ProviderExecutionEventKind,
    ProviderExecutionEventStatus, ProviderPermissionMode, ProviderSession, ProviderStatus,
    StreamingProviderAdapter, StreamingProviderInput,
};

mod parse;
mod session;

#[cfg(test)]
pub mod tests;

pub(crate) use parse::*;

pub const PI_COMMAND: &str = "pi";
const ARIA_ASK_EXTENSION: &str = include_str!("aria-ask.ts");
const MIN_PI_VERSION: &str = "0.83.0";
const PI_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProbeFailure {
    CommandFailed,
    TimedOut,
    Unparseable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PiVersion {
    Known((u32, u32, u32)),
    Unknown(ProbeFailure),
}

fn ask_extension_error(message: impl std::fmt::Display) -> ProviderAdapterError {
    ProviderAdapterError::parse_error(
        format!("failed to prepare Pi ask extension: {message}"),
        String::new(),
        String::new(),
    )
}

fn ensure_private_cache_directory(cache: &Path) -> Result<(), ProviderAdapterError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(false);
    #[cfg(unix)]
    builder.mode(0o700);
    match builder.create(cache) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            std::fs::create_dir_all(
                cache
                    .parent()
                    .ok_or_else(|| ask_extension_error("cache path has no parent"))?,
            )
            .map_err(|error| ask_extension_error(format!("create cache parent: {error}")))?;
            builder
                .create(cache)
                .map_err(|error| ask_extension_error(format!("create cache directory: {error}")))?;
        }
        Err(error) => {
            return Err(ask_extension_error(format!(
                "create cache directory: {error}"
            )));
        }
    }
    let metadata = std::fs::symlink_metadata(cache)
        .map_err(|error| ask_extension_error(format!("inspect cache directory: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ask_extension_error("cache path is not a directory"));
    }
    #[cfg(unix)]
    std::fs::set_permissions(cache, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| ask_extension_error(format!("restrict cache directory: {error}")))?;
    Ok(())
}

fn validate_ask_extension(path: &Path) -> Result<bool, ProviderAdapterError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(ask_extension_error(format!("inspect extension: {error}"))),
    };
    if metadata.file_type().is_symlink() {
        return Err(ask_extension_error("extension path must not be a symlink"));
    }
    if !metadata.is_file() {
        return Err(ask_extension_error("extension path is not a regular file"));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| ask_extension_error(format!("read extension: {error}")))?;
    if content != ARIA_ASK_EXTENSION {
        return Err(ask_extension_error(
            "existing extension content does not match Aria's extension",
        ));
    }
    Ok(true)
}

fn ensure_ask_extension_in(cache: &Path) -> Result<PathBuf, ProviderAdapterError> {
    ensure_private_cache_directory(cache)?;
    let hash = hex::encode(Sha256::digest(ARIA_ASK_EXTENSION.as_bytes()));
    let path = cache.join(format!("aria-ask-{}.ts", &hash[..8]));
    if validate_ask_extension(&path)? {
        return Ok(path);
    }

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    match options.open(&path) {
        Ok(mut file) => {
            file.write_all(ARIA_ASK_EXTENSION.as_bytes())
                .map_err(|error| ask_extension_error(format!("write extension: {error}")))?;
            file.sync_all()
                .map_err(|error| ask_extension_error(format!("sync extension: {error}")))?;
            Ok(path)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            validate_ask_extension(&path)?;
            Ok(path)
        }
        Err(error) => Err(ask_extension_error(format!("create extension: {error}"))),
    }
}

fn ensure_ask_extension() -> Result<PathBuf, ProviderAdapterError> {
    let home = std::env::var_os("HOME").ok_or_else(|| ask_extension_error("HOME is not set"))?;
    ensure_ask_extension_in(&PathBuf::from(home).join(".cache").join("cadence-aria"))
}

fn parse_pi_version(output: &str) -> PiVersion {
    output
        .split_whitespace()
        .find_map(|token| {
            let token = token.trim_start_matches('v');
            let mut parts = token.split('.');
            Some((
                parts.next()?.parse().ok()?,
                parts.next()?.parse().ok()?,
                parts.next()?.parse().ok()?,
            ))
        })
        .map(PiVersion::Known)
        .unwrap_or(PiVersion::Unknown(ProbeFailure::Unparseable))
}

async fn probe_pi_version(command: &Path) -> PiVersion {
    probe_pi_version_with_timeout(command, PI_VERSION_PROBE_TIMEOUT).await
}

async fn probe_pi_version_with_timeout(command: &Path, timeout: Duration) -> PiVersion {
    let request = BoundedCommandRequest {
        executable: command.to_string_lossy().into_owned(),
        argv: vec!["--version".to_string()],
        working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        timeout,
        cancellation: CancellationToken::new(),
        environment: BTreeMap::new(),
        stdout_limit: 64 * 1024,
        stderr_limit: 64 * 1024,
    };
    match TokioBoundedCommandRunner.run_inherited(request).await {
        Ok(result) if result.timed_out => PiVersion::Unknown(ProbeFailure::TimedOut),
        Ok(result) if result.exit_code == Some(0) => parse_pi_version(&result.stdout),
        Ok(_) | Err(_) => PiVersion::Unknown(ProbeFailure::CommandFailed),
    }
}

fn ensure_pi_version_compatible(version: &PiVersion) -> Result<(), ProviderAdapterError> {
    let PiVersion::Known(version) = version else {
        return Ok(());
    };
    let minimum = match parse_pi_version(MIN_PI_VERSION) {
        PiVersion::Known(minimum) => minimum,
        PiVersion::Unknown(_) => unreachable!("minimum Pi version must be valid"),
    };
    if version < &minimum {
        return Err(ProviderAdapterError::parse_error(
            format!(
                "Pi version {}.{}.{} is incompatible; Pi {MIN_PI_VERSION} or newer is required",
                version.0, version.1, version.2
            ),
            String::new(),
            String::new(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct PiProvider {
    command: PathBuf,
}

impl PiProvider {
    pub fn new(command: PathBuf) -> Self {
        Self { command }
    }

    /// Constructs Pi's Auto-only RPC command line.
    /// - `-e <path>` loads Aria's structured ask extension.
    /// - No `--session-dir`: Pi uses its default `~/.pi` directory.
    /// - No `--no-extensions`: Pi preserves user-global extensions.
    /// - The project repository is passed as the process cwd at spawn time.
    pub(crate) fn build_args(
        &self,
        resume_session_id: Option<&str>,
        extension_path: &Path,
    ) -> Vec<String> {
        let mut args = vec![
            "--mode".to_string(),
            "rpc".to_string(),
            "-e".to_string(),
            extension_path.display().to_string(),
        ];
        if let Some(session_id) = resume_session_id.map(str::trim).filter(|id| !id.is_empty()) {
            args.push("--session-id".to_string());
            args.push(session_id.to_string());
        }
        args
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PiProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let version = probe_pi_version(&self.command).await;
        ensure_pi_version_compatible(&version)?;
        let extension_path = ensure_ask_extension()?;
        let args = self.build_args(input.resume_provider_session_id.as_deref(), &extension_path);
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let command = self.command.to_string_lossy().to_string();
        let process = ProcessManager::spawn(
            &command,
            &arg_refs,
            &input.working_dir,
            &input.env_vars,
            cancel.clone(),
        )
        .await?;

        let peer = JsonRpcPeer::new(process.stdout, process.stdin);
        let stderr = process.stderr;
        let mut child = process.child;
        let (event_tx, event_rx) = mpsc::channel(32);
        // Pi is Auto-only. Keep bridge construction consistent with the other streaming
        // providers, but never use it for authorization or command forwarding.
        let bridge = ApprovalBridge::new(ProviderPermissionMode::Auto, event_tx.clone());
        // The bridge owns a different command channel, so Abort must use this one.
        let (command_tx, command_rx) = mpsc::channel(8);
        let _ = event_tx
            .send(ProviderEvent::StatusChanged(ProviderStatus::Starting))
            .await;
        let _ = event_tx
            .send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "provider".to_string(),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Started,
                title: "Pi provider started".to_string(),
                detail: None,
                command: None,
                cwd: Some(input.working_dir.display().to_string()),
                output: None,
                exit_code: None,
            }))
            .await;

        tokio::spawn(async move {
            let stderr_output = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
            let stderr_output_for_task = std::sync::Arc::clone(&stderr_output);
            let stderr_task = tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut output = stderr_output_for_task.lock().await;
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&line);
                }
            });

            let result =
                session::run_pi_session(peer, command_rx, event_tx.clone(), input, cancel).await;
            drop(bridge);
            if result.is_err() {
                let _ = child.start_kill();
            }
            let status = child.wait().await;
            let _ = stderr_task.await;
            if let Err(error) = result {
                let stderr = stderr_output.lock().await.trim().to_string();
                let status_text = match status {
                    Ok(status) => format!("exit status: {status}"),
                    Err(wait_error) => format!("failed to wait for process: {wait_error}"),
                };
                let message = if stderr.is_empty() {
                    format!("{} ({status_text})", error.details)
                } else {
                    format!("{} ({status_text}); stderr: {stderr}", error.details)
                };
                // run_pi_session already emitted the terminal Failed event; emitting an
                // additional execution event here would violate fail-fast terminality.
                tracing::debug!(%message, "Pi provider session ended with failure");
            }
        });

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}
