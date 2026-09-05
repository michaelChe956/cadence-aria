use std::path::PathBuf;
use std::time::Duration;

use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::json_rpc_peer::{JsonRpcPeer, OutboundIdNamespace};
use crate::cross_cutting::process_manager::ProcessManager;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderExecutionEvent, ProviderExecutionEventKind,
    ProviderExecutionEventStatus, ProviderSession, ProviderStatus, ProviderVersionSupplier,
    StreamingProviderAdapter, StreamingProviderInput, UsageReportData, canonical_tool_policy,
    validate_tool_policy_for_role,
};
use crate::cross_cutting::tool_policy_audit::{DurableToolPolicyEvent, ProviderStartAudit};

mod parse;
mod response;
pub(crate) mod session;
mod support;

#[cfg(test)]
pub mod tests;

pub(crate) use parse::*;
pub(crate) use response::*;
pub(crate) use support::*;

pub(crate) const CODEX_RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Codex app-server 默认 sandbox 模式。当前唯一支持的是 `danger-full-access`,
/// 这是受限写尚未就绪的已知 gap。gateway 据此在路由级阻断 Codex 启动(见
/// `provider_gateway::CODEX_DANGER_FULL_ACCESS_SANDBOX_MODE`),直到受限写
/// sandbox 配置可用。
pub const CODEX_DEFAULT_SANDBOX_MODE: &str = "danger-full-access";
pub(crate) const CODEX_RESUME_STALL_ERROR: &str = "Codex resume stalled before provider progress";

pub(crate) fn is_resume_stall_failure(message: &str) -> bool {
    message.contains(CODEX_RESUME_STALL_ERROR)
}

#[cfg(not(test))]
pub(crate) const CODEX_RESUME_STALL_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
pub(crate) const CODEX_RESUME_STALL_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Clone)]
pub struct CodexProvider {
    command: PathBuf,
    /// 策略会话 provider 版本 supplier（测试 seam；3.3 接线真实 CLI 探测后保留）。
    version_supplier: Option<ProviderVersionSupplier>,
}

impl std::fmt::Debug for CodexProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexProvider")
            .field("command", &self.command)
            .field(
                "version_supplier",
                &self
                    .version_supplier
                    .as_ref()
                    .map(|_| "<provider-version-supplier>"),
            )
            .finish()
    }
}

/// codex CLI `--version` 有界探测超时（GC9）。
pub const CODEX_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// codex CLI `--version` 有界探测（Task 3.3）：成功非空返回精确字符串；
/// 空输出/命令失败 → `Unavailable`；超时 → `Timeout`。
pub async fn probe_codex_version(
    command: &std::path::Path,
    timeout: Duration,
) -> Result<String, crate::cross_cutting::streaming_provider::VersionProbeError> {
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandRequest, TokioBoundedCommandRunner,
    };
    use crate::cross_cutting::streaming_provider::VersionProbeError;
    use std::collections::BTreeMap;
    use tokio_util::sync::CancellationToken;

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
        Ok(result) if result.timed_out => Err(VersionProbeError::Timeout),
        Ok(result) if result.exit_code == Some(0) => {
            let version = result.stdout.trim();
            if version.is_empty() {
                Err(VersionProbeError::Unavailable)
            } else {
                Ok(version.to_string())
            }
        }
        Ok(_) | Err(_) => Err(VersionProbeError::Unavailable),
    }
}

impl CodexProvider {
    pub fn new(command: PathBuf) -> Self {
        Self {
            command,
            version_supplier: None,
        }
    }

    /// 注入策略会话 provider 版本 supplier（fixture/测试 seam）。
    pub fn with_version_supplier(mut self, supplier: ProviderVersionSupplier) -> Self {
        self.version_supplier = Some(supplier);
        self
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
    async fn start(
        &self,
        mut input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // 双向 spawn 前守卫（Task 3.1）：非法角色×策略组合在创建子进程之前拒绝
        //（位于 ProcessManager::spawn 之前；策略会话的握手/审计在 Task 3.2 同样
        // 位于本守卫之后、spawn 之前/之后按契约分层）。
        validate_tool_policy_for_role(&input.role, input.tool_policy.as_ref()).map_err(
            |error| {
                ProviderAdapterError::parse_error(error.to_string(), String::new(), String::new())
            },
        )?;
        // 策略上下文（Task 3.2/3.3，spawn 前）：版本解析（supplier seam 优先，默认
        // 真实 `--version` 探测+进程内缓存，不可得 fail-closed）与 resume 冻结三元组
        // 比对（记录缺失或 digest/version/dialect 任一不一致 → 追加 superseded 终止
        // 审计并新建会话，丢弃 resume id）。
        let mut policy_context: Option<(
            std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>,
            String,
            String,
        )> = None;
        if let Some(policy) = input.tool_policy.as_ref() {
            let sink = input.audit_sink.clone().ok_or_else(|| {
                ProviderAdapterError::parse_error(
                    "codex policy session: audit sink is required for policy sessions",
                    String::new(),
                    String::new(),
                )
            })?;
            let provider_version = match self.version_supplier.clone() {
                Some(supplier) => supplier().map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("codex policy session: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?,
                None => crate::cross_cutting::streaming_provider::cached_cli_version(
                    &self.command,
                    probe_codex_version(&self.command, CODEX_VERSION_PROBE_TIMEOUT),
                )
                .await
                .map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("codex policy session: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?,
            };
            let canonical = canonical_tool_policy(session::TOOL_POLICY_PROVIDER_NAME, policy)
                .map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("codex policy session: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?;
            let resume_id = input
                .resume_provider_session_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string);
            if let Some(resume_id) = resume_id.as_ref() {
                let stored = sink.find_provider_start(resume_id).map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("codex policy session: resume lookup failed: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?;
                let current = ProviderStartAudit {
                    tool_policy_digest: canonical.digest.clone(),
                    provider_version: provider_version.clone(),
                    dialect: session::CODEX_POLICY_DIALECT.to_string(),
                    ..ProviderStartAudit::default()
                };
                if matches!(
                    crate::cross_cutting::tool_policy_audit::resume_with_audit_record(
                        stored, &current
                    ),
                    crate::cross_cutting::tool_policy_audit::ResumeDecision::RejectSupersedeAndStartNew
                ) {
                    sink.append_bound(
                        crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::SessionTerminated(
                            crate::cross_cutting::tool_policy_audit::SessionTerminatedAudit {
                                reason_code: "superseded_policy_drift".to_string(),
                            },
                        ),
                    )
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!(
                                "codex policy session: superseded audit append failed: {error}"
                            ),
                            String::new(),
                            String::new(),
                        )
                    })?;
                    input.resume_provider_session_id = None;
                }
            }
            policy_context = Some((sink, provider_version, canonical.digest));
        }
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

        // GC7：codex peer 出站 request id 使用 typed namespace `aria-<seq>`，
        // 与 server→client 入站原生数字 id 值域隔离；pi/kimi peer 保持默认 Numeric。
        let peer = JsonRpcPeer::new(process.stdout, process.stdin)
            .with_outbound_id_namespace(OutboundIdNamespace::Aria);
        let stderr = process.stderr;
        let mut child = process.child;
        // 策略会话（Task 3.2）：握手（initialize/initialized + thread/start|resume）
        // 从后台任务前置到 start 内有界完成；成功后、返回前写 `provider_start`，
        // 握手/写失败终止子进程并 fail-closed。非策略/Coder 路径零变化。
        let mut native_session_id = None;
        let policy_handshake = if let Some((sink, provider_version, tool_policy_digest)) =
            policy_context
        {
            let handshake = match session::codex_session_handshake(&peer, &input).await {
                Ok(handshake) => handshake,
                Err(error) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(error);
                }
            };
            let Some(thread_id) = handshake.thread_id.clone() else {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(ProviderAdapterError::parse_error(
                    "codex policy session: thread/start response missing thread id",
                    String::new(),
                    String::new(),
                ));
            };
            let launch_params = session::codex_launch_params(&input);
            let audit_event = DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                provider: session::TOOL_POLICY_PROVIDER_NAME.to_string(),
                role: UsageReportData::role_text(&input.role).to_string(),
                tool_policy_digest,
                argv: args.clone(),
                sandbox: launch_params
                    .get("sandbox")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string),
                approval_policy: launch_params
                    .get("approvalPolicy")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string),
                provider_version,
                dialect: session::CODEX_POLICY_DIALECT.to_string(),
                native_session_id: thread_id.clone(),
            });
            if let Err(error) = sink.append_bound(audit_event) {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(ProviderAdapterError::parse_error(
                    format!("codex policy session: provider_start audit append failed: {error}"),
                    String::new(),
                    String::new(),
                ));
            }
            native_session_id = Some(thread_id);
            Some(handshake)
        } else {
            None
        };
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

            let result = match policy_handshake {
                Some(handshake) => {
                    session::run_codex_session_loop(
                        peer,
                        bridge,
                        event_tx.clone(),
                        input,
                        cancel.clone(),
                        handshake,
                    )
                    .await
                }
                None => {
                    session::run_codex_session(
                        peer,
                        bridge,
                        event_tx.clone(),
                        input,
                        cancel.clone(),
                    )
                    .await
                }
            };
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
            native_session_id,
            events: event_rx,
            commands,
        })
    }
}
