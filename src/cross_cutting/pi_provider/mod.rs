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
use crate::cross_cutting::process_manager::{
    PROVIDER_CHILD_EXIT_STREAM_DRAIN, ProcessManager, SessionTailExit, bounded_child_wait,
    bounded_task_join, provider_child_exit_error, run_session_tail_with_exit_watch,
};
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_resources::TrustedLaunchOwnership;
use crate::cross_cutting::streaming_provider::{
    LcVersionCache, LcVersionObservation, LcVersionProbe, LcVersionProbeKey, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, ProviderSession, ProviderStatus, ProviderVersionSupplier,
    StreamingProviderAdapter, StreamingProviderInput, VersionProbeError, canonical_tool_policy,
    validate_tool_policy_for_role,
};
use crate::cross_cutting::tool_policy_audit::{
    DurableToolPolicyEvent, ProviderStartAudit, ResumeDecision, ToolPolicyAuditSink,
    append_superseded_policy_drift, resume_with_audit_record,
};
use crate::product::logical_codebase::provider_projection::ProviderPolicyProjector;

mod parse;
mod projection;
mod session;

pub use projection::PiPolicyProjector;

/// pi 在 tool-policy canonical 序列中的 provider 名（CLI 名常量）。
pub const TOOL_POLICY_PROVIDER_NAME: &str = "pi";

/// DenyFileWriteBuiltins 的 pi canonical 物理片段（argv 片段冻结：`--exclude-tools
/// edit,write`；flag/value 按出现顺序原样、大小写保留）。
pub fn deny_file_write_builtins_tokens() -> Vec<String> {
    vec!["--exclude-tools".to_string(), "edit,write".to_string()]
}

#[cfg(test)]
pub mod tests;
#[cfg(test)]
pub mod usage_tests;

pub(crate) use parse::*;

pub const PI_COMMAND: &str = "pi";
const ARIA_ASK_EXTENSION: &str = include_str!("aria-ask.ts");
const MIN_PI_VERSION: &str = "0.83.0";
const PI_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProbeFailure {
    CommandFailed,
    TimedOut,
    Unparseable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PiVersion {
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
        // A1-b2(Bug45 批2 环境回退):探测环境不注入——空 env map,子进程
        // 经底座继承宿主 PATH,裸名 CLI 照常解析。
        environment: std::collections::BTreeMap::new(),
        stdout_limit: 64 * 1024,
        stderr_limit: 64 * 1024,
    };
    match TokioBoundedCommandRunner.run_inherited(request).await {
        Ok(result) if result.timed_out => PiVersion::Unknown(ProbeFailure::TimedOut),
        Ok(result) if result.exit_code == Some(0) => parse_pi_version(&result.stdout),
        Ok(_) | Err(_) => PiVersion::Unknown(ProbeFailure::CommandFailed),
    }
}

/// 真实 CLI 的可执行身份(Task 3.5/B0a2 `LcVersionProbeKey` 维度):内容
/// SHA-256——同路径替换 CLI 二进制即换 key,重观测不与旧值串值;不可读
/// (如尚未解析 PATH 的裸命令名)退化为路径文本(identity 仍随路径漂移,
/// 60 秒 TTL 窗口内的同值迟钝由 owner 裁决接受)。
fn lc_executable_identity(command: &Path) -> String {
    match std::fs::read(command) {
        Ok(bytes) => format!("sha256:{}", hex::encode(Sha256::digest(&bytes))),
        Err(_) => format!("unresolved:{}", command.display()),
    }
}

/// 策略会话版本映射（GC9，Task 3.3）：`Known` → 精确字符串；
/// `Unknown(TimedOut)` → `Timeout`；其余 `Unknown(_)` → `Unavailable`。
/// 仅策略会话走本 fail-closed 映射；既有 `ensure_pi_version_compatible` 的
/// unknown→Ok 旧路径与 Coder/非策略会话零变化。
pub(crate) fn pi_policy_version(version: &PiVersion) -> Result<String, VersionProbeError> {
    match version {
        PiVersion::Known((major, minor, patch)) => Ok(format!("pi {major}.{minor}.{patch}")),
        PiVersion::Unknown(ProbeFailure::TimedOut) => Err(VersionProbeError::Timeout),
        PiVersion::Unknown(_) => Err(VersionProbeError::Unavailable),
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

/// pi 的 adapter dialect 常量（GC9 冻结：`pi-rpc`）。resume 冻结三元组的方言位。
pub const PI_POLICY_DIALECT: &str = "pi-rpc";

fn tool_policy_session_error(message: impl std::fmt::Display) -> ProviderAdapterError {
    ProviderAdapterError::parse_error(
        format!("pi policy session: {message}"),
        String::new(),
        String::new(),
    )
}

#[derive(Clone)]
pub struct PiProvider {
    command: PathBuf,
    /// 策略会话 provider 版本 supplier（测试 seam；3.3 接线真实 CLI 探测后保留）。
    version_supplier: Option<ProviderVersionSupplier>,
    /// Task 3.5(B0d):LC 只读 facts 的 60 秒 TTL/single-flight 版本观测缓存
    /// (B0a2 合同;per-instance,clone 共享同一缓存槽)。与 direct `start`
    /// 的无过期 `cached_cli_version` 严格分离——direct 旧语义零变化。
    lc_version_cache: LcVersionCache,
    /// supplier 注入实例的 scope(`LcVersionProbeKey.supplier_scope`):不同
    /// 实例的测试 supplier 与真实 CLI 观测互不串值。
    lc_supplier_scope: u64,
    /// aria-ask extension 缓存目录注入面(测试 seam;`None` 沿
    /// `$HOME/.cache/cadence-aria` 现状)。
    extension_cache: Option<PathBuf>,
}

impl std::fmt::Debug for PiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PiProvider")
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

impl PiProvider {
    pub fn new(command: PathBuf) -> Self {
        // supplier scope 按构造实例分配(0 保留给真实 CLI 探测);clone 共享。
        static LC_SUPPLIER_SCOPE: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        Self {
            command,
            version_supplier: None,
            lc_version_cache: LcVersionCache::new(),
            lc_supplier_scope: LC_SUPPLIER_SCOPE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1,
            extension_cache: None,
        }
    }

    /// 注入策略会话 provider 版本 supplier（fixture/测试 seam）。
    pub fn with_version_supplier(mut self, supplier: ProviderVersionSupplier) -> Self {
        self.version_supplier = Some(supplier);
        self
    }

    /// 注入 aria-ask extension 缓存目录(测试 seam:mismatch 判定前零写断言
    /// 的可观测面;生产缺省沿 `$HOME/.cache/cadence-aria`)。
    pub fn with_extension_cache(mut self, cache: PathBuf) -> Self {
        self.extension_cache = Some(cache);
        self
    }

    /// 解析并准备 aria-ask extension:注入目录优先,缺省 `$HOME/.cache/
    /// cadence-aria`(同一 `ensure_ask_extension_in` 实现,direct 语义不变)。
    fn prepare_ask_extension(&self) -> Result<PathBuf, ProviderAdapterError> {
        match self.extension_cache.as_deref() {
            Some(cache) => ensure_ask_extension_in(cache),
            None => ensure_ask_extension(),
        }
    }

    /// LC 路径共用的前置守卫(Task 3.5:与 `prepare_lc_facts` 同源):run-context
    /// guard + adapter 匹配,整体早于版本观测/extension/session child。
    fn guard_lc_pi_launch(
        input: &StreamingProviderInput,
        launch: &crate::product::logical_codebase::provider_gateway::ValidatedSessionLaunchPolicy,
    ) -> Result<(), ProviderAdapterError> {
        crate::cross_cutting::streaming_provider::guard_lc_validated_launch(
            launch.is_root_recipe_phase(),
            &input.role,
            &input.role,
            input.tool_policy.as_ref(),
        )
        .map_err(|error| {
            ProviderAdapterError::parse_error(error.to_string(), String::new(), String::new())
        })?;
        if input.provider_type != crate::protocol::contracts::ProviderType::Pi
            || launch.envelope().provider_dialect
                != crate::product::logical_codebase::policy::ProviderDialect::PiRpcV1
        {
            return Err(ProviderAdapterError::parse_error(
                "pi lc launch: only Pi RPC launches are accepted by this adapter",
                String::new(),
                String::new(),
            ));
        }
        Ok(())
    }

    /// LC 版本观测 key(B0a2 合同):provider + canonical executable 路径 +
    /// 可执行身份(真实 CLI 为内容摘要,内容替换即换 key) + 版本观测参数 +
    /// supplier 实例范围。supplier 注入与真实 CLI 探测不同 key,不串值。
    fn lc_version_probe_key(&self) -> LcVersionProbeKey {
        let canonical_executable = self
            .command
            .canonicalize()
            .unwrap_or_else(|_| self.command.clone());
        let (executable_identity, argv, supplier_scope) = match self.version_supplier.as_ref() {
            Some(_) => (
                format!("version-supplier:{}", self.lc_supplier_scope),
                Vec::new(),
                self.lc_supplier_scope,
            ),
            None => (
                lc_executable_identity(&self.command),
                vec!["--version".to_string()],
                0,
            ),
        };
        LcVersionProbeKey {
            provider: crate::product::models::ProviderName::Pi,
            canonical_executable,
            executable_identity,
            argv,
            supplier_scope,
        }
    }

    /// LC 版本只读观测(60 秒 TTL/single-flight,B0a2):supplier seam 优先,
    /// 默认真实 `--version` 有界探测(沿既有 probe+fail-closed 映射,不复制
    /// 版本解析算法);失败/超时/取消不缓存、不回退过期成功。prepare 与
    /// spawn 前复验消费同一 key/缓存,不伪称每次实测二进制。
    async fn observe_lc_version(
        &self,
        cancel: CancellationToken,
    ) -> Result<(LcVersionProbeKey, LcVersionObservation), ProviderAdapterError> {
        let key = self.lc_version_probe_key();
        let probe: LcVersionProbe = match self.version_supplier.clone() {
            Some(supplier) => std::sync::Arc::new(move |_key| {
                let supplier = supplier.clone();
                Box::pin(async move { supplier() })
            }),
            None => {
                let command = self.command.clone();
                std::sync::Arc::new(move |_key| {
                    let command = command.clone();
                    Box::pin(async move {
                        let probed =
                            probe_pi_version_with_timeout(&command, PI_VERSION_PROBE_TIMEOUT).await;
                        pi_policy_version(&probed)
                    })
                })
            }
        };
        let observation = self
            .lc_version_cache
            .cached_lc_cli_version(key.clone(), probe, cancel)
            .await
            .map_err(|error| {
                ProviderAdapterError::parse_error(
                    format!("pi lc prepare facts: {error}"),
                    String::new(),
                    String::new(),
                )
            })?;
        Ok((key, observation))
    }

    /// Constructs Pi's Auto-only RPC command line.
    /// - `-e <path>` loads Aria's structured ask extension.
    /// - No `--session-dir`: Pi uses its default `~/.pi` directory.
    /// - No `--no-extensions`: Pi preserves user-global extensions.
    /// - The project repository is passed as the process cwd at spawn time.
    /// - Tool policy（REQ-ENV-09）：`Some(DenyFileWriteBuiltins)` 时在 `--session-id`
    ///   逻辑之后追加冻结片段 `--exclude-tools edit,write`（置于 session id 之后，
    ///   不改变其顺序；空 session id 不会降级成无限制 argv）。
    pub(crate) fn build_args(
        &self,
        resume_session_id: Option<&str>,
        extension_path: &Path,
        tool_policy: Option<&crate::cross_cutting::streaming_provider::ProviderToolPolicy>,
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
        if let Some(crate::cross_cutting::streaming_provider::ProviderToolPolicy {
            intent:
                crate::cross_cutting::streaming_provider::ToolPolicyIntent::DenyFileWriteBuiltins,
        }) = tool_policy
        {
            args.extend(deny_file_write_builtins_tokens());
        }
        args
    }

    /// LC validated 启动的 argv(Task 4b):与 direct `build_args` 同形态
    /// (rpc 模式 + aria-ask extension + `--session-id` + 策略角色的
    /// exclude-tools 冻结片段),独立方法显式冻结 LC 语义,后续 LC 特化
    /// 不污染 direct argv。direct `build_args` 保持逐字节不变。
    pub(crate) fn build_lc_validated_args(
        &self,
        resume_session_id: Option<&str>,
        extension_path: &Path,
        tool_policy: Option<&crate::cross_cutting::streaming_provider::ProviderToolPolicy>,
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
        if let Some(crate::cross_cutting::streaming_provider::ProviderToolPolicy {
            intent:
                crate::cross_cutting::streaming_provider::ToolPolicyIntent::DenyFileWriteBuiltins,
        }) = tool_policy
        {
            args.extend(deny_file_write_builtins_tokens());
        }
        args
    }
}

/// Task 9a:LC 启动的完整 resume 审计字面量(投影摘要 + v2 resume 材料)。
/// action evidence 摘要与 `capability_projection_digest` 同源;resume 指纹为
/// 五参 `from_envelope` 全上下文计算。
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

impl PiProvider {
    /// direct 启动实现(`StreamingProviderAdapter::start` 的本体)。B1-1:
    /// `owned` 携带受信 launch 归属时,spawn 前预留、spawn 点回填真实
    /// PID+starttime;`None` 保持既有裸 start 语义。预留紧邻 spawn
    /// (F2:版本探测/extension 等可取消 await 之前零 durable 副作用),
    /// 收口注销生命周期由 `start_owned` 覆写与 Drop 守卫承担。
    async fn start_direct(
        &self,
        mut input: StreamingProviderInput,
        cancel: CancellationToken,
        owned: Option<&TrustedLaunchOwnership>,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // 双向 spawn 前守卫（Task 3.1）：非法角色×策略组合在创建子进程之前拒绝。
        validate_tool_policy_for_role(&input.role, input.tool_policy.as_ref()).map_err(
            |error| {
                ProviderAdapterError::parse_error(error.to_string(), String::new(), String::new())
            },
        )?;
        let version = probe_pi_version(&self.command).await;
        ensure_pi_version_compatible(&version)?;
        let extension_path = self.prepare_ask_extension()?;
        // 策略会话（Task 3.2/3.3）：spawn 前解析版本并执行 resume 冻结三元组比对；
        // pi 的有界握手是 id 预生成/传入（既有 `--session-id` 语义）；fresh 策略
        // 会话预生成 uuid 并经 `--session-id` 传入，使 native session id 在 spawn 前
        // 即可知。spawn 后、start 返回前写 `provider_start`；握手/写失败终止子进程
        // 并 fail-closed。
        let mut policy_start: Option<(
            std::sync::Arc<dyn ToolPolicyAuditSink>,
            String,
            String,
            String,
        )> = None;
        // GC9：resume 记录缺失与 drift 同路径处置（清除 resume id、全新会话）；
        // 「标记 superseded」仅带内 ToolPolicyWarning（🔴 无旧文件可写，不伪造
        // durable 文件），在事件通道建立后送出。
        let mut superseded_record_missing = false;
        // 批2 Task 20（硬约束6，codex T13 同构）：run-bound audit sink 在位
        // 即统一 launch 审计合同——岗位纪律会话（Author deny 投影）必须
        // 携带 sink（缺失 fail-closed 保持）；ReviewerVerify 窗口合同等
        // 合同 run 同样走预生成 id 握手+provider_start 首行。marker 不进
        // 本合同（BOOT-04，root recipe receipt 承担）。
        let discipline_policy = input.tool_policy.as_ref().is_some_and(|policy| {
            matches!(
                policy.intent,
                crate::cross_cutting::streaming_provider::ToolPolicyIntent::DenyFileWriteBuiltins
            )
        });
        let audit_contract_sink = if discipline_policy {
            Some(input.audit_sink.clone().ok_or_else(|| {
                tool_policy_session_error("audit sink is required for policy sessions")
            })?)
        } else {
            input.audit_sink.clone()
        };
        if let Some(sink) = audit_contract_sink {
            let context_label = if discipline_policy {
                "pi policy session"
            } else {
                "pi contract session"
            };
            // 版本解析（Task 3.3）：supplier seam 优先；默认走真实 `--version` 探测
            // （进程内缓存、有界超时），不可得则合同会话启动 fail-closed。
            let provider_version = match self.version_supplier.clone() {
                Some(supplier) => supplier().map_err(tool_policy_session_error)?,
                None => {
                    // P2-2：版本探测走既有进程内缓存（GC9；claude/codex 同法）。
                    let command = self.command.clone();
                    crate::cross_cutting::streaming_provider::cached_cli_version(
                        &self.command,
                        async move {
                            let probed =
                                probe_pi_version_with_timeout(&command, PI_VERSION_PROBE_TIMEOUT)
                                    .await;
                            pi_policy_version(&probed)
                        },
                    )
                    .await
                    .map_err(tool_policy_session_error)?
                }
            };
            // 决策项①(T20b P0):不 panic——deny/ReviewerVerify → canonical
            // digest(窗口域);None → 无策略指纹(空串,codex T13 口径);
            // marker 不进本合同(BOOT-04,root recipe receipt 承担)。
            let tool_policy_digest = match input.tool_policy.as_ref() {
                Some(
                    policy @ crate::cross_cutting::streaming_provider::ProviderToolPolicy {
                        intent:
                            crate::cross_cutting::streaming_provider::ToolPolicyIntent::DenyFileWriteBuiltins,
                    },
                )
                | Some(
                    policy @ crate::cross_cutting::streaming_provider::ProviderToolPolicy {
                        intent:
                            crate::cross_cutting::streaming_provider::ToolPolicyIntent::ReviewerVerify { .. },
                    },
                ) => canonical_tool_policy(TOOL_POLICY_PROVIDER_NAME, policy)
                    .map_err(|error| {
                        tool_policy_session_error(format!("{context_label}: {error}"))
                    })?
                    .digest,
                Some(
                    crate::cross_cutting::streaming_provider::ProviderToolPolicy {
                        intent:
                            crate::cross_cutting::streaming_provider::ToolPolicyIntent::BootstrapExecutorMarker(_),
                    },
                ) => String::new(),
                None => String::new(),
            };
            // 批2 Task 20（RevB2T13 ⚠4 同构）：resume 三元组门只服务岗位
            // 纪律会话；非策略合同 run 连续性由 native 会话承担。
            let resume_id = if discipline_policy {
                input
                    .resume_provider_session_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(ToString::to_string)
            } else {
                None
            };
            if let Some(resume_id) = resume_id.as_ref() {
                let stored = sink
                    .find_provider_start(resume_id)
                    .map_err(tool_policy_session_error)?;
                let current = ProviderStartAudit {
                    workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
                    provider_session_id: resume_id.clone(),
                    tool_policy_canonical_digest: tool_policy_digest.clone(),
                    provider_version: provider_version.clone(),
                    adapter_dialect: PI_POLICY_DIALECT.to_string(),
                    ..ProviderStartAudit::default()
                };
                if let Some(stored) = stored.as_ref()
                    && matches!(
                        resume_with_audit_record(Some(stored.record.clone()), &current),
                        ResumeDecision::RejectSupersedeAndStartNew
                    )
                {
                    // P1-4 裁决：superseded 终止审计写入被取代旧 run 的文件（其
                    // provider_start 已是首行；被终止的是旧会话），新 run 照常从
                    // provider_start 开始。
                    append_superseded_policy_drift(sink.as_ref(), stored)
                        .map_err(tool_policy_session_error)?;
                    input.resume_provider_session_id = None;
                } else if stored.is_none() {
                    // GC9：记录缺失与 drift 同路径处置——清除 resume id、以全新会话
                    // （新预生成 id + 新 run 的 provider_start）启动；「标记 superseded」
                    // 仅带内 ToolPolicyWarning：无旧文件可写，🔴 不得伪造无
                    // provider_start 首行的 durable 文件（首行不变量优先）。
                    input.resume_provider_session_id = None;
                    superseded_record_missing = true;
                }
            }
            let native_session_id = input
                .resume_provider_session_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            policy_start = Some((
                sink,
                provider_version,
                tool_policy_digest,
                native_session_id,
            ));
        }
        let resume_session_id = match &policy_start {
            Some((_, _, _, native_session_id)) => Some(native_session_id.clone()),
            _ => input
                .resume_provider_session_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string),
        };
        let args = self.build_args(
            resume_session_id.as_deref(),
            &extension_path,
            input.tool_policy.as_ref(),
        );
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let command = self.command.to_string_lossy().to_string();
        // B1-1 F2:预留紧邻 spawn(与 Claude/Codex 同序)——此前的版本探测/
        // extension 等可取消 await 不产生 durable 副作用;launch future 在
        // 探测处被引擎 drop 时不会留下"预留未确认"。守卫覆盖预留→spawn→
        // 回填之间仅剩的可取消点(瞬态 spawn 重试 sleep),Drop 即收口注销。
        let (reservation, mut launch_guard) = match owned {
            Some(ownership) => (
                Some(ownership.reserve()?),
                Some(
                    crate::cross_cutting::streaming_provider::LaunchReservationGuard::new(
                        ownership.clone(),
                    ),
                ),
            ),
            None => (None, None),
        };
        let process = match ProcessManager::spawn(
            &command,
            &arg_refs,
            &input.working_dir,
            &input.env_vars,
            cancel.clone(),
        )
        .await
        {
            Ok(process) => process,
            Err(error) => {
                // spawn 失败=零子进程:预留未确认,显式注销(守卫之外幂等兜底)。
                if let Some(guard) = launch_guard.as_mut() {
                    guard.disarm();
                }
                if let Some(ownership) = owned {
                    ownership.release_after_session_tail().await;
                }
                return Err(error);
            }
        };

        let peer = JsonRpcPeer::new(process.stdout, process.stdin);
        let stderr = process.stderr;
        let mut child = process.child;
        // B1-1 接线片:spawn 成功即回填真实 PID+starttime(/proc 观察,PID
        // 复用防护);确认失败整组停止——kill+wait 后上抛,收口注销由
        // 守卫/`start_owned` 错误路径承担(身份未确认按无身份注销)。
        if let (Some(ownership), Some(reservation)) = (owned, reservation.as_ref()) {
            if let Err(error) =
                ownership.backfill_spawned_identity(reservation, child.id().unwrap_or(0))
            {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(error);
            }
        }
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

        // GC9：resume 记录缺失的带内 superseded 标记（先于新会话 provider_start，
        // 与 drift 路径的 durable 顺序镜像）。
        if superseded_record_missing {
            let _ = event_tx
                .send(crate::cross_cutting::streaming_provider::superseded_policy_record_missing_warning())
                .await;
        }

        // 策略会话：spawn 后、start 返回前写 `provider_start`（握手=id 已预生成/传入
        // 完成）；append 失败终止子进程并返回错误（engine 沿既有 kill 链判失败）。
        let mut native_session_id = None;
        if let Some((sink, provider_version, digest, session_id)) = policy_start {
            let audit_event = DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                provider: TOOL_POLICY_PROVIDER_NAME.to_string(),
                role: crate::cross_cutting::streaming_provider::adapter_role_text(&input.role)
                    .to_string(),
                workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
                provider_session_id: session_id.clone(),
                tool_policy_canonical_digest: digest,
                argv: args.clone(),
                sandbox: None,
                approval_policy: None,
                provider_version,
                adapter_dialect: PI_POLICY_DIALECT.to_string(),
                lc_projection: None,
            });
            if let Err(error) = sink.append_bound(audit_event) {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(tool_policy_session_error(format!(
                    "provider_start audit append failed: {error}"
                )));
            }
            native_session_id = Some(session_id);
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

            // r19 续修(fix 轮 2):exit-watch 竞速。run_pi_session 对自身错误
            // 已发终态 Failed(Failed-先于-join 先例);子进程先退出且排空
            // 耗尽时泵被截断、终态未发——由 wrapper 显式补发同形态终态。
            let result = match run_session_tail_with_exit_watch(
                session::run_pi_session(peer, command_rx, event_tx.clone(), input, cancel),
                &mut child,
                PROVIDER_CHILD_EXIT_STREAM_DRAIN,
                provider_child_exit_error,
            )
            .await
            {
                SessionTailExit::Stream(result) => result,
                SessionTailExit::ChildExitBeforeTerminal { error } => {
                    let _ = event_tx
                        .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                        .await;
                    let _ = event_tx
                        .send(ProviderEvent::Failed {
                            message: error.details.clone(),
                        })
                        .await;
                    Err(error)
                }
            };
            drop(bridge);
            if result.is_err() {
                let _ = child.start_kill();
            }
            // r19 续修:终态 join 一律有界(红线 a)。
            let status =
                match bounded_child_wait(&mut child, PROVIDER_CHILD_EXIT_STREAM_DRAIN).await {
                    Some(status) => Ok(status),
                    None => Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "bounded child join elapsed before exit status",
                    )),
                };
            let _ = bounded_task_join(stderr_task, PROVIDER_CHILD_EXIT_STREAM_DRAIN).await;
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

        // 会话已建立并交由 `release_session_after_tail` 尾部收口:守卫退出舞台。
        if let Some(guard) = launch_guard.as_mut() {
            guard.disarm();
        }
        Ok(ProviderSession {
            native_session_id,
            events: event_rx,
            commands: command_tx,
        })
    }
}

/// B1-1(波 2):成功路径的会话包装——源事件通道真实终结(子进程退出后
/// 有界排深,终结保证契约;**非**协议 `Completed` 事件点)或消费方提前
/// 丢弃会话后,按登记身份真实收口(killpg+有界观察)再注销。包装通道
/// 只转发事件,不引入新的无界等待;注销不推迟消费方 `recv` 终结(包装
/// 通道在源终结点之后才关闭)。与 `streaming_provider::start_owned_default`
/// 的成功路径生命周期同构,供 direct/raw 与 LC validated 两个 spawn 面复用。
fn release_session_after_tail(
    session: ProviderSession,
    ownership: TrustedLaunchOwnership,
) -> ProviderSession {
    let ProviderSession {
        native_session_id,
        events,
        commands,
    } = session;
    let (tail_tx, tail_rx) = mpsc::channel(32);
    let release = ownership.clone();
    tokio::spawn(async move {
        let mut source = events;
        while let Some(event) = source.recv().await {
            if tail_tx.send(event).await.is_err() {
                // 消费方提前丢弃会话(中止路径):同样先收口注销。
                break;
            }
        }
        // 会话真实终止 → 进程组真实收口后注销;静止不可证时保留 durable
        // 记录停等(scope 级 quiesce/恢复驱动接管)。
        release.release_after_session_tail().await;
    });
    ProviderSession {
        native_session_id,
        events: tail_rx,
        commands,
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PiProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // B1-1(波 2):raw start 面统一走 start_direct;ownership=None 保持
        // 既有裸语义(无登记、零行为变化)。归属生命周期在 start_owned
        // 覆写中接线,不从此入口旁路。
        self.start_direct(input, cancel, None).await
    }

    /// B1-1(波 2):受信资源归属启动覆写——在默认生命周期(预留→start→
    /// 错误路径收口注销/成功路径尾部收口注销)之上,把载体穿到 spawn 点:
    /// 预留紧邻 spawn(F2),spawn 成功即经 `backfill_spawned_identity`
    /// 回填真实 PID+starttime(确认失败整组停止)。未携带载体保持既有
    /// 裸 start 语义(无登记,零行为变化)。注销不在协议 `Completed`
    /// 事件点——源事件通道真实终结(终结保证契约)后才按登记身份收口;
    /// 静止不可证时保留 durable 记录停等(scope 级 quiesce/恢复驱动接管)。
    async fn start_owned(
        &self,
        input: StreamingProviderInput,
        ownership: Option<TrustedLaunchOwnership>,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let Some(ownership) = ownership else {
            // 无归属请求(legacy/单仓 direct/测试面):既有裸 start 语义。
            return self.start(input, cancel).await;
        };
        match self.start_direct(input, cancel, Some(&ownership)).await {
            Ok(session) => Ok(release_session_after_tail(session, ownership)),
            Err(error) => {
                // 错误路径:adapter 错误链已 kill+wait;有登记身份则
                // killpg+有界观察真实收口后注销(未回填身份按无身份注销)。
                ownership.release_after_session_tail().await;
                Err(error)
            }
        }
    }

    /// LC validated 启动(Task 4b):只接受 gateway 产出的 validated input,
    /// 消费冻结 envelope 派生的不可伪造 boundary plan 与 LC 权限投影,以
    /// canonical LC root 为进程 cwd 启动,统一落盘 `ProviderStartAudit.
    /// lc_projection`;无通用 tool policy 的角色(Coding/Executor)同样执行
    /// exact version 解析、原生会话与统一 launch audit,不以
    /// `tool_policy=None` 早退。Task 9:resume 在 child 前比较全 LC audit
    /// (9a),spawn 后由 RPC `get_state` 真实确认同 id(9c)——缺/错 id
    /// kill/reap 并显式记录「未恢复」,不回填请求 id、不清 id 转 fresh。
    /// Task 3.5(B0d):exact version 与 `prepare_lc_facts` 消费同一 60 秒
    /// TTL 观测;full-context resume 携带 prepare 冻结候选投影时,复验
    /// version 后直接消费该投影(audit 与 prepared 同 fingerprint);
    /// extension 准备在 resume mismatch 判定之后(拒绝路径零 extension 写)。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // B1-1:读取受信 launch 归属(crate 私有绑定面;`into_parts` 不携带,
        // 先取后拆)。`None` = 无归属请求(存量构造/测试),零登记零注销。
        let launch_ownership = validated.launch_ownership().cloned();
        let (input, launch) = validated.into_parts();
        let envelope = launch.envelope().clone();

        // 共用 LC 前置守卫(Task 3.5:与 `prepare_lc_facts` 同源):run-context
        // guard + adapter 匹配,整体早于版本观测/extension/session child。
        Self::guard_lc_pi_launch(&input, &launch)?;

        // 统一 launch audit sink:LC 会话必须绑定 run-bound sink,缺失
        // fail-closed(1b 的 prepare 契约之前由本路径强制)。
        let sink = input.audit_sink.clone().ok_or_else(|| {
            ProviderAdapterError::parse_error(
                "pi lc validated start: audit sink is required for LC launches",
                String::new(),
                String::new(),
            )
        })?;

        // Task 3.5(B0d):exact version 与 `prepare_lc_facts` 同源——同一 60
        // 秒 TTL/single-flight 观测(prepare 与 spawn 前复验同 key 命中,不
        // 伪称每次实测二进制;失败/超时/取消不缓存)。全 LC 路径必填,非仅
        // 策略角色;direct `start` 的无过期 `cached_cli_version` 不受影响。
        let (_version_probe_key, observation) = self.observe_lc_version(cancel.clone()).await?;
        let provider_version = observation.exact_version().to_string();

        // 投影来源(B0d):full-context resume 携带 prepare 冻结的候选投影时,
        // adapter 先复验本家 mutable fact(exact version)——漂移 typed 拒
        // (等价 gateway `assert_frozen_version` 的 provider_version 维度),
        // 一致则**消费该投影**为 audit 字面量(prepared 与 audit 同
        // fingerprint);裸 validate(无候选投影)沿现状:同版本 projector
        // 现算。trust/MCP bundle digest 的 gateway 侧装配归 1b/1c/B0f;空串
        // 同样纳入 session digest,装配后任一漂移都会改变 projection_digest。
        let lc_projection = match launch.lc_projection() {
            Some(prepared) => {
                if prepared.exact_version() != provider_version.as_str() {
                    return Err(ProviderAdapterError::parse_error(
                        format!(
                            "pi lc validated start: prepared projection provider version {} drifted from current observation {provider_version}; refusing to spawn",
                            prepared.exact_version()
                        ),
                        String::new(),
                        String::new(),
                    ));
                }
                prepared.clone()
            }
            None => {
                let boundary = projection::lc_boundary_plan(&envelope).map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("pi lc validated start: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?;
                let projection_input = crate::product::logical_codebase::provider_projection::ProviderProjectionInput::new(
                    envelope.clone(),
                    crate::product::logical_codebase::provider_gateway::ProviderRef::pi(
                        launch.capability_snapshot_ref(),
                    ),
                    envelope.action,
                    input.role.clone(),
                    input.permission_mode.clone(),
                    input.tool_policy.clone(),
                    projection::PI_LC_APPROVAL_POLICY.to_string(),
                    String::new(),
                    envelope.config_artifact_ref.clone(),
                    String::new(),
                    Some(boundary),
                );
                PiPolicyProjector::new(provider_version.clone())
                    .project(&projection_input)
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!("pi lc validated start: {error}"),
                            String::new(),
                            String::new(),
                        )
                    })?
            }
        };

        // Task 9a:LC 显式 resume 在 child 前比较全 LC audit(完整字面量含
        // 投影)。存档缺 lc_projection(旧/direct 形态)或任一投影摘要漂移
        // → superseded 终止审计写被取代旧 run、零 child 拒绝(不清 resume
        // id 静默 fresh);无存档记录时依赖 native 握手确认(9c 收口),
        // 本层不清 id。
        if let Some(resume_id) = input
            .resume_provider_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            let stored = sink.find_provider_start(resume_id).map_err(|error| {
                ProviderAdapterError::parse_error(
                    format!("pi lc validated start: resume lookup failed: {error}"),
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
                                "pi lc validated start: superseded audit append failed: {error}"
                            ),
                            String::new(),
                            String::new(),
                        )
                    })?;
                    return Err(ProviderAdapterError::parse_error(
                        "pi lc validated start: resume audit drifted or legacy record lacks lc projection; refusing to spawn a fresh child for an explicit LC resume",
                        String::new(),
                        String::new(),
                    ));
                }
            }
        }

        // Task 3.5(B0d):extension 准备移至 resume mismatch 判定之后——
        // mismatch 拒绝路径零 extension 写(读事实阶段不落盘);以下才进入
        // launcher 覆盖面(RPC/MCP 继续既有 adapter)。
        let extension_path = self.prepare_ask_extension()?;

        // 原生会话 id:fresh 预生成 uuid,resume 复用冻结 id(spawn 后由
        // Task 9c 的 get_state 确认真实同 id 才放行)。
        let native_session_id = input
            .resume_provider_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // LC argv 与投影同源;进程 cwd = envelope 冻结的 canonical LC root。
        let args = self.build_lc_validated_args(
            Some(native_session_id.as_str()),
            &extension_path,
            input.tool_policy.as_ref(),
        );
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let command = self.command.to_string_lossy().to_string();
        let process_cwd = envelope.working_directory.clone();
        // B1-1 接线片:载体在位 → 预留先于 spawn(durable;失败零 spawn),
        // 载体/预留活到错误/取消/正常尾部的收口注销。
        let owned = launch_ownership
            .as_ref()
            .map(|ownership| {
                ownership
                    .reserve()
                    .map(|reservation| (ownership.clone(), reservation))
            })
            .transpose()?;
        // B1-1 F2:守卫覆盖预留→spawn→回填之间仅剩的可取消点(瞬态 spawn
        // 重试 sleep / spawn 后握手的可取消 await);launch future 被 drop
        // 时由 Drop 收口注销,不留下未确认登记。
        let mut launch_guard = owned.as_ref().map(|(ownership, _)| {
            crate::cross_cutting::streaming_provider::LaunchReservationGuard::new(ownership.clone())
        });
        let process = match ProcessManager::spawn(
            &command,
            &arg_refs,
            &process_cwd,
            &input.env_vars,
            cancel.clone(),
        )
        .await
        {
            Ok(process) => process,
            Err(error) => {
                // 预留已落盘但 spawn 未发生:无身份收口注销后上抛。
                if let Some((ownership, _reservation)) = owned.as_ref() {
                    ownership.release_after_session_tail().await;
                }
                return Err(error);
            }
        };

        let peer = JsonRpcPeer::new(process.stdout, process.stdin);
        let stderr = process.stderr;
        let mut child = process.child;
        // B1-1 接线片:spawn 成功即回填真实 PID+starttime(/proc 观察,PID
        // 复用防护);确认失败整组停止——kill+wait 后收口注销再上抛
        // (不伪称零 spawn)。
        if let Some((ownership, reservation)) = owned.as_ref() {
            if let Err(error) =
                ownership.backfill_spawned_identity(reservation, child.id().unwrap_or(0))
            {
                let _ = child.start_kill();
                let _ = child.wait().await;
                ownership.release_after_session_tail().await;
                return Err(error);
            }
        }
        let (event_tx, event_rx) = mpsc::channel(32);
        // pi 是 Auto-only:与 direct 路径一致构造 bridge,但授权/命令转发
        // 永不经过它。
        let bridge = ApprovalBridge::new(ProviderPermissionMode::Auto, event_tx.clone());

        // Task 9c:LC 显式 resume 的原生确认——spawn 后、事件/audit 落盘前
        // 用 RPC `get_state` 应答核对同 id;缺/错 id 属于已启动 child 后的
        // runtime 失败:kill/reap 并返回显式「未恢复」错误(不伪称零
        // spawn、不回填请求 id、不清 id 转 fresh)。fresh 会话不受影响。
        if let Some(requested_native_id) = input
            .resume_provider_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            if let Err(error) =
                session::confirm_pi_native_resume_id(&peer, &cancel, requested_native_id).await
            {
                let _ = child.start_kill();
                let _ = child.wait().await;
                // B1-1:握手失败(已 spawn)——kill+wait 后按登记身份收口
                // 注销再上抛。
                if let Some((ownership, _reservation)) = owned.as_ref() {
                    ownership.release_after_session_tail().await;
                }
                return Err(error);
            }
        }
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
                cwd: Some(process_cwd.display().to_string()),
                output: None,
                exit_code: None,
            }))
            .await;

        // 统一 launch audit:spawn 后、start 返回前写 `provider_start`
        // (握手 = id 已预生成/传入完成);无通用 tool policy 也有 canonical
        // digest(空 token 序列形态)。append 失败终止子进程并 fail-closed
        // (engine 沿既有 kill 链判失败)。
        let tool_policy_digest =
            match projection::lc_tool_policy_canonical_digest(input.tool_policy.as_ref()) {
                Ok(digest) => digest,
                Err(error) => {
                    // B1-1:spawn 已发生——归属路径先 kill+wait 再收口注销,
                    // 不留活 child;无归属路径保持既有上抛语义。
                    if let Some((ownership, _reservation)) = owned.as_ref() {
                        let _ = child.start_kill();
                        let _ = child.wait().await;
                        ownership.release_after_session_tail().await;
                    }
                    return Err(ProviderAdapterError::parse_error(
                        format!("pi lc validated start: {error}"),
                        String::new(),
                        String::new(),
                    ));
                }
            };
        let audit_event = DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
            provider: TOOL_POLICY_PROVIDER_NAME.to_string(),
            role: crate::cross_cutting::streaming_provider::adapter_role_text(&input.role)
                .to_string(),
            workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
            provider_session_id: native_session_id.clone(),
            tool_policy_canonical_digest: tool_policy_digest,
            argv: args.clone(),
            sandbox: None,
            approval_policy: None,
            provider_version,
            adapter_dialect: PI_POLICY_DIALECT.to_string(),
            lc_projection: Some(crate::cross_cutting::tool_policy_audit::LcProjectionAudit {
                action: projection::action_text(envelope.action).to_string(),
                wire_dialect: projection::wire_dialect_text(lc_projection.wire_dialect())
                    .to_string(),
                capability_projection_digest: lc_projection
                    .capability_projection_digest()
                    .to_string(),
                projection_digest: lc_projection.projection_digest().to_string(),
                boundary_evidence_ref: lc_projection.boundary_evidence_ref().to_string(),
            }),
        });
        if let Err(error) = sink.append_bound(audit_event) {
            let _ = child.start_kill();
            let _ = child.wait().await;
            // B1-1:审计失败 kill 链之后按登记身份收口注销再上抛。
            if let Some((ownership, _reservation)) = owned.as_ref() {
                ownership.release_after_session_tail().await;
            }
            return Err(tool_policy_session_error(format!(
                "provider_start audit append failed: {error}"
            )));
        }

        // 会话收尾任务(与 direct `start` 同构:stderr 收集 + run_pi_session +
        // 失败 kill 链)。
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

            // r19 续修(fix 轮 2):同 direct 路径——exit-watch 竞速 + 截断时
            // wrapper 补发终态 Failed + 终态 join 有界(红线 a)。
            let result = match run_session_tail_with_exit_watch(
                session::run_pi_session(peer, command_rx, event_tx.clone(), input, cancel),
                &mut child,
                PROVIDER_CHILD_EXIT_STREAM_DRAIN,
                provider_child_exit_error,
            )
            .await
            {
                SessionTailExit::Stream(result) => result,
                SessionTailExit::ChildExitBeforeTerminal { error } => {
                    let _ = event_tx
                        .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                        .await;
                    let _ = event_tx
                        .send(ProviderEvent::Failed {
                            message: error.details.clone(),
                        })
                        .await;
                    Err(error)
                }
            };
            drop(bridge);
            if result.is_err() {
                let _ = child.start_kill();
            }
            let status =
                match bounded_child_wait(&mut child, PROVIDER_CHILD_EXIT_STREAM_DRAIN).await {
                    Some(status) => Ok(status),
                    None => Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "bounded child join elapsed before exit status",
                    )),
                };
            let _ = bounded_task_join(stderr_task, PROVIDER_CHILD_EXIT_STREAM_DRAIN).await;
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

        let session = ProviderSession {
            native_session_id: Some(native_session_id),
            events: event_rx,
            commands: command_tx,
        };
        // B1-1:成功路径包装——源事件通道真实终结(终结保证契约;**非**
        // 协议 Completed 事件点)或消费方提前丢弃后,按登记身份收口注销。
        if let Some(guard) = launch_guard.as_mut() {
            guard.disarm();
        }
        Ok(match launch_ownership {
            Some(ownership) => release_session_after_tail(session, ownership),
            None => session,
        })
    }

    /// Task 3.5(B0d):LC 公共 prepare 合同(B0a2)的 Pi 真实现——只读观测
    /// 本家 facts:exact version(经 `LcVersionCache` 60 秒
    /// TTL/single-flight)、Auto-only aria-ask approval 投影、无可信 MCP
    /// 配置(空 digest)、由 envelope 派生的不可伪造 boundary plan 与
    /// **同版本** `PiPolicyProjector`。全程零 extension 写、零 child、零
    /// durable 写(extension 准备归 `start_validated` 的 mismatch 判定
    /// 之后);守卫早于版本观测。
    async fn prepare_lc_facts(
        &self,
        input: &StreamingProviderInput,
        launch: &crate::product::logical_codebase::provider_gateway::ValidatedSessionLaunchPolicy,
        cancel: CancellationToken,
    ) -> Result<
        crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts,
        ProviderAdapterError,
    > {
        Self::guard_lc_pi_launch(input, launch)?;
        let (version_probe_key, observation) = self.observe_lc_version(cancel).await?;
        let exact_version = observation.exact_version().to_string();
        let boundary = projection::lc_boundary_plan(launch.envelope()).map_err(|error| {
            ProviderAdapterError::parse_error(
                format!("pi lc prepare facts: {error}"),
                String::new(),
                String::new(),
            )
        })?;
        Ok(
            crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts::new(
                crate::product::logical_codebase::provider_gateway::ProviderRefType::Pi,
                crate::product::logical_codebase::policy::ProviderDialect::PiRpcV1,
                crate::product::logical_codebase::policy::ProviderWireDialect::PiRpc,
                exact_version.clone(),
                observation.observed_at(),
                version_probe_key,
                projection::PI_LC_APPROVAL_POLICY.to_string(),
                String::new(),
                boundary,
                std::sync::Arc::new(PiPolicyProjector::new(exact_version)),
            ),
        )
    }
}

/// B1 审查 F2 回归:launch future 在版本探测处被引擎 drop 时,不得留下
/// "预留未确认"的 durable Process 记录(该记录会让同 attempt 后续
/// quiesce 恒返回 RecoveryRequired,阻塞下一次开窗)。预留紧邻 spawn 之后,
/// 版本探测阶段零 durable 副作用。
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
    /// rpc 模式落 rpc marker 后常驻。测试据此把 launch future 精确停在版本
    /// 探测 await 上,并在 drop 后确认从未进入 provider spawn。
    fn blocking_probe_fixture(
        dir: &Path,
        probe_marker: &Path,
        release_marker: &Path,
        rpc_marker: &Path,
    ) -> PathBuf {
        let body = format!(
            "if [ \"$1\" = \"--version\" ]; then\n  touch \"{probe}\"\n  n=0\n  while [ ! -f \"{release}\" ] && [ $n -lt 200 ]; do sleep 0.01; n=$((n+1)); done\n  echo 0.83.0\n  exit 0\nfi\ntouch \"{rpc}\"\nwhile IFS= read -r line; do :; done",
            probe = probe_marker.display(),
            release = release_marker.display(),
            rpc = rpc_marker.display(),
        );
        write_executable(dir, "fake-pi-blocking-probe", &body)
    }

    fn process_records(store_root: &Path, key: &ResourceScopeKey) -> Vec<ProviderResourceRecord> {
        ProviderResourceStore::new(store_root)
            .list_scope(key)
            .expect("list pi cancel scope")
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
            provider_type: ProviderType::Pi,
            role: AdapterRole::Executor,
            prompt: "pi cancel probe".to_string(),
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
    async fn pi_direct_cancel_during_version_probe_leaves_no_reservation() {
        let base = tempfile::tempdir().expect("pi cancel base");
        let private_root = base.path().join("private-root");
        std::fs::create_dir_all(&private_root).expect("private root");
        let store_root = base.path().join("resource-store");
        std::fs::create_dir_all(&store_root).expect("store root");
        let key = ResourceScopeKey {
            scope_id: "code-delivery-attempt-pi-cancel".to_string(),
            run_id: "role-run-pi-cancel".to_string(),
            boot_identity: format!("aria-engine-{}-pi-cancel", std::process::id()),
        };
        let scope = ProviderRunResourceScope::open_for_recovery(
            &key,
            None,
            &private_root,
            &store_root,
            None,
        )
        .expect("open pi cancel scope");
        let ownership = TrustedLaunchOwnership::mint(
            Arc::new(scope),
            "code_delivery provider process: pi cancel",
        );

        let marker_dir = tempfile::tempdir().expect("pi cancel markers");
        let probe_marker = marker_dir.path().join("probe-started");
        let release_marker = marker_dir.path().join("probe-release");
        let rpc_marker = marker_dir.path().join("rpc-spawned");
        let working_dir = marker_dir.path().join("cwd");
        std::fs::create_dir_all(&working_dir).expect("working dir");
        let provider = PiProvider::new(blocking_probe_fixture(
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
            match tokio::time::timeout(Duration::from_millis(10), &mut launch).await {
                Ok(_) => panic!("launch 不得在版本探测完成前返回"),
                Err(_) => {}
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
            "取消于版本探测时不得进入 provider spawn(rpc marker 不得出现)"
        );
    }
}
