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
    StreamingProviderAdapter, StreamingProviderInput, canonical_tool_policy,
    validate_tool_policy_for_role,
};
use crate::cross_cutting::tool_policy_audit::{
    DurableToolPolicyEvent, LcProjectionAudit, ProviderStartAudit,
};
use crate::product::logical_codebase::provider_projection::ProviderPolicyProjector as _;

mod parse;
mod projection;
mod response;
pub(crate) mod session;
mod support;

#[cfg(test)]
pub mod tests;

pub(crate) use parse::*;
pub use projection::CodexPolicyProjector;
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

impl CodexProvider {
    /// `start_validated` 的真实执行体(gateway 路由级 Codex 阻断迁移到 LC
    /// projection 判定归 Task 3/8;此前真实 gateway validate 无法为 Codex
    /// 产出 validated policy,tests 子模块直接驱动本私有方法锁定受限分支)。
    ///
    /// 顺序契约(REQ-LCG-04):角色守卫 → adapter 匹配 → 危险门(零 child)
    /// → audit sink → exact version → 受限投影 → 以 canonical root 为进程
    /// cwd spawn → LC 握手(协议 params 来自投影)→ provider_start 统一审计
    /// (`lc_projection` 必填)→ 会话循环。
    #[cfg(test)]
    async fn start_lc_validated(
        &self,
        input: StreamingProviderInput,
        envelope: &crate::product::logical_codebase::policy::SessionPolicyEnvelope,
        capability_snapshot_ref: &str,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // 既有测试驱动的 Normal 相位入口(Task 7 保持签名零变化)。
        self.start_lc_validated_with_phase(input, envelope, capability_snapshot_ref, false, cancel)
            .await
    }

    /// `start_validated` 的相位感知执行体(Task 7):`root_recipe_phase`
    /// 来自 validated policy 的冻结相位,供共用 LC guard 消费 marker 语义。
    async fn start_lc_validated_with_phase(
        &self,
        mut input: StreamingProviderInput,
        envelope: &crate::product::logical_codebase::policy::SessionPolicyEnvelope,
        capability_snapshot_ref: &str,
        root_recipe_phase: bool,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let lc_error = |details: String| {
            ProviderAdapterError::parse_error(details, String::new(), String::new())
        };

        // 1) 共用 LC run-context guard(Task 7):projection/input role 一致 +
        //    root recipe marker 相位语义 + 双向守卫,早于版本/session child
        //    (与 direct `start` 的纯双向守卫同源)。
        crate::cross_cutting::streaming_provider::guard_lc_validated_launch(
            root_recipe_phase,
            &input.role,
            &input.role,
            input.tool_policy.as_ref(),
        )
        .map_err(|error| lc_error(format!("codex lc validated start: {error}")))?;

        // 2) adapter 匹配:validated input 必须是 Codex app-server(不匹配即拒,
        //    不回退其它 provider/dialect)。
        if input.provider_type != crate::protocol::contracts::ProviderType::Codex
            || envelope.provider_dialect
                != crate::product::logical_codebase::policy::ProviderDialect::CodexCliV1
        {
            return Err(lc_error(
                "codex lc validated start: only Codex app-server launches are accepted by this adapter"
                    .to_string(),
            ));
        }

        // 3) REQ-LCG-04 危险门(在任何 spawn 之前,零 child):Coder 形态
        //    (Coding action + 无通用 tool policy)若受限写面未冻结,启动后
        //    direct 映射只能是 danger-full-access——永久拒绝,details 即
        //    稳定码(与 gateway 路由门同源字节)。
        if let Some(reason) = projection::lc_danger_refusal(envelope, input.tool_policy.as_ref()) {
            return Err(ProviderAdapterError::parse_error(
                reason,
                String::new(),
                String::new(),
            ));
        }

        // 4) 统一 launch audit sink:LC 会话必须绑定 run-bound sink,缺失
        //    fail-closed(1b 的 prepare 契约之前由本路径强制)。
        let sink = input.audit_sink.clone().ok_or_else(|| {
            lc_error("codex lc validated start: audit sink is required for LC launches".to_string())
        })?;

        // 5) exact version(supplier seam 优先,默认真实 `--version` 探测+进程内
        //    缓存;不可得 fail-closed)。全 LC 路径必填,非仅策略角色。Coder
        //    形态(direct 映射=danger-full-access)的预认证失败同以危险
        //    稳定码拒绝(Task 5b):版本未知即受限投影不可认证,唯一替代是
        //    永久拒绝的 danger-full-access。
        let version_failure =
            |error: crate::cross_cutting::streaming_provider::VersionProbeError| {
                if projection::lc_coder_danger_shape(envelope, input.tool_policy.as_ref()) {
                    tracing::warn!(
                        target: "codex_provider",
                        %error,
                        "codex lc coder launch uncertifiable: exact version unknown"
                    );
                    ProviderAdapterError::parse_error(
                    crate::product::logical_codebase::provider_gateway::CODEX_DANGER_FULL_ACCESS_UNSUPPORTED,
                    String::new(),
                    String::new(),
                )
                } else {
                    lc_error(format!("codex lc validated start: {error}"))
                }
            };
        let provider_version = match self.version_supplier.clone() {
            Some(supplier) => supplier().map_err(version_failure)?,
            None => crate::cross_cutting::streaming_provider::cached_cli_version(
                &self.command,
                probe_codex_version(&self.command, CODEX_VERSION_PROBE_TIMEOUT),
            )
            .await
            .map_err(version_failure)?,
        };

        // 6) 受限投影(REQ-LCG-04):envelope 派生不可伪造 boundary plan;
        //    trust/MCP bundle digest 的 gateway 侧装配归 1b/1c(空串同样纳入
        //    session digest,装配后任一漂移都会改变 projection_digest)。
        //    approval 由投影从 action×permission 派生,gateway 不预冻结。
        let boundary = projection::lc_boundary_plan(envelope)
            .map_err(|error| lc_error(format!("codex lc validated start: {error}")))?;
        let projection_input =
            crate::product::logical_codebase::provider_projection::ProviderProjectionInput::new(
                envelope.clone(),
                crate::product::logical_codebase::provider_gateway::ProviderRef::codex(
                    capability_snapshot_ref,
                ),
                envelope.action,
                input.role.clone(),
                input.permission_mode.clone(),
                input.tool_policy.clone(),
                String::new(),
                String::new(),
                envelope.config_artifact_ref.clone(),
                String::new(),
                Some(boundary),
            );
        let projector = CodexPolicyProjector::new(provider_version.clone());
        let lc_projection = projector
            .project(&projection_input)
            .map_err(|error| lc_error(format!("codex lc validated start: {error}")))?;
        let sandbox = projector.sandbox_projection(&lc_projection);

        // 派生一致性防线:投影冻结的 target_root 必须等于 envelope 冻结
        // target(不一致即内部错误,fail-closed,不静默采纳)。
        if sandbox.target_root() != envelope.target.worktree.as_path() {
            return Err(lc_error(
                "codex lc validated start: projection target root disagrees with the frozen envelope target"
                    .to_string(),
            ));
        }

        // 7) spawn:进程 cwd=投影冻结的 canonical LC root(双 cwd 合同,协议
        //    target 只进 wire params,不进进程 cwd)。
        let args = self.build_args();
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let command = self.command.to_string_lossy().to_string();
        let process_cwd = sandbox.process_cwd().to_path_buf();
        let process = ProcessManager::spawn(
            &command,
            &arg_refs,
            &process_cwd,
            &input.env_vars,
            cancel.clone(),
        )
        .await?;

        // GC7:与 direct 路径同源的出站 request id typed namespace。
        let peer = JsonRpcPeer::new(process.stdout, process.stdin)
            .with_outbound_id_namespace(OutboundIdNamespace::Aria);
        let stderr = process.stderr;
        let mut child = process.child;

        // 8) LC 握手:thread/start|thread/resume 共用受限投影 params(协议
        //    cwd=投影冻结面,不以 raw 输入覆盖);失败终止子进程 fail-closed;
        //    LC 路径必得非空 thread id(与 direct 策略路径同源兜底)。
        let handshake = match session::codex_lc_session_handshake(&peer, &input, &sandbox).await {
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
            return Err(lc_error(
                "codex lc validated start: thread/start response missing or blank thread id"
                    .to_string(),
            ));
        };

        // 9) provider_start 统一审计:sandbox/approvalPolicy 与 wire 同源;
        //    lc_projection 必填(LC 会话不因 tool_policy=None 跳过统一 audit);
        //    boundary 引用同样从 sandbox 投影单一来源取值。
        let tool_policy_digest =
            projection::lc_tool_policy_canonical_digest(input.tool_policy.as_ref())
                .map_err(|error| lc_error(format!("codex lc validated start: {error}")))?;
        let audit_event = DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
            provider: session::TOOL_POLICY_PROVIDER_NAME.to_string(),
            role: crate::cross_cutting::streaming_provider::adapter_role_text(&input.role)
                .to_string(),
            workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
            provider_session_id: thread_id.clone(),
            tool_policy_canonical_digest: tool_policy_digest,
            argv: args.clone(),
            sandbox: Some(sandbox.mode().wire_text().to_string()),
            approval_policy: Some(sandbox.approval_policy().to_string()),
            provider_version,
            adapter_dialect: session::CODEX_POLICY_DIALECT.to_string(),
            lc_projection: Some(LcProjectionAudit {
                action: projection::action_text(envelope.action).to_string(),
                wire_dialect: projection::wire_dialect_text(lc_projection.wire_dialect())
                    .to_string(),
                capability_projection_digest: lc_projection
                    .capability_projection_digest()
                    .to_string(),
                projection_digest: lc_projection.projection_digest().to_string(),
                boundary_evidence_ref: sandbox.boundary_evidence_ref().to_string(),
            }),
        });
        if let Err(error) = sink.append_bound(audit_event) {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(lc_error(format!(
                "codex lc validated start: provider_start audit append failed: {error}"
            )));
        }

        // 10) 事件通道 + 会话循环(与 direct 策略路径同构):会话事件的 cwd
        //     报告进程 cwd=canonical root,raw working_dir 不泄漏;审批行为
        //     沿既有循环(策略会话 exec/fileChange 即时拒绝,Coder 沿
        //     ApprovalBridge 映射 Auto/Supervised)。
        input.working_dir = process_cwd.clone();
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
                cwd: Some(process_cwd.display().to_string()),
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

            let result = session::run_codex_session_loop(
                peer,
                bridge,
                event_tx.clone(),
                input,
                cancel.clone(),
                handshake,
            )
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
            native_session_id: Some(thread_id),
            events: event_rx,
            commands,
        })
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
        // GC9：resume 记录缺失与 drift 同路径处置（清除 resume id、全新会话）；
        // 「标记 superseded」仅带内 ToolPolicyWarning（🔴 无旧文件可写，不伪造
        // durable 文件），在事件通道建立后送出。
        let mut superseded_record_missing = false;
        let mut policy_context: Option<(
            std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>,
            String,
            String,
        )> = None;
        // Task 1.2（BOOT-04）：策略会话机制只服务 deny 意图；BootstrapExecutor
        // marker 已在上方守卫完成结构复核，不进入策略握手/审计（自举执行器
        // 的 durable 审计由 root recipe receipt 承担）。
        if let Some(
            policy @ crate::cross_cutting::streaming_provider::ProviderToolPolicy {
                intent:
                    crate::cross_cutting::streaming_provider::ToolPolicyIntent::DenyFileWriteBuiltins,
            },
        ) = input.tool_policy.as_ref()
        {
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
                    workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
                    provider_session_id: resume_id.clone(),
                    tool_policy_canonical_digest: canonical.digest.clone(),
                    provider_version: provider_version.clone(),
                    adapter_dialect: session::CODEX_POLICY_DIALECT.to_string(),
                    ..ProviderStartAudit::default()
                };
                if let Some(stored) = stored.as_ref()
                    && matches!(
                        crate::cross_cutting::tool_policy_audit::resume_with_audit_record(
                            Some(stored.record.clone()),
                            &current
                        ),
                        crate::cross_cutting::tool_policy_audit::ResumeDecision::RejectSupersedeAndStartNew
                    )
                {
                    // P1-4 裁决：superseded 终止审计写入被取代旧 run 的文件（其
                    // provider_start 已是首行；被终止的是旧会话），新 run 照常从
                    // provider_start 开始。
                    crate::cross_cutting::tool_policy_audit::append_superseded_policy_drift(
                        sink.as_ref(),
                        stored,
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
                } else if stored.is_none() {
                    // GC9：记录缺失与 drift 同路径处置——清除 resume id、以全新会话
                    // （thread/start + 新 run 的 provider_start）启动；「标记 superseded」
                    // 仅带内 ToolPolicyWarning：无旧文件可写，🔴 不得伪造无
                    // provider_start 首行的 durable 文件（首行不变量优先）。
                    input.resume_provider_session_id = None;
                    superseded_record_missing = true;
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
                    "codex policy session: thread/start response missing or blank thread id",
                    String::new(),
                    String::new(),
                ));
            };
            let launch_params = session::codex_launch_params(&input);
            let audit_event = DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                provider: session::TOOL_POLICY_PROVIDER_NAME.to_string(),
                role: crate::cross_cutting::streaming_provider::adapter_role_text(&input.role)
                    .to_string(),
                workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
                provider_session_id: thread_id.clone(),
                tool_policy_canonical_digest: tool_policy_digest,
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
                adapter_dialect: session::CODEX_POLICY_DIALECT.to_string(),
                // direct 策略会话:无 LC 投影(仅 LC validated 启动落盘)。
                lc_projection: None,
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

        // GC9：resume 记录缺失的带内 superseded 标记（事件通道在握手/append 之后
        // 建立，故在此送出；新会话 provider_start 已先落盘）。
        if superseded_record_missing {
            let _ = event_tx
                .send(crate::cross_cutting::streaming_provider::superseded_policy_record_missing_warning())
                .await;
        }

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
    /// LC validated 启动(Task 5,REQ-LCG-04):只接受 gateway 产出的
    /// validated input,经 `CodexPolicyProjector` 投影受限 sandbox
    /// (read-only/workspace-write,danger-full-access 永久拒绝),以 canonical
    /// LC root 为进程 cwd、投影 protocol cwd 为协议 cwd 启动;统一落盘
    /// `ProviderStartAudit.lc_projection`(sandbox/approvalPolicy 与 wire
    /// 同源;无通用 tool policy 的角色不早退)。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, launch) = validated.into_parts();
        self.start_lc_validated_with_phase(
            input,
            launch.envelope(),
            launch.capability_snapshot_ref(),
            launch.is_root_recipe_phase(),
            cancel,
        )
        .await
    }
}
