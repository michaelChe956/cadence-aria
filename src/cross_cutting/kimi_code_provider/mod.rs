use std::path::PathBuf;
use std::time::Duration;

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandRequest, TokioBoundedCommandRunner,
};
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::process_manager::ProcessManager;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderSession, ProviderStatus, StreamingProviderAdapter,
    StreamingProviderInput,
};
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

mod parse;
mod session;

#[cfg(test)]
pub mod tests;

#[allow(unused_imports)]
pub(crate) use session::run_kimi_session;

pub const KIMI_COMMAND: &str = "kimi";
const MIN_KIMI_VERSION: &str = "0.34.0";
const KIMI_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct KimiVersion(pub u64, pub u64, pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KimiVersionCommand {
    pub program: String,
    pub args: Vec<String>,
}

pub fn kimi_version_command() -> KimiVersionCommand {
    KimiVersionCommand {
        program: KIMI_COMMAND.to_string(),
        args: vec!["--version".to_string()],
    }
}

pub fn parse_kimi_version(output: &str) -> KimiVersion {
    output
        .split_whitespace()
        .find_map(|token| {
            let token = token.trim_start_matches('v');
            let mut parts = token.split('.');
            Some(KimiVersion(
                parts.next()?.parse().ok()?,
                parts.next()?.parse().ok()?,
                parts.next()?.parse().ok()?,
            ))
        })
        .unwrap_or(KimiVersion(0, 0, 0))
}

pub fn ensure_kimi_version_compatible(version: &KimiVersion) -> Result<(), ProviderAdapterError> {
    let minimum = parse_kimi_version(MIN_KIMI_VERSION);
    if version < &minimum {
        return Err(ProviderAdapterError::parse_error(
            format!(
                "Kimi version {}.{}.{} is incompatible; Kimi {MIN_KIMI_VERSION} or newer is required",
                version.0, version.1, version.2
            ),
            String::new(),
            String::new(),
        ));
    }
    Ok(())
}

pub async fn probe_kimi_version(command: &std::path::Path) -> KimiVersion {
    let request = BoundedCommandRequest {
        executable: command.to_string_lossy().into_owned(),
        argv: vec!["--version".to_string()],
        working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        timeout: KIMI_VERSION_PROBE_TIMEOUT,
        cancellation: CancellationToken::new(),
        environment: Default::default(),
        stdout_limit: 64 * 1024,
        stderr_limit: 64 * 1024,
    };
    match TokioBoundedCommandRunner.run_inherited(request).await {
        Ok(result) if result.exit_code == Some(0) => parse_kimi_version(&result.stdout),
        _ => KimiVersion(0, 0, 0),
    }
}

#[derive(Debug, Clone)]
pub struct KimiCodeProvider {
    command: PathBuf,
}

impl KimiCodeProvider {
    pub fn new(command: PathBuf) -> Self {
        Self { command }
    }

    pub(crate) fn build_args(&self) -> Vec<String> {
        vec!["acp".to_string()]
    }
}

pub(crate) fn format_kimi_exit_failure(
    details: String,
    status: Option<std::process::ExitStatus>,
) -> String {
    let suffix = match status.and_then(|status| status.code()) {
        Some(0) => "Kimi ACP process exited with code 0 before terminal prompt result",
        Some(1) => "Kimi ACP process exited with code 1 (non-retryable failure)",
        Some(75) => "Kimi ACP process exited with code 75 (temporary failure)",
        Some(code) => return format!("{details}; Kimi ACP process exited with code {code}"),
        None => return details,
    };
    format!("{details}; {suffix}")
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for KimiCodeProvider {
    fn supports_tool_calls(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let version = probe_kimi_version(&self.command).await;
        ensure_kimi_version_compatible(&version)?;
        let args = self.build_args();
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let process = ProcessManager::spawn(
            &self.command.to_string_lossy(),
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
        let (command_tx, command_rx) = mpsc::channel(8);
        let _ = event_tx
            .send(ProviderEvent::StatusChanged(ProviderStatus::Starting))
            .await;
        tokio::spawn(async move {
            let stderr_task = tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(target: "kimi_code_provider", "kimi stderr: {line}");
                }
            });
            let result = session::run_kimi_session(
                peer,
                command_rx,
                event_tx.clone(),
                input,
                cancel.clone(),
            )
            .await;
            let status = if result.is_err() {
                match tokio::time::timeout(std::time::Duration::from_millis(100), child.wait())
                    .await
                {
                    Ok(Ok(status)) => Some(status),
                    Ok(Err(wait_error)) => {
                        tracing::debug!(target: "kimi_code_provider", %wait_error, "failed to reap Kimi ACP process");
                        None
                    }
                    Err(_) => {
                        child.terminate().await;
                        None
                    }
                }
            } else {
                child.wait().await.ok()
            };
            let _ = stderr_task.await;
            if let Err(error) = result
                && error.details != session::KIMI_SESSION_ABORTED
            {
                let message = format_kimi_exit_failure(error.details, status);
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                    .await;
                let _ = event_tx.send(ProviderEvent::Failed { message }).await;
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}
