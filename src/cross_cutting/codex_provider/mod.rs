use std::path::PathBuf;
use std::time::Duration;

use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::process_manager::ProcessManager;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderExecutionEvent, ProviderExecutionEventKind,
    ProviderExecutionEventStatus, ProviderSession, ProviderStatus, StreamingProviderAdapter,
    StreamingProviderInput,
};

mod parse;
mod response;
mod session;
mod support;

#[cfg(test)]
pub mod tests;

pub(crate) use parse::*;
pub(crate) use response::*;
pub(crate) use support::*;

pub(crate) const CODEX_RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const CODEX_DEFAULT_SANDBOX_MODE: &str = "danger-full-access";
pub(crate) const CODEX_RESUME_STALL_ERROR: &str = "Codex resume stalled before provider progress";
#[cfg(not(test))]
pub(crate) const CODEX_RESUME_STALL_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
pub(crate) const CODEX_RESUME_STALL_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct CodexProvider {
    command: PathBuf,
}

impl CodexProvider {
    pub fn new(command: PathBuf) -> Self {
        Self { command }
    }

    fn build_args(&self) -> Vec<String> {
        vec![
            "app-server".to_string(),
            "--enable".to_string(),
            "default_mode_request_user_input".to_string(),
        ]
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CodexProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let args = self.build_args();
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
        let bridge = ApprovalBridge::new(input.permission_mode.clone(), event_tx.clone());
        let commands = bridge.command_sender();
        let _ = event_tx
            .send(ProviderEvent::StatusChanged(ProviderStatus::Starting))
            .await;
        let _ = event_tx
            .send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "provider".to_string(),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Started,
                title: "Codex provider started".to_string(),
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
                session::run_codex_session(peer, bridge, event_tx.clone(), input, cancel.clone())
                    .await;
            if result.is_err() {
                let _ = child.start_kill();
            }
            let status = child.wait().await;
            let _ = stderr_task.await;
            if let Err(error) = result {
                let stderr =
                    support::combine_stderr(stderr_output.lock().await.clone(), error.stderr);
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "provider".to_string(),
                        kind: ProviderExecutionEventKind::Provider,
                        status: ProviderExecutionEventStatus::Failed,
                        title: "Codex provider failed".to_string(),
                        detail: Some(error.details.clone()),
                        command: None,
                        cwd: None,
                        output: if stderr.trim().is_empty() {
                            None
                        } else {
                            Some(stderr.clone())
                        },
                        exit_code: None,
                    }))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Failed {
                        message: support::format_codex_failure(error.details, status, stderr),
                    })
                    .await;
            }
        });

        Ok(ProviderSession {
            events: event_rx,
            commands,
        })
    }
}
