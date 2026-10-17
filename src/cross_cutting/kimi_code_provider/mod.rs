use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandRequest, TokioBoundedCommandRunner,
};
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::process_manager::{
    PROVIDER_CHILD_EXIT_STREAM_DRAIN, ProcessManager, SessionTailExit, bounded_task_join,
    provider_child_exit_error, run_session_tail_with_exit_watch,
};
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    LcVersionCache, LcVersionObservation, LcVersionProbe, LcVersionProbeKey, ProviderEvent,
    ProviderSession, ProviderStatus, StreamingProviderAdapter, StreamingProviderInput,
    VersionProbeError, adapter_role_text,
};
use crate::cross_cutting::tool_policy_audit::{LcProjectionAudit, ProviderStartAudit};
use crate::product::logical_codebase::policy::ProviderDialect;
use crate::product::logical_codebase::provider_gateway::ProviderRef;
use crate::product::logical_codebase::provider_projection::{
    ProviderLcPrepareFacts, ProviderPolicyProjector, ProviderProjectionInput,
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

/// `KimiVersion` 的 audit/facts 统一文本形态(与既有 `start_validated`
/// 的 `provider_version` 字面量同形)。
fn kimi_version_text(version: KimiVersion) -> String {
    format!("kimi {}.{}.{}", version.0, version.1, version.2)
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
        // 批2 Task 3 修复轮(F2):探测环境显式携带宿主 PATH(平台观察
        // 语义,provider_health::probe_provider 同款)——裸名 CLI 不因
        // env_clear 后缺 PATH 而恒 Unavailable。
        environment: crate::cross_cutting::provider_environment::platform_observation_environment(),
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

/// Task 3.6(B0e):LC 只读 facts 的 supplier——除既有 command/MCP 注入外,
/// 携带 LC 版本观测的 60 秒 TTL 缓存与 supplier 实例范围;direct
/// `start` 不消费本状态,语义原样。
#[derive(Clone)]
pub struct KimiCodeProvider {
    command: PathBuf,
    mcp_injection: Option<KimiMcpInjectionConfig>,
    /// LC 版本只读观测的 60 秒 TTL 缓存(prepare 与 spawn 前复验同 key
    /// 命中;direct `start` 的无过期 `probe_kimi_version` 语义原样,
    /// 两路严格分离)。
    lc_version_cache: LcVersionCache,
    /// 版本观测的 supplier 实例范围:每个 provider 实例唯一,不同实例
    /// 的观测互不串值(`LcVersionProbeKey::supplier_scope`)。
    supplier_scope: u64,
}

impl std::fmt::Debug for KimiCodeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // LcVersionCache 无 Debug(观测缓存非投影语义面),手动实现
        // 保持既有 Debug 输出的两个字段。
        f.debug_struct("KimiCodeProvider")
            .field("command", &self.command)
            .field("mcp_injection", &self.mcp_injection)
            .finish_non_exhaustive()
    }
}

impl KimiCodeProvider {
    pub fn new(command: PathBuf) -> Self {
        Self {
            command,
            mcp_injection: None,
            lc_version_cache: LcVersionCache::new(),
            supplier_scope: next_kimi_supplier_scope(),
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

/// supplier 实例范围分配:进程内单调递增,每实例唯一。
fn next_kimi_supplier_scope() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl KimiCodeProvider {
    /// LC 版本观测 key:provider + canonical executable + 只读可执行身份
    /// (len/mtime;身份变化即换 key 再观测,不失效其它 key)+ `--version`
    /// argv + supplier 实例范围(3.2 合同同形)。
    fn lc_version_probe_key(&self) -> LcVersionProbeKey {
        LcVersionProbeKey {
            provider: crate::product::models::ProviderName::KimiCode,
            canonical_executable: self
                .command
                .canonicalize()
                .unwrap_or_else(|_| self.command.clone()),
            executable_identity: self.lc_executable_identity(),
            argv: vec!["--version".to_string()],
            supplier_scope: self.supplier_scope,
        }
    }

    /// 只读可执行身份(文件 len+mtime;不可得时固定哨兵,不写不探测)。
    fn lc_executable_identity(&self) -> String {
        match std::fs::metadata(&self.command) {
            Ok(metadata) => {
                let mtime = metadata
                    .modified()
                    .ok()
                    .and_then(|moment| moment.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| format!("{}.{}", duration.as_secs(), duration.subsec_nanos()))
                    .unwrap_or_else(|| "unknown".to_string());
                format!("len={},mtime={}", metadata.len(), mtime)
            }
            Err(_) => "metadata-unavailable".to_string(),
        }
    }

    /// LC 版本观测 probe 闭包:有界 `--version` → 解析为 `kimi X.Y.Z`
    /// 文本(兼容门在观测后统一施加);失败/超时 typed
    /// `VersionProbeError`(不缓存、不回退过期成功,合同归
    /// `LcVersionCache`)。
    fn lc_version_probe(&self) -> LcVersionProbe {
        let executable = self.command.to_string_lossy().into_owned();
        std::sync::Arc::new(move |_key| {
            let executable = executable.clone();
            Box::pin(async move {
                let request = BoundedCommandRequest {
                    executable,
                    argv: vec!["--version".to_string()],
                    working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                    timeout: KIMI_VERSION_PROBE_TIMEOUT,
                    cancellation: CancellationToken::new(),
                    // 批2 Task 3 修复轮(F2):探测环境显式携带宿主 PATH(平台观察
                    // 语义,provider_health::probe_provider 同款)——裸名 CLI 不因
                    // env_clear 后缺 PATH 而恒 Unavailable。
                    environment:
                        crate::cross_cutting::provider_environment::platform_observation_environment(
                        ),
                    stdout_limit: 64 * 1024,
                    stderr_limit: 64 * 1024,
                };
                match TokioBoundedCommandRunner.run_inherited(request).await {
                    Ok(result) if result.exit_code == Some(0) => {
                        Ok(kimi_version_text(parse_kimi_version(&result.stdout)))
                    }
                    Ok(result) if result.timed_out => Err(VersionProbeError::Timeout),
                    Ok(_) => Err(VersionProbeError::Unavailable),
                    Err(_) => Err(VersionProbeError::Unavailable),
                }
            })
        })
    }

    /// LC exact version 只读观测(TTL 缓存 + 兼容门;不可得 fail-closed)。
    /// prepare 与 spawn 前复验共用同一 key/缓存,命中不伪称每次实测
    /// 二进制;可执行身份变化换 key 再观测。
    async fn observe_lc_version(
        &self,
        cancel: CancellationToken,
    ) -> Result<(LcVersionProbeKey, LcVersionObservation), ProviderAdapterError> {
        let key = self.lc_version_probe_key();
        let probe = self.lc_version_probe();
        let observation = self
            .lc_version_cache
            .cached_lc_cli_version(key.clone(), probe, cancel)
            .await
            .map_err(|error| {
                ProviderAdapterError::execution_failed(
                    None,
                    String::new(),
                    format!("kimi lc exact version observation failed: {error}"),
                    0,
                )
            })?;
        let version = parse_kimi_version(observation.exact_version());
        ensure_kimi_version_compatible(&version)?;
        Ok((key, observation))
    }

    /// LC MCP 来源事实:Aria 注入 bundle digest 或 native 项目配置标记
    /// (单独标来源,不冒充 Aria bundle digest)。
    fn lc_mcp_bundle_digest(&self) -> String {
        match self.mcp_injection.as_ref() {
            Some(config) => config.bundle.digest().to_string(),
            None => projection::KIMI_NATIVE_MCP_SOURCE.to_string(),
        }
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
            // r19 续修(fix 轮 2):exit-watch 竞速——子进程先退出且排空耗尽时
            // 泵被截断,合成错误沿既有 Err 路径发终态 Failed(非 KIMI_SESSION_
            // ABORTED 语义,不会被吞)。
            let result = match run_session_tail_with_exit_watch(
                session::run_kimi_session_with_mcp(
                    peer,
                    command_rx,
                    event_tx.clone(),
                    input,
                    mcp_injection,
                    cancel.clone(),
                ),
                &mut child,
                PROVIDER_CHILD_EXIT_STREAM_DRAIN,
                provider_child_exit_error,
            )
            .await
            {
                SessionTailExit::Stream(result) => result,
                SessionTailExit::ChildExitBeforeTerminal { error } => Err(error),
            };
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
                match tokio::time::timeout(PROVIDER_CHILD_EXIT_STREAM_DRAIN, child.wait()).await {
                    Ok(Ok(status)) => Some(status),
                    _ => None,
                }
            };
            let stderr_output = bounded_task_join(stderr_task, PROVIDER_CHILD_EXIT_STREAM_DRAIN)
                .await
                .unwrap_or_default();
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
        //    Kimi 拒绝非空通用策略,继续既有 ClientServicePolicy)。批2
        //    Task 20 唯一例外:`ReviewerVerify` 窗口合同——kimi 侧由
        //    冻结执行合同的受信岗位授予等价消费(窗口引用进 grant,不
        //    造通用策略物理面),不在此拒绝。
        if input.tool_policy.is_some()
            && !matches!(
                input.tool_policy.as_ref().map(|policy| &policy.intent),
                Some(
                    crate::cross_cutting::streaming_provider::ToolPolicyIntent::ReviewerVerify {
                        ..
                    }
                )
            )
        {
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

        // 4) exact version:LC 只读 TTL 观测(与 prepare 同 key 命中,不
        //    伪称每次实测;不可得 fail-closed)。全 LC 路径必填,不以
        //    `tool_policy=None` 早退;direct `start` 的无过期
        //    `probe_kimi_version` 语义原样(两路严格分离)。
        let (_probe_key, observation) = self.observe_lc_version(cancel.clone()).await?;
        let provider_version = observation.exact_version().to_string();

        // 5) 不可伪造 boundary plan + LC 投影。prepared 候选路径(B0e):
        //    validate 携带冻结 full projection 时消费该投影——spawn 前
        //    同 facts 复验(version/MCP 来源/boundary 引用与冻结值一致,
        //    不一致 typed 拒于 child 之前),audit 字面量与 gateway 冻结
        //    的 full projection(含 root trust digest)同源,不建第二套
        //    投影矩阵;裸 validate 路径(既有测试/旧链)保持本 adapter
        //    自建投影(trust 空)。`mcp_bundle_digest` 只对应 Aria 注入
        //    bundle;无注入时标记 native 项目配置来源(单独标来源,不
        //    冒充 Aria bundle digest)。装配后任一漂移都会改变
        //    projection_digest。
        let boundary = projection::lc_boundary_plan(&envelope).map_err(|error| {
            ProviderAdapterError::parse_error(
                format!("kimi lc validated start: {error}"),
                String::new(),
                String::new(),
            )
        })?;
        let mcp_bundle_digest = self.lc_mcp_bundle_digest();
        let lc_projection = match launch.lc_projection() {
            Some(frozen) => {
                if frozen.exact_version() != provider_version {
                    return Err(ProviderAdapterError::parse_error(
                        format!(
                            "kimi lc validated start rejected: provider version drifted before spawn (frozen={}, observed={}) (session NOT started; no child was spawned)",
                            frozen.exact_version(),
                            provider_version
                        ),
                        String::new(),
                        String::new(),
                    ));
                }
                if frozen.mcp_bundle_digest() != mcp_bundle_digest {
                    return Err(ProviderAdapterError::parse_error(
                        format!(
                            "kimi lc validated start rejected: mcp source drifted before spawn (frozen={}, current={}) (session NOT started; no child was spawned)",
                            frozen.mcp_bundle_digest(),
                            mcp_bundle_digest
                        ),
                        String::new(),
                        String::new(),
                    ));
                }
                if frozen.boundary_evidence_ref()
                    != projection::boundary_evidence_ref_for(&boundary)
                {
                    return Err(ProviderAdapterError::parse_error(
                        "kimi lc validated start rejected: boundary evidence drifted from the frozen full projection (session NOT started; no child was spawned)",
                        String::new(),
                        String::new(),
                    ));
                }
                frozen.clone()
            }
            None => {
                let projection_input = ProviderProjectionInput::new(
                    envelope.clone(),
                    ProviderRef::kimi_code(launch.capability_snapshot_ref()),
                    envelope.action,
                    input.role.clone(),
                    input.permission_mode.clone(),
                    None,
                    projection::KIMI_LC_APPROVAL_POLICY.to_string(),
                    mcp_bundle_digest.clone(),
                    envelope.config_artifact_ref.clone(),
                    String::new(),
                    Some(boundary.clone()),
                );
                KimiPolicyProjector::new(provider_version.clone())
                    .project(&projection_input)
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!("kimi lc validated start: {error}"),
                            String::new(),
                            String::new(),
                        )
                    })?
            }
        };
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
            // 批2 Task 20：冻结执行合同的受信岗位授予（duty/承载/窗口
            // 引用）——ReviewerVerify 会话据此获得窗口引用的 FsWrite 判定。
            trusted_duty: launch
                .frozen_execution()
                .map(crate::cross_cutting::kimi_code_provider::client_services::TrustedDutyGrant::from_frozen_execution),
            native_id_tx,
        };

        // 握手等待上界先于会话任务的 move 计算(input.timeout_secs);
        // 会话任务持有独立的 cancel 克隆,start 侧继续等待/响应当前 token。
        let handshake_bound = Duration::from_secs(input.timeout_secs.max(1));
        let session_cancel = cancel.clone();

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
            // r19 续修(fix 轮 2):同 direct 路径——exit-watch 竞速 + 终态
            // join 有界(红线 a)。
            let result = match run_session_tail_with_exit_watch(
                session::run_kimi_session_validated(
                    peer,
                    command_rx,
                    event_tx.clone(),
                    input,
                    mcp_injection,
                    lc_session,
                    session_cancel.clone(),
                ),
                &mut child,
                PROVIDER_CHILD_EXIT_STREAM_DRAIN,
                provider_child_exit_error,
            )
            .await
            {
                SessionTailExit::Stream(result) => result,
                SessionTailExit::ChildExitBeforeTerminal { error } => Err(error),
            };
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
                match tokio::time::timeout(PROVIDER_CHILD_EXIT_STREAM_DRAIN, child.wait()).await {
                    Ok(Ok(status)) => Some(status),
                    _ => None,
                }
            };
            let stderr_output = bounded_task_join(stderr_task, PROVIDER_CHILD_EXIT_STREAM_DRAIN)
                .await
                .unwrap_or_default();
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

    /// Task 3.6(B0e):LC 只读 prepare facts——把 `start_validated` 的
    /// version/boundary/MCP 来源提取为只读观测(零 spawn 会话 child):
    /// exact version 经 [`LcVersionCache`] 60 秒 TTL(与 spawn 前复验
    /// 同 key)、approval/MCP/boundary 为 provider 特定 facts、projector
    /// 与该 version 同源。guard(Kimi 通用策略例外/adapter 匹配)先于
    /// version probe 与 child。
    async fn prepare_lc_facts(
        &self,
        input: &StreamingProviderInput,
        launch: &crate::product::logical_codebase::provider_gateway::ValidatedSessionLaunchPolicy,
        cancel: CancellationToken,
    ) -> Result<ProviderLcPrepareFacts, ProviderAdapterError> {
        // 1) Kimi 通用策略例外(与 `start_validated` 同源的第二道门;
        //    guard 早于 probe/child)。
        if input.tool_policy.is_some() {
            return Err(ProviderAdapterError::parse_error(
                format!(
                    "kimi lc prepare facts rejected: {}: kimi carries no generic tool policy",
                    projection::KIMI_GENERIC_TOOL_POLICY_FORBIDDEN
                ),
                String::new(),
                String::new(),
            ));
        }
        // 2) adapter 匹配(不匹配即拒,不回退其它 provider/dialect)。
        let envelope = launch.envelope();
        if input.provider_type != crate::protocol::contracts::ProviderType::KimiCode
            || envelope.provider_dialect != ProviderDialect::KimiAcpV1
        {
            return Err(ProviderAdapterError::parse_error(
                "kimi lc prepare facts: only Kimi ACP launches are accepted by this adapter",
                String::new(),
                String::new(),
            ));
        }
        // 3) exact version(TTL 观测 + 兼容门;不可得 fail-closed)。
        let (probe_key, observation) = self.observe_lc_version(cancel).await?;
        // 4) provider 特定 facts:boundary 由 envelope 派生;MCP 来源只
        //    对应 Aria 注入 bundle,无注入时 native 标记(不冒充 digest)。
        let boundary = projection::lc_boundary_plan(envelope).map_err(|error| {
            ProviderAdapterError::parse_error(
                format!("kimi lc prepare facts: {error}"),
                String::new(),
                String::new(),
            )
        })?;
        Ok(ProviderLcPrepareFacts::new(
            crate::product::logical_codebase::provider_gateway::ProviderRefType::KimiCode,
            envelope.provider_dialect,
            crate::product::logical_codebase::policy::ProviderWireDialect::KimiAcp,
            observation.exact_version().to_string(),
            observation.observed_at(),
            probe_key,
            projection::KIMI_LC_APPROVAL_POLICY.to_string(),
            self.lc_mcp_bundle_digest(),
            boundary,
            std::sync::Arc::new(KimiPolicyProjector::new(
                observation.exact_version().to_string(),
            )),
        ))
    }
}
