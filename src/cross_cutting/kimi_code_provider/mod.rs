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

mod client_services;
pub mod mcp_bundle;
mod parse;
mod session;

use mcp_bundle::KimiMcpInjection;
pub use mcp_bundle::{McpBundleError, McpServerConfig, ValidatedMcpServerBundle};

pub use mcp_bundle::codegraph_server_config;

#[cfg(test)]
pub mod tests;

#[allow(unused_imports)]
pub(crate) use session::run_kimi_session;

pub const KIMI_COMMAND: &str = "kimi";
pub const MIN_KIMI_VERSION: &str = "0.34.0";
const KIMI_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const KIMI_LOGIN_GUIDANCE: &str = "Kimi Code is not logged in; run `kimi login` and retry.";
const KIMI_SENSITIVE_MARKERS: [&str; 4] = ["token", "authorization", "api_key", "config"];

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

/// kimi MCP 受控注入的 provider 级状态：受控 bundle +（resume 场景的）冻结 digest。
/// 数据来源是 Aria-owned 的 session policy envelope（`LogicalCodebaseProviderGateway`
/// 侧组装，见 tasks.md 6.1 与 session-policy-envelope REQ-ENV-06）；未提供时保持
/// `mcpServers: []`，绝不从普通 provider 配置透传任意 JSON。
#[derive(Debug, Clone)]
pub struct KimiMcpInjectionConfig {
    bundle: ValidatedMcpServerBundle,
    frozen_digest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct KimiCodeProvider {
    command: PathBuf,
    mcp_injection: Option<KimiMcpInjectionConfig>,
}

impl KimiCodeProvider {
    pub fn new(command: PathBuf) -> Self {
        Self {
            command,
            mcp_injection: None,
        }
    }

    /// 附加受控 MCP bundle（新会话场景：无冻结 digest，resume 校验不适用）。
    pub fn with_mcp_bundle(mut self, bundle: ValidatedMcpServerBundle) -> Self {
        self.mcp_injection = Some(KimiMcpInjectionConfig {
            bundle,
            frozen_digest: None,
        });
        self
    }

    /// 附加受控 MCP bundle 与上次 run 审计冻结的 digest（resume 场景：
    /// digest 漂移时拒绝 `session/load`、启动新会话并标记旧会话 superseded）。
    pub fn with_mcp_bundle_for_resume(
        mut self,
        bundle: ValidatedMcpServerBundle,
        frozen_digest: String,
    ) -> Self {
        self.mcp_injection = Some(KimiMcpInjectionConfig {
            bundle,
            frozen_digest: Some(frozen_digest),
        });
        self
    }

    pub(crate) fn build_args(&self) -> Vec<String> {
        vec!["acp".to_string()]
    }
}

pub(crate) fn format_kimi_exit_failure(
    details: String,
    status: Option<std::process::ExitStatus>,
) -> String {
    if kimi_authentication_failure(&details) {
        return KIMI_LOGIN_GUIDANCE.to_string();
    }
    let suffix = match status.and_then(|status| status.code()) {
        Some(0) => "Kimi ACP process exited with code 0 before terminal prompt result",
        Some(1) => "Kimi ACP process exited with code 1 (non-retryable failure)",
        Some(75) => "Kimi ACP process exited with code 75 (temporary failure)",
        Some(code) => {
            return sanitize_kimi_failure(format!(
                "{details}; Kimi ACP process exited with code {code}"
            ));
        }
        None => return sanitize_kimi_failure(details),
    };
    sanitize_kimi_failure(format!("{details}; {suffix}"))
}

fn kimi_authentication_failure(details: &str) -> bool {
    let normalized = details.to_ascii_lowercase();
    normalized.contains("unauthorized")
        || normalized.contains("not logged in")
        || normalized.contains("authentication")
        || normalized.contains("\"code\":401")
        || normalized.contains("code 401")
}

fn sanitize_kimi_failure(message: String) -> String {
    message
        .lines()
        .filter(|line| {
            let normalized = line.to_ascii_lowercase();
            !KIMI_SENSITIVE_MARKERS
                .iter()
                .any(|marker| normalized.contains(marker))
        })
        .collect::<Vec<_>>()
        .join("\n")
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
        let mcp_injection = self.mcp_injection.clone();
        let (event_tx, event_rx) = mpsc::channel(32);
        let (command_tx, command_rx) = mpsc::channel(8);
        let _ = event_tx
            .send(ProviderEvent::StatusChanged(ProviderStatus::Starting))
            .await;
        tokio::spawn(async move {
            let stderr_task = tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                let mut output = String::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&line);
                    tracing::debug!(
                        target: "kimi_code_provider",
                        "kimi stderr: {}",
                        sanitize_kimi_failure(line)
                    );
                }
                output
            });
            let resuming = input
                .resume_provider_session_id
                .as_deref()
                .map(str::trim)
                .is_some_and(|id| !id.is_empty());
            let mcp_injection =
                mcp_injection.map(|config| match (resuming, config.frozen_digest) {
                    (true, Some(frozen_digest)) => {
                        KimiMcpInjection::for_resume(config.bundle, frozen_digest)
                    }
                    _ => KimiMcpInjection::for_new_session(config.bundle),
                });
            let result = session::run_kimi_session_with_mcp(
                peer,
                command_rx,
                event_tx.clone(),
                input,
                mcp_injection,
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
            let stderr_output = stderr_task.await.unwrap_or_default();
            if let Err(error) = result
                && error.details != session::KIMI_SESSION_ABORTED
            {
                let message =
                    format_kimi_exit_failure(format!("{}\n{stderr_output}", error.details), status);
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
