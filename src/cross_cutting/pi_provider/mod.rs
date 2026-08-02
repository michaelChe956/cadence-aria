use std::path::PathBuf;

use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
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

#[derive(Debug, Clone)]
pub struct PiProvider {
    command: PathBuf,
}

impl PiProvider {
    pub fn new(command: PathBuf) -> Self {
        Self { command }
    }

    /// Constructs Pi's Auto-only RPC command line.
    /// - No `-e`: Pi receives no Aria authorization extension.
    /// - No `--session-dir`: Pi uses its default `~/.pi` directory.
    /// - No `--no-extensions`: Pi preserves user-global extensions.
    /// - The project repository is passed as the process cwd at spawn time.
    pub(crate) fn build_args(&self, resume_session_id: Option<&str>) -> Vec<String> {
        let mut args = vec!["--mode".to_string(), "rpc".to_string()];
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
        let args = self.build_args(input.resume_provider_session_id.as_deref());
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
