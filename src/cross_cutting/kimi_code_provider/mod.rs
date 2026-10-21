use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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
use crate::cross_cutting::provider_resources::{ResourceReservation, TrustedLaunchOwnership};
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
        // A1-b2(Bug45 批2 环境回退):探测环境不注入——空 env map,子进程
        // 经底座继承宿主 PATH,裸名 CLI 照常解析。
        environment: std::collections::BTreeMap::new(),
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
                    // A1-b2(Bug45 批2 环境回退):探测环境不注入——空 env
                    // map,子进程经底座继承宿主 PATH,裸名 CLI 照常解析。
                    environment: std::collections::BTreeMap::new(),
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

/// B1-1(波 2):direct 路径的资源登记启动体。engine 已用真实 scope 铸造
/// `TrustedLaunchOwnership` 并经 `start_owned` 传入;本 impl 只负责在
/// spawn 点回填真实 PID+starttime,并让登记句柄活到会话真实收口。
impl KimiCodeProvider {
    /// spawn 点统一登记片:预留已在调用方完成;spawn 成功后立即回填真实
    /// PID+`/proc` starttime(PID 复用防护)。回填失败(身份不可读/确认写
    /// 失败)整组停止子进程并返回 Err——登记由调用方按无身份/已确认身份
    /// 收口,不在此提前注销。
    async fn spawn_registered(
        &self,
        args: &[&str],
        working_dir: &Path,
        env_vars: &BTreeMap<String, String>,
        cancel: CancellationToken,
        registration: Option<&(TrustedLaunchOwnership, ResourceReservation)>,
    ) -> Result<crate::cross_cutting::process_manager::ManagedProcess, ProviderAdapterError> {
        let command = self.command.to_string_lossy().to_string();
        let mut process =
            ProcessManager::spawn(&command, args, working_dir, env_vars, cancel).await?;
        if let Some((ownership, reservation)) = registration {
            let Some(pid) = process.child.spawn_leader_pid() else {
                process.child.terminate().await;
                return Err(ProviderAdapterError::provider_unavailable(
                    "kimi provider launch identity unavailable: spawn 未暴露组长 PID",
                ));
            };
            if let Err(error) = ownership.backfill_spawned_identity(reservation, pid) {
                process.child.terminate().await;
                return Err(error);
            }
        }
        Ok(process)
    }

    /// direct 启动体:`ownership = None` 为既有裸 start 语义(零登记);
    /// `Some` 为受信载体 launch。预留紧邻 spawn(F2:版本探测等可取消
    /// await 之前零 durable 副作用);spawn 前失败=零 spawn,预留未确认
    /// 由外层按无身份注销;会话真实尾部(子进程回收、事件通道关闭)后
    /// 按登记身份收口注销——不在协议 Completed。
    async fn start_session(
        &self,
        input: StreamingProviderInput,
        ownership: Option<TrustedLaunchOwnership>,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let mut registration: Option<(TrustedLaunchOwnership, ResourceReservation)> = None;
        let mut launch_guard = None;
        let result = self
            .start_session_inner(
                input,
                ownership.as_ref(),
                &mut registration,
                &mut launch_guard,
                cancel,
            )
            .await;
        // 错误路径(含 spawn 前门禁)与内层提前返回:守卫退出舞台,显式
        // 收口注销;成功路径内层已把句柄交给尾部任务并 disarm。
        if let Some(guard) = launch_guard.as_mut() {
            guard.disarm();
        }
        if result.is_err()
            && let Some((ownership, _reservation)) = registration.take()
        {
            ownership.release_after_session_tail().await;
        }
        result
    }

    async fn start_session_inner(
        &self,
        input: StreamingProviderInput,
        ownership: Option<&TrustedLaunchOwnership>,
        registration: &mut Option<(TrustedLaunchOwnership, ResourceReservation)>,
        launch_guard: &mut Option<crate::cross_cutting::streaming_provider::LaunchReservationGuard>,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // 批2 Task 20a-fix(F2):四家同源启动检查首步——非法角色×策略
        // 组合(Reviewer 无窗/空窗/deny 回退、合同专属失配)在版本探测与
        // 子进程之前拒绝(0 会话,不 fallback)。
        crate::cross_cutting::streaming_provider::validate_tool_policy_for_role(
            &input.role,
            input.tool_policy.as_ref(),
        )
        .map_err(|error| {
            ProviderAdapterError::parse_error(error.to_string(), String::new(), String::new())
        })?;
        // 版本兼容门(既有 direct start 契约,微修复轮 2 恢复):守卫先行,
        // 版本探测紧随——CLI 版本低于兼容线或 --version 不可解析时在
        // adapter 层 fail-closed,不推迟到 ACP 握手。
        let version = probe_kimi_version(&self.command).await;
        ensure_kimi_version_compatible(&version)?;
        let args = self.build_args();
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        // B1-1 F2:预留紧邻 spawn(版本探测之后,与 Claude/Codex 同序);
        // 守卫覆盖预留→spawn→回填之间仅剩的可取消点(瞬态 spawn 重试
        // sleep),launch future 被引擎 drop 时由 Drop 收口注销。
        if let Some(ownership) = ownership {
            *registration = Some((ownership.clone(), ownership.reserve()?));
            *launch_guard = Some(
                crate::cross_cutting::streaming_provider::LaunchReservationGuard::new(
                    ownership.clone(),
                ),
            );
        }
        let process = self
            .spawn_registered(
                &arg_refs,
                &input.working_dir,
                &input.env_vars,
                cancel.clone(),
                registration.as_ref(),
            )
            .await?;
        let peer = JsonRpcPeer::new(process.stdout, process.stdin);
        let stderr = process.stderr;
        let mut child = process.child;
        let mcp_injection = self.mcp_injection.clone();
        let (event_tx, event_rx) = mpsc::channel(32);
        let (command_tx, command_rx) = mpsc::channel(8);
        // B1-1:登记句柄随会话尾部任务存活——错误路径与正常后台尾部都在
        // 子进程回收、事件通道关闭后才注销。
        let tail_ownership = registration
            .take()
            .map(|(ownership, _reservation)| ownership);
        // 登记句柄已交给尾部任务:守卫退出舞台(此后注销归尾部收口)。
        if let Some(guard) = launch_guard.as_mut() {
            guard.disarm();
        }
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
            // 先关事件通道(消费方 recv 终结不被注销等待推迟),再按登记
            // 身份 killpg+有界观察收口;静止不可证时保留 durable 记录。
            drop(event_tx);
            if let Some(ownership) = tail_ownership {
                ownership.release_after_session_tail().await;
            }
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

/// Task 9a(REQ-ENV-04):LC 启动的完整 resume 审计字面量(投影摘要 + v2
/// resume 材料),与 codex/pi/claude 三家的同名装配逐字段同源。action
/// evidence 摘要与 `capability_projection_digest` 同源;resume 指纹为五参
/// `from_envelope` 全上下文计算(policy/authority/cwd/target/git identity/
/// exact version/语义 tool policy/trust/config/MCP 都在投影摘要内)。
fn lc_start_audit_literal(
    envelope: &crate::product::logical_codebase::policy::SessionPolicyEnvelope,
    lc_projection: &crate::product::logical_codebase::provider_projection::ProviderPolicyProjection,
) -> crate::cross_cutting::tool_policy_audit::LcProviderStartAudit {
    let git_identity =
        crate::product::logical_codebase::provider_gateway::canonical_target_git_identity(
            &envelope.target.worktree,
        );
    let action_evidence = lc_projection.capability_projection_digest().to_string();
    crate::cross_cutting::tool_policy_audit::LcProviderStartAudit {
        action: projection::action_text(envelope.action).to_string(),
        wire_dialect: projection::wire_dialect_text(lc_projection.wire_dialect()).to_string(),
        capability_projection_digest: action_evidence.clone(),
        projection_digest: lc_projection.projection_digest().to_string(),
        boundary_evidence_ref: lc_projection.boundary_evidence_ref().to_string(),
        resume_fingerprint:
            crate::product::logical_codebase::provider_gateway::SessionResumeFingerprint::from_envelope(
                envelope,
                lc_projection,
                &action_evidence,
                &git_identity,
            )
            .digest,
        action_evidence_digest: action_evidence,
        target_git_identity: git_identity,
    }
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
        self.start_session(input, None, cancel).await
    }

    /// B1-1(波 2):direct 路径的受信资源登记接线。载体由 engine 三处真实
    /// scope 铸造(本次 launch 唯一资源 ID):预留紧邻 spawn(F2:版本探测
    /// 等可取消 await 之前零 durable 副作用)→ spawn → 回填真实 PID+
    /// starttime → 会话真实收口(进程组静止)后注销。未携带载体 = 无归属
    /// 请求(legacy/测试面),走 trait 默认——既有裸 start 语义,不在此
    /// 伪造登记、也不绕过默认方法。
    async fn start_owned(
        &self,
        input: StreamingProviderInput,
        ownership: Option<TrustedLaunchOwnership>,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let Some(ownership) = ownership else {
            return crate::cross_cutting::streaming_provider::start_owned_default(
                self, input, None, cancel,
            )
            .await;
        };
        self.start_session(input, Some(ownership), cancel).await
    }

    /// LC validated 启动(Task 4c):只接受 gateway 产出的 validated input。
    /// Kimi 通用 tool 为 `None`——非空通用策略在 version/child 之前拒绝
    /// (稳定码 `provider_generic_tool_policy_forbidden`);`None` 不早退,
    /// 仍执行 exact version probe、native session/handshake 与统一
    /// `ProviderStartAudit.lc_projection` 落盘。进程 cwd 与 ACP 协议 cwd
    /// 都保持 canonical LC root;target 写面由宿主 fs/terminal handler
    /// 消费不可伪造 boundary plan(不进协议 cwd)。`mcp_bundle_digest`
    /// 只对应 Aria 注入,无注入时标记 native 项目配置来源。direct
    /// `start` 的 wire 行为不变(无载体 = 零登记)。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // B1-1:validated 载体的受信归属与 direct 同一生命周期——预留紧邻
        // spawn(F2:版本探测/投影/argv 等可取消 await 之前零 durable 副
        // 作用);spawn 后回填真实 PID+starttime;登记句柄活到会话真实尾部。
        // 未携带载体 = 无归属请求(存量构造)。
        let ownership = validated.launch_ownership().cloned();
        let mut registration: Option<(TrustedLaunchOwnership, ResourceReservation)> = None;
        let mut launch_guard = None;
        let result = async {
        let (mut input, launch) = validated.into_parts();
        let envelope = launch.envelope().clone();

        // 1) 拒绝非空通用策略(version/child 之前;Global Constraints:
        //    Kimi 拒绝非空通用策略,继续既有 ClientServicePolicy)。批2
        //    Task 20 唯一例外:`ReviewerVerify` 窗口合同——kimi 侧由
        //    冻结执行合同的受信岗位授予等价消费(窗口引用进 grant,不
        //    造通用策略物理面),不在此拒绝。T7 稳定码合同保持:marker
        //    等非豁免 intent 先按 provider_generic_tool_policy_forbidden 拒。
        if input.tool_policy.is_some()
            && !matches!(
                input.tool_policy.as_ref().map(|policy| &policy.intent),
                Some(
                    crate::cross_cutting::streaming_provider::ToolPolicyIntent::ReviewerVerify { .. }
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

        // 批2 Task 20a-fix(F2)+T20b 决策项③:LC validated 共用 run-context
        // guard(三家同构)——角色一致性 + RootRecipe 相位 marker 门 +
        // 双向策略守卫(Reviewer 无窗=0 会话,不 fallback)。
        crate::cross_cutting::streaming_provider::guard_lc_validated_launch(
            launch.is_root_recipe_phase(),
            &input.role,
            &input.role,
            input.tool_policy.as_ref(),
        )
        .map_err(|error| {
            ProviderAdapterError::parse_error(error.to_string(), String::new(), String::new())
        })?;

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

        // 5b) Task 9a(REQ-ENV-04):LC 显式 resume 在 child 前比较全 LC
        //     audit(完整字面量含冻结投影 + v2 resume 材料),与
        //     codex/pi/claude 三家同形。存档缺 `lc_projection`(旧/direct
        //     形态)或任一投影摘要漂移 → superseded 终止审计写被取代旧
        //     run、零 child 拒绝,绝不清 resume id 静默 fresh。无存档记录
        //     时与三家同为「依赖原生握手确认」,本层不额外拒绝(共享口径,
        //     不在此单方改变)。门位于版本/投影复验之后、预留与 spawn 之前
        //     ——拒绝路径零 child、零登记、零新 provider_start。生产构造
        //     (`KimiCodeProvider::new`,无 Aria MCP bundle)同样必经此门,
        //     不因 MCP digest 惰性而退化。
        if let Some(resume_id) = input
            .resume_provider_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            let stored = sink.find_provider_start(resume_id).map_err(|error| {
                ProviderAdapterError::parse_error(
                    format!("kimi lc validated start: resume lookup failed: {error}"),
                    String::new(),
                    String::new(),
                )
            })?;
            if let Some(stored) = stored.as_ref() {
                let current = lc_start_audit_literal(&envelope, &lc_projection);
                if matches!(
                    crate::cross_cutting::tool_policy_audit::resume_with_lc_start_record(
                        &stored.record,
                        &current
                    ),
                    crate::cross_cutting::tool_policy_audit::ResumeDecision::RejectSupersedeAndStartNew
                ) {
                    // P1-4 同源裁决:superseded 终止审计写入被取代旧 run。
                    crate::cross_cutting::tool_policy_audit::append_superseded_policy_drift(
                        sink.as_ref(),
                        stored,
                    )
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!(
                                "kimi lc validated start: superseded audit append failed: {error}"
                            ),
                            String::new(),
                            String::new(),
                        )
                    })?;
                    return Err(ProviderAdapterError::parse_error(
                        "kimi lc validated start: resume audit drifted or legacy record lacks lc projection; refusing to spawn a fresh child for an explicit LC resume (session NOT started; no child was spawned)",
                        String::new(),
                        String::new(),
                    ));
                }
            }
        }

        // 6) argv 与 direct 同源(冻结 `acp`,无策略物理片段);进程 cwd =
        //    envelope 冻结的 canonical LC root;ACP 协议 cwd 同样保持
        //    root(Task 4 Interfaces:ACP cwd 保持 root,target 写面经
        //    plan 由宿主 handler 消费,不进协议 cwd)。
        let args = self.build_args();
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let process_cwd = envelope.working_directory.clone();
        // B1-1 F2:预留紧邻 spawn(版本观测/投影复验/argv 全部完成之后)。
        // 守卫覆盖预留→spawn→回填之间仅剩的可取消点(瞬态 spawn 重试
        // sleep 与 spawn 后握手),launch future 被引擎 drop 时由 Drop 收口。
        if let Some(ownership) = ownership.as_ref() {
            registration = Some((ownership.clone(), ownership.reserve()?));
            launch_guard = Some(
                crate::cross_cutting::streaming_provider::LaunchReservationGuard::new(
                    ownership.clone(),
                ),
            );
        }
        let process = self
            .spawn_registered(
                &arg_refs,
                &process_cwd,
                &input.env_vars,
                cancel.clone(),
                registration.as_ref(),
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
        // B1-1:登记句柄随会话尾部任务存活——握手失败/取消/审计失败与正常
        // 后台尾部都在子进程回收、事件通道关闭后才注销。
        let tail_ownership = registration
            .take()
            .map(|(ownership, _reservation)| ownership);
        // 登记句柄已交给尾部任务:守卫退出舞台(此后注销归尾部收口)。
        if let Some(guard) = launch_guard.as_mut() {
            guard.disarm();
        }

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
            // 先关事件通道(消费方 recv 终结不被注销等待推迟),再按登记
            // 身份 killpg+有界观察收口;静止不可证时保留 durable 记录停等。
            drop(event_tx);
            if let Some(ownership) = tail_ownership {
                ownership.release_after_session_tail().await;
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
        .await;
        // 错误路径(含 spawn 前门禁):守卫退出舞台,显式收口注销;成功
        // 路径内层已把句柄交给尾部任务并 disarm。spawn 之前的失败零
        // spawn,无身份 → 直接注销;spawn 后回填失败的子进程已整组停止。
        if let Some(guard) = launch_guard.as_mut() {
            guard.disarm();
        }
        if result.is_err()
            && let Some((ownership, _reservation)) = registration.take()
        {
            ownership.release_after_session_tail().await;
        }
        result
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
        //    guard 早于 probe/child)。批2 Task 20a-fix(F1):ReviewerVerify
        //    窗口合同例外放行——kimi 侧窗口经 TrustedDutyGrant 消费。
        if input.tool_policy.is_some()
            && !matches!(
                input.tool_policy.as_ref().map(|policy| &policy.intent),
                Some(
                    crate::cross_cutting::streaming_provider::ToolPolicyIntent::ReviewerVerify { .. }
                )
            )
        {
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

/// B1 审查 F2 回归:Kimi direct 面的预留必须紧邻 spawn(版本探测之后)。
/// launch future 在 `--version` 探测处被引擎 drop 时不得留下"预留未确认"
/// 的 durable Process 记录(该记录会让同 attempt 后续 quiesce 恒返回
/// RecoveryRequired,阻塞下一次开窗)。
#[cfg(all(test, unix))]
mod b1_launch_cancel_tests {
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::cross_cutting::provider_resources::{
        OwnedResourceKind, ProviderResourceRecord, ProviderRunResourceScope, ResourceScopeKey,
        TrustedLaunchOwnership,
    };
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::product::provider_resource_store::ProviderResourceStore;
    use crate::protocol::contracts::{AdapterRole, ProviderType};

    fn write_executable(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).expect("create fixture");
        writeln!(file, "#!/bin/sh\n{body}").expect("write fixture");
        let mut permissions = file.metadata().expect("fixture metadata").permissions();
        permissions.set_mode(0o755);
        file.set_permissions(permissions).expect("chmod fixture");
        path
    }

    /// `--version` 先落 probe marker,再等 release(有界 2s)后打印兼容版本;
    /// ACP 模式落 rpc marker 后常驻。测试据此把 launch future 精确停在版本
    /// 探测 await 上,并在 drop 后确认从未进入 provider spawn。
    fn blocking_probe_fixture(
        dir: &Path,
        probe_marker: &Path,
        release_marker: &Path,
        rpc_marker: &Path,
    ) -> PathBuf {
        let body = format!(
            "if [ \"$1\" = \"--version\" ]; then\n  touch \"{probe}\"\n  n=0\n  while [ ! -f \"{release}\" ] && [ $n -lt 200 ]; do sleep 0.01; n=$((n+1)); done\n  echo kimi 0.34.0\n  exit 0\nfi\ntouch \"{rpc}\"\nwhile IFS= read -r line; do :; done",
            probe = probe_marker.display(),
            release = release_marker.display(),
            rpc = rpc_marker.display(),
        );
        write_executable(dir, "fake-kimi-blocking-probe", &body)
    }

    fn process_records(store_root: &Path, key: &ResourceScopeKey) -> Vec<ProviderResourceRecord> {
        ProviderResourceStore::new(store_root)
            .list_scope(key)
            .expect("list kimi cancel scope")
            .into_iter()
            .filter(|record| matches!(record.kind, OwnedResourceKind::Process))
            .collect()
    }

    fn input_for_test(working_dir: PathBuf) -> StreamingProviderInput {
        StreamingProviderInput {
            working_directory: None,
            baseline_tree: None,
            tool_policy: None,
            audit_sink: None,
            provider_type: ProviderType::KimiCode,
            role: AdapterRole::Orchestrator,
            prompt: "kimi cancel probe".to_string(),
            working_dir,
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: 60,
        }
    }

    #[tokio::test]
    async fn kimi_direct_cancel_during_version_probe_leaves_no_reservation() {
        let base = tempfile::tempdir().expect("kimi cancel base");
        let private_root = base.path().join("private-root");
        std::fs::create_dir_all(&private_root).expect("private root");
        let store_root = base.path().join("resource-store");
        std::fs::create_dir_all(&store_root).expect("store root");
        let key = ResourceScopeKey {
            scope_id: "code-delivery-attempt-kimi-cancel".to_string(),
            run_id: "role-run-kimi-cancel".to_string(),
            boot_identity: format!("aria-engine-{}-kimi-cancel", std::process::id()),
        };
        let scope = ProviderRunResourceScope::open_for_recovery(
            &key,
            None,
            &private_root,
            &store_root,
            None,
        )
        .expect("open kimi cancel scope");
        let ownership = TrustedLaunchOwnership::mint(
            Arc::new(scope),
            "code_delivery provider process: kimi cancel",
        );

        let marker_dir = tempfile::tempdir().expect("kimi cancel markers");
        let probe_marker = marker_dir.path().join("probe-started");
        let release_marker = marker_dir.path().join("probe-release");
        let rpc_marker = marker_dir.path().join("rpc-spawned");
        let working_dir = marker_dir.path().join("cwd");
        std::fs::create_dir_all(&working_dir).expect("working dir");
        let provider = KimiCodeProvider::new(blocking_probe_fixture(
            marker_dir.path(),
            &probe_marker,
            &release_marker,
            &rpc_marker,
        ));

        let mut launch = Box::pin(provider.start_owned(
            input_for_test(working_dir),
            Some(ownership),
            CancellationToken::new(),
        ));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if probe_marker.exists() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "fixture 的 --version 探测必须启动(probe marker 未出现)"
            );
            if tokio::time::timeout(Duration::from_millis(10), &mut launch)
                .await
                .is_ok()
            {
                panic!("launch 不得在版本探测完成前返回")
            }
        }
        // 引擎取消时序:provider_start 的 select 分支 cancel 后 drop launch future。
        drop(launch);
        std::fs::write(&release_marker, b"release").expect("release probe");

        // 取消发生在版本探测(预留之前)→ scope 内不得有任何 durable 登记。
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if process_records(&store_root, &key).is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "取消于版本探测时不得留下未确认预留: {:?}",
                process_records(&store_root, &key)
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !rpc_marker.exists(),
            "取消于版本探测时不得进入 provider spawn(ACP marker 不得出现)"
        );
    }
}
