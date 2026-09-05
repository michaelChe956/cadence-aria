use std::collections::BTreeMap;
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
use crate::cross_cutting::process_manager::ProcessManager;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderExecutionEvent, ProviderExecutionEventKind,
    ProviderExecutionEventStatus, ProviderPermissionMode, ProviderSession, ProviderStatus,
    ProviderVersionSupplier, StreamingProviderAdapter, StreamingProviderInput, VersionProbeError,
    canonical_tool_policy, validate_tool_policy_for_role,
};
use crate::cross_cutting::tool_policy_audit::{
    DurableToolPolicyEvent, ProviderStartAudit, ResumeDecision, ToolPolicyAuditSink,
    append_superseded_policy_drift, resume_with_audit_record,
};

mod parse;
mod session;

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
        environment: BTreeMap::new(),
        stdout_limit: 64 * 1024,
        stderr_limit: 64 * 1024,
    };
    match TokioBoundedCommandRunner.run_inherited(request).await {
        Ok(result) if result.timed_out => PiVersion::Unknown(ProbeFailure::TimedOut),
        Ok(result) if result.exit_code == Some(0) => parse_pi_version(&result.stdout),
        Ok(_) | Err(_) => PiVersion::Unknown(ProbeFailure::CommandFailed),
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
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PiProvider {
    async fn start(
        &self,
        mut input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // 双向 spawn 前守卫（Task 3.1）：非法角色×策略组合在创建子进程之前拒绝。
        validate_tool_policy_for_role(&input.role, input.tool_policy.as_ref()).map_err(
            |error| {
                ProviderAdapterError::parse_error(error.to_string(), String::new(), String::new())
            },
        )?;
        let version = probe_pi_version(&self.command).await;
        ensure_pi_version_compatible(&version)?;
        let extension_path = ensure_ask_extension()?;
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
        if let Some(policy) = input.tool_policy.as_ref() {
            let sink = input.audit_sink.clone().ok_or_else(|| {
                tool_policy_session_error("audit sink is required for policy sessions")
            })?;
            // 版本解析（Task 3.3）：supplier seam 优先；默认走真实 `--version` 探测
            // （进程内缓存、有界超时），不可得则策略会话启动 fail-closed。
            let provider_version = match self.version_supplier.clone() {
                Some(supplier) => supplier().map_err(tool_policy_session_error)?,
                None => {
                    let probed =
                        probe_pi_version_with_timeout(&self.command, PI_VERSION_PROBE_TIMEOUT)
                            .await;
                    pi_policy_version(&probed).map_err(tool_policy_session_error)?
                }
            };
            let canonical = canonical_tool_policy(TOOL_POLICY_PROVIDER_NAME, policy)
                .map_err(|error| tool_policy_session_error(error.to_string()))?;
            // resume 冻结三元组比对（Task 3.3，spawn 前）：记录缺失或 digest/version/
            // dialect 任一不一致 → 追加 superseded 终止审计并新建会话（丢弃 resume id）。
            let resume_id = input
                .resume_provider_session_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string);
            if let Some(resume_id) = resume_id.as_ref() {
                let stored = sink
                    .find_provider_start(resume_id)
                    .map_err(tool_policy_session_error)?;
                let current = ProviderStartAudit {
                    workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
                    provider_session_id: resume_id.clone(),
                    tool_policy_canonical_digest: canonical.digest.clone(),
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
                }
            }
            let native_session_id = input
                .resume_provider_session_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            policy_start = Some((sink, provider_version, canonical.digest, native_session_id));
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

        // 策略会话：spawn 后、start 返回前写 `provider_start`（握手=id 已预生成/传入
        // 完成）；append 失败终止子进程并返回错误（engine 沿既有 kill 链判失败）。
        let mut native_session_id = None;
        if let Some((sink, provider_version, digest, session_id)) = policy_start {
            let audit_event = DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                provider: TOOL_POLICY_PROVIDER_NAME.to_string(),
                role: crate::cross_cutting::streaming_provider::UsageReportData::role_text(
                    &input.role,
                )
                .to_string(),
                workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
                provider_session_id: session_id.clone(),
                tool_policy_canonical_digest: digest,
                argv: args.clone(),
                sandbox: None,
                approval_policy: None,
                provider_version,
                adapter_dialect: PI_POLICY_DIALECT.to_string(),
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
            native_session_id,
            events: event_rx,
            commands: command_tx,
        })
    }
}
