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
    StreamingProviderInput, adapter_role_text,
};
use crate::cross_cutting::tool_policy_audit::{LcProjectionAudit, ProviderStartAudit};
use crate::product::logical_codebase::policy::ProviderDialect;
use crate::product::logical_codebase::provider_gateway::ProviderRef;
use crate::product::logical_codebase::provider_projection::{
    ProviderPolicyProjector, ProviderProjectionInput,
};
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

mod client_services;
pub mod mcp_bundle;
mod parse;
mod projection;
mod session;

pub use projection::KimiPolicyProjector;

use mcp_bundle::KimiMcpInjection;
pub use mcp_bundle::{McpBundleError, McpServerConfig, ValidatedMcpServerBundle};

pub use mcp_bundle::codegraph_server_config;

#[cfg(test)]
pub mod tests;

#[allow(unused_imports)]
pub(crate) use session::run_kimi_session;

pub const KIMI_COMMAND: &str = "kimi";
/// kimi 在 tool-policy canonical 序列中的 provider 名(LC 统一 launch audit
/// `ProviderStartAudit.provider` 与 4a/4b 的 claude-code/pi 同形)。
pub const TOOL_POLICY_PROVIDER_NAME: &str = "kimi-code";
/// kimi LC 会话的 adapter dialect(wire dialect 序列化值同形 `kimi-acp`)。
pub const KIMI_POLICY_DIALECT: &str = "kimi-acp";
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
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    /// LC validated 启动(Task 4c):只接受 gateway 产出的 validated input。
    /// Kimi 通用 tool 为 `None`——非空通用策略在 version/child 之前拒绝
    /// (稳定码 `provider_generic_tool_policy_forbidden`);`None` 不早退,
    /// 仍执行 exact version probe、native session/handshake 与统一
    /// `ProviderStartAudit.lc_projection` 落盘。进程 cwd 与 ACP 协议 cwd
    /// 都保持 canonical LC root;target 写面由宿主 fs/terminal handler
    /// 消费不可伪造 boundary plan(不进协议 cwd)。`mcp_bundle_digest`
    /// 只对应 Aria 注入,无注入时标记 native 项目配置来源。direct
    /// `start` 保持逐字节不变。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (mut input, launch) = validated.into_parts();
        let envelope = launch.envelope().clone();

        // 1) 拒绝非空通用策略(version/child 之前;Global Constraints:
        //    Kimi 拒绝非空通用策略,继续既有 ClientServicePolicy)。
        if input.tool_policy.is_some() {
            return Err(ProviderAdapterError::parse_error(
                format!(
                    "kimi lc validated start rejected: {}: kimi carries no generic tool policy",
                    projection::KIMI_GENERIC_TOOL_POLICY_FORBIDDEN
                ),
                String::new(),
                String::new(),
            ));
        }

        // 2) adapter 匹配:validated input 必须是 Kimi ACP(不匹配即拒,
        //    不回退其它 provider/dialect)。
        if input.provider_type != crate::protocol::contracts::ProviderType::KimiCode
            || envelope.provider_dialect != ProviderDialect::KimiAcpV1
        {
            return Err(ProviderAdapterError::parse_error(
                "kimi lc validated start: only Kimi ACP launches are accepted by this adapter",
                String::new(),
                String::new(),
            ));
        }

        // 3) 统一 launch audit sink:LC 会话必须绑定 run-bound sink,缺失
        //    fail-closed(1b 的 prepare 契约之前由本路径强制)。
        let sink = input.audit_sink.clone().ok_or_else(|| {
            ProviderAdapterError::parse_error(
                "kimi lc validated start: audit sink is required for LC launches",
                String::new(),
                String::new(),
            )
        })?;

        // 4) exact version probe(真实 `--version` 探测 + 兼容门;不可得
        //    fail-closed)。全 LC 路径必填,不以 `tool_policy=None` 早退。
        let version = probe_kimi_version(&self.command).await;
        ensure_kimi_version_compatible(&version)?;
        let provider_version = format!("kimi {}.{}.{}", version.0, version.1, version.2);

        // 5) 不可伪造 boundary plan + LC 投影。`mcp_bundle_digest` 只对应
        //    Aria 注入 bundle;无注入时标记 native 项目配置来源(单独标
        //    来源,不冒充 Aria bundle digest)。trust digest 的 gateway 侧
        //    装配(ProviderTrustSource)归 1b/1c;空串同样纳入 session
        //    digest,装配后任一漂移都会改变 projection_digest。
        eprintln!("LC4C-DBG: version probed ok: {provider_version}");
        let boundary = projection::lc_boundary_plan(&envelope).map_err(|error| {
            ProviderAdapterError::parse_error(
                format!("kimi lc validated start: {error}"),
                String::new(),
                String::new(),
            )
        })?;
        let mcp_bundle_digest = match self.mcp_injection.as_ref() {
            Some(config) => config.bundle.digest().to_string(),
            None => projection::KIMI_NATIVE_MCP_SOURCE.to_string(),
        };
        let projection_input = ProviderProjectionInput::new(
            envelope.clone(),
            ProviderRef::kimi_code(launch.capability_snapshot_ref()),
            envelope.action,
            input.role.clone(),
            input.permission_mode.clone(),
            None,
            projection::KIMI_LC_APPROVAL_POLICY.to_string(),
            mcp_bundle_digest,
            envelope.config_artifact_ref.clone(),
            String::new(),
            Some(boundary.clone()),
        );
        let projector = KimiPolicyProjector::new(provider_version.clone());
        let lc_projection = projector.project(&projection_input).map_err(|error| {
            ProviderAdapterError::parse_error(
                format!("kimi lc validated start: {error}"),
                String::new(),
                String::new(),
            )
        })?;
        let tool_policy_digest = projection::lc_tool_policy_canonical_digest(
            input.tool_policy.as_ref(),
        )
        .map_err(|error| {
            ProviderAdapterError::parse_error(
                format!("kimi lc validated start: {error}"),
                String::new(),
                String::new(),
            )
        })?;

        // 6) argv 与 direct 同源(冻结 `acp`,无策略物理片段);进程 cwd =
        //    envelope 冻结的 canonical LC root;ACP 协议 cwd 同样保持
        //    root(Task 4 Interfaces:ACP cwd 保持 root,target 写面经
        //    plan 由宿主 handler 消费,不进协议 cwd)。
        let args = self.build_args();
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let command = self.command.to_string_lossy().to_string();
        let process_cwd = envelope.working_directory.clone();
        let process = ProcessManager::spawn(
            &command,
            &arg_refs,
            &process_cwd,
            &input.env_vars,
            cancel.clone(),
        )
        .await?;
        input.working_dir = envelope.working_directory.clone();

        let peer = JsonRpcPeer::new(process.stdout, process.stdin);
        let stderr = process.stderr;
        let mut child = process.child;
        let mcp_injection = self.mcp_injection.clone();
        let (event_tx, event_rx) = mpsc::channel(32);
        let (command_tx, command_rx) = mpsc::channel(8);
        let _ = event_tx
            .send(ProviderEvent::StatusChanged(ProviderStatus::Starting))
            .await;

        // 7) 统一 launch audit 模板:除原生会话 id 外全部在 spawn 前冻结;
        //    会话任务在 native session/handshake 完成后回填 id 落盘。
        let audit_template = ProviderStartAudit {
            provider: TOOL_POLICY_PROVIDER_NAME.to_string(),
            role: adapter_role_text(&input.role).to_string(),
            workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
            provider_session_id: String::new(),
            tool_policy_canonical_digest: tool_policy_digest,
            argv: args.clone(),
            sandbox: None,
            approval_policy: None,
            provider_version,
            adapter_dialect: KIMI_POLICY_DIALECT.to_string(),
            lc_projection: Some(LcProjectionAudit {
                action: projection::action_text(envelope.action).to_string(),
                wire_dialect: projection::wire_dialect_text(lc_projection.wire_dialect())
                    .to_string(),
                capability_projection_digest: lc_projection
                    .capability_projection_digest()
                    .to_string(),
                projection_digest: lc_projection.projection_digest().to_string(),
                boundary_evidence_ref: lc_projection.boundary_evidence_ref().to_string(),
            }),
        };
        let (native_id_tx, native_id_rx) = tokio::sync::oneshot::channel();
        let lc_session = session::LcValidatedSession {
            boundary,
            audit_sink: sink,
            audit_template,
            native_id_tx,
        };

        // 握手等待上界先于会话任务的 move 计算(input.timeout_secs);
        // 会话任务持有独立的 cancel 克隆,start 侧继续等待/响应当前 token。
        let handshake_bound = Duration::from_secs(u64::from(input.timeout_secs.max(1)));
        let session_cancel = cancel.clone();

        eprintln!("LC4C-DBG: child spawned, handing to session task");
        // 8) 会话任务(与 direct `start` 同构:stderr 收集 + MCP 注入决策
        //    + 失败 kill 链);差异:携带 LC boundary plan 与统一 audit
        //    落盘上下文。
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
            let result = session::run_kimi_session_validated(
                peer,
                command_rx,
                event_tx.clone(),
                input,
                mcp_injection,
                lc_session,
                session_cancel.clone(),
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

        // 9) 有界等待 native session/handshake 结果(握手 id 落盘成功后
        //    回传;失败/超时/取消即 fail-closed,子进程由会话任务 kill 链
        //    回收)。
        let native_session_id = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(ProviderAdapterError::execution_failed(
                    None,
                    String::new(),
                    "kimi lc validated start cancelled before the native handshake completed",
                    0,
                ));
            }
            reported = tokio::time::timeout(handshake_bound, native_id_rx) => match reported {
                Err(_elapsed) => {
                    return Err(ProviderAdapterError::timeout_with_details(
                        "kimi lc validated start timed out waiting for the native session handshake",
                        String::new(),
                        String::new(),
                        handshake_bound.as_millis() as u64,
                    ));
                }
                // 通道在回传前关闭(会话任务先行结束/panic)。
                Ok(Err(_channel_closed)) => {
                    return Err(ProviderAdapterError::execution_failed(
                        None,
                        String::new(),
                        "kimi lc validated start ended before the native handshake reported",
                        0,
                    ));
                }
                // 回传的握手结果:id 或握手/audit 落盘失败。
                Ok(Ok(handshake)) => match handshake {
                    Ok(id) => id,
                    Err(error) => return Err(error),
                },
            }
        };

        Ok(ProviderSession {
            native_session_id: Some(native_session_id),
            events: event_rx,
            commands: command_tx,
        })
    }
}
