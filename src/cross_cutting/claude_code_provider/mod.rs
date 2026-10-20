use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::{ApprovalBridge, ChoiceDecision};
use crate::cross_cutting::process_manager::{ManagedProcessChild, ProcessManager};
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_resources::{ResourceReservation, TrustedLaunchOwnership};
use crate::cross_cutting::streaming_provider::{
    ChoiceAnswerData, ChoiceOptionData, ChoiceQuestionData, ChoiceRequestData, ChoiceRequestSource,
    LcVersionCache, LcVersionObservation, LcVersionProbe, LcVersionProbeKey, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, ProviderSession, ProviderStatus, ProviderVersionSupplier, RiskLevel,
    StreamingProviderAdapter, StreamingProviderInput, UsageReportData, canonical_tool_policy,
    validate_tool_policy_for_role,
};
use crate::cross_cutting::structured_output::StructuredOutputContract;
use crate::cross_cutting::tool_policy_audit::{
    DurableToolPolicyEvent, ProviderStartAudit, ToolPolicyAuditSink,
};

mod ask_user_question;
mod direct_start;
mod projection;
mod stream;
mod tool;
mod validated_start;

pub use projection::ClaudePolicyProjector;

#[cfg(test)]
pub mod tests;

const TOOL_RESULT_PREVIEW_MAX_BYTES: usize = 500;

/// claude 的 adapter dialect 常量（GC9 冻结：`claude-stream-json`）。
pub const CLAUDE_POLICY_DIALECT: &str = "claude-stream-json";

/// claude 在 tool-policy canonical 序列中的 provider 名（CLI 名常量）。
pub const TOOL_POLICY_PROVIDER_NAME: &str = "claude-code";

/// DenyFileWriteBuiltins 的 claude canonical 物理片段（denylist 冻结：
/// `--disallowedTools Edit,Write,NotebookEdit`；名单大小写与成员冻结）。
pub fn deny_file_write_builtins_tokens() -> Vec<String> {
    vec![
        "--disallowedTools".to_string(),
        "Edit,Write,NotebookEdit".to_string(),
    ]
}

/// claude CLI `--version` 有界探测超时（GC9）。
pub const CLAUDE_VERSION_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// claude CLI `--version` 有界探测（Task 3.3）：成功非空返回精确字符串；
/// 空输出/命令失败 → `Unavailable`；超时 → `Timeout`。
pub async fn probe_claude_version(
    command: &std::path::Path,
    timeout: std::time::Duration,
) -> Result<String, crate::cross_cutting::streaming_provider::VersionProbeError> {
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandRequest, TokioBoundedCommandRunner,
    };
    use crate::cross_cutting::streaming_provider::VersionProbeError;
    use tokio_util::sync::CancellationToken;

    let request = BoundedCommandRequest {
        executable: command.to_string_lossy().into_owned(),
        argv: vec!["--version".to_string()],
        working_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        timeout,
        cancellation: CancellationToken::new(),
        // A1-b2(Bug45 批2 环境回退):探测环境不注入——空 env map,子进程
        // 经底座继承宿主 PATH,裸名 CLI 照常解析(显式覆盖层语义见
        // bounded_command_runner)。
        environment: std::collections::BTreeMap::new(),
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

/// CLI 可执行身份(LC probe key 维度,Task 3.3):文件长度 + mtime 纳秒的
/// 稳定文本;重写/替换同一 canonical 路径下的二进制即换身份 → 新 key 再
/// 观测(不命中旧 TTL 缓存)。元数据不可得时退化为路径文本(探测本身随后
/// fail-closed,不伪造身份)。
fn lc_executable_identity(command: &std::path::Path) -> String {
    match std::fs::metadata(command) {
        Ok(metadata) => {
            let mtime_ns = metadata
                .modified()
                .ok()
                .and_then(|moment| moment.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            format!("claude-cli:len:{}:mtime_ns:{mtime_ns}", metadata.len())
        }
        Err(_) => format!("claude-cli:unresolved:{}", command.to_string_lossy()),
    }
}
#[derive(Debug, Clone)]
struct ClaudePermissionRequest {
    request_id: String,
    tool_use_id: Option<String>,
    tool_name: String,
    description: String,
    input: Value,
}

#[derive(Debug, Clone)]
struct ResolvedAskUserQuestion {
    answers: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone)]
struct ToolUseBlock {
    id: String,
    name: String,
    input: Value,
}

#[derive(Debug, Clone)]
struct ToolResultBlock {
    tool_use_id: String,
    output: String,
    is_error: bool,
}

#[derive(Clone)]
pub struct ClaudeCodeProvider {
    command: PathBuf,
    /// 策略会话 provider 版本 supplier（测试 seam；3.3 接线真实 CLI 探测后保留）。
    version_supplier: Option<ProviderVersionSupplier>,
    /// supplier 实例范围（LC probe key 的 `supplier_scope` 维度;真实 CLI
    /// 探测恒 0,每个注入 supplier 实例单调分配,测试 supplier 与真实 CLI
    /// 不串值）。
    version_supplier_scope: u64,
    /// Task 3.3(B0b):LC exact-version 只读观测的 60 秒 TTL/single-flight
    /// 缓存(实例内;与 direct 路径的无过期 `cached_cli_version` 严格分离,
    /// 进程/实例重建即失效,不跨实例复用旧值)。
    lc_version_cache: LcVersionCache,
}

impl std::fmt::Debug for ClaudeCodeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeCodeProvider")
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeStreamOutcome {
    TerminalEventEmitted,
    Aborted,
    EofWithoutResult,
}

fn permission_mode_for_claude(mode: &ProviderPermissionMode) -> &'static str {
    // 新版 CLI UI 显示为 manual，wire 兼容名仍为 default。
    match mode {
        ProviderPermissionMode::Auto | ProviderPermissionMode::Supervised => "default",
    }
}

impl ClaudeCodeProvider {
    pub fn new(command: PathBuf) -> Self {
        Self {
            command,
            version_supplier: None,
            version_supplier_scope: 0,
            lc_version_cache: LcVersionCache::new(),
        }
    }

    /// 注入策略会话 provider 版本 supplier（fixture/测试 seam）。
    pub fn with_version_supplier(mut self, supplier: ProviderVersionSupplier) -> Self {
        static NEXT_SUPPLIER_SCOPE: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(1);
        self.version_supplier = Some(supplier);
        self.version_supplier_scope =
            NEXT_SUPPLIER_SCOPE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self
    }

    /// LC 版本观测 probe key:provider + canonical executable 路径 + 可执行
    /// 身份(长度+mtime;路径/身份任一变化即换 key,不失效其它 key)+
    /// `--version` argv + supplier 实例范围(3.2 `LcVersionProbeKey` 合同)。
    fn lc_version_probe_key(&self, executable_identity: String) -> LcVersionProbeKey {
        LcVersionProbeKey {
            provider: crate::product::models::ProviderName::ClaudeCode,
            canonical_executable: std::fs::canonicalize(&self.command)
                .unwrap_or_else(|_| self.command.clone()),
            executable_identity,
            argv: vec!["--version".to_string()],
            supplier_scope: self.version_supplier_scope,
        }
    }

    /// Task 3.3(B0b):Claude LC 只读 facts 的单一观测实现——`prepare_lc_facts`
    /// 与 `start_validated` 共用同一提取(guard/adapter 匹配由两入口各自
    /// 先行,观测本身零 spawn/零写):
    /// - exact version:supplier seam 优先(测试);默认真实 CLI `--version`
    ///   有界探测经实例内 60 秒 TTL/single-flight `LcVersionCache`(失败/
    ///   取消不缓存、不回退过期成功);
    /// - boundary:envelope 派生的不可伪造 boundary plan(与 argv 同源);
    /// - projector:与 exact version 同源的 `ClaudePolicyProjector`(provider
    ///   owner 不变;不从 registry/capability 空 version 取证)。
    async fn observe_lc_facts(
        &self,
        launch: &crate::product::logical_codebase::provider_gateway::ValidatedSessionLaunchPolicy,
        cancel: CancellationToken,
    ) -> Result<
        crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts,
        ProviderAdapterError,
    > {
        let (observation, key) = match self.version_supplier.clone() {
            Some(supplier) => {
                let version = supplier().map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("claude lc prepare facts: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?;
                let key = self.lc_version_probe_key(format!(
                    "claude-cli:supplier:{}",
                    self.version_supplier_scope
                ));
                (
                    LcVersionObservation::new(version, std::time::Instant::now()),
                    key,
                )
            }
            None => {
                let key = self.lc_version_probe_key(lc_executable_identity(&self.command));
                let command = self.command.clone();
                let probe: LcVersionProbe = Arc::new(move |_probe_key| {
                    let command = command.clone();
                    Box::pin(async move {
                        probe_claude_version(&command, CLAUDE_VERSION_PROBE_TIMEOUT).await
                    })
                });
                let observation = self
                    .lc_version_cache
                    .cached_lc_cli_version(key.clone(), probe, cancel)
                    .await
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!("claude lc prepare facts: {error}"),
                            String::new(),
                            String::new(),
                        )
                    })?;
                (observation, key)
            }
        };
        let envelope = launch.envelope();
        let boundary = projection::lc_boundary_plan(envelope).map_err(|error| {
            ProviderAdapterError::parse_error(
                format!("claude lc prepare facts: {error}"),
                String::new(),
                String::new(),
            )
        })?;
        let projector = ClaudePolicyProjector::new(observation.exact_version().to_string())
            .with_root_recipe_phase(launch.is_root_recipe_phase());
        Ok(
            crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts::new(
                crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode,
                envelope.provider_dialect,
                crate::product::logical_codebase::policy::ProviderWireDialect::ClaudeCodeStreamJson,
                observation.exact_version().to_string(),
                observation.observed_at(),
                key,
                projection::CLAUDE_LC_APPROVAL_POLICY.to_string(),
                String::new(),
                boundary,
                Arc::new(projector),
            ),
        )
    }

    /// - Tool policy（REQ-ENV-09）：`Some(DenyFileWriteBuiltins)` 时追加冻结片段
    ///   `--disallowedTools Edit,Write,NotebookEdit`（名单大小写与成员冻结，fresh/resume
    ///   均保留）；非策略 input（`None`）保持原 argv。
    fn build_args(
        &self,
        resume_provider_session_id: Option<&str>,
        tool_policy: Option<&crate::cross_cutting::streaming_provider::ProviderToolPolicy>,
    ) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            "--verbose".to_string(),
            "--output-format=stream-json".to_string(),
            "--input-format=stream-json".to_string(),
            "--include-partial-messages".to_string(),
            "--replay-user-messages".to_string(),
        ];

        if let Some(session_id) = resume_provider_session_id
            .map(str::trim)
            .filter(|session_id| !session_id.is_empty())
        {
            args.push("--resume".to_string());
            args.push(session_id.to_string());
        }

        if let Some(crate::cross_cutting::streaming_provider::ProviderToolPolicy {
            intent:
                crate::cross_cutting::streaming_provider::ToolPolicyIntent::DenyFileWriteBuiltins,
        }) = tool_policy
        {
            args.extend(deny_file_write_builtins_tokens());
        }

        args.push("--permission-prompt-tool=stdio".to_string());

        args
    }

    /// LC validated 启动的 argv(Task 4a):在 headless 基础片段上追加冻结的
    /// MCP/内建工具 allowlist(`--allowedTools`,只列既有合法只读工具);策略
    /// 角色(Planning/Review)追加 deny token 冻结片段。与投影/audit 同源
    /// (allowlist/approval 常量同 projection.rs)。direct `build_args` 保持
    /// 逐字节不变。
    fn build_lc_validated_args(
        &self,
        resume_provider_session_id: Option<&str>,
        tool_policy: Option<&crate::cross_cutting::streaming_provider::ProviderToolPolicy>,
    ) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            "--verbose".to_string(),
            "--output-format=stream-json".to_string(),
            "--input-format=stream-json".to_string(),
            "--include-partial-messages".to_string(),
            "--replay-user-messages".to_string(),
        ];

        if let Some(session_id) = resume_provider_session_id
            .map(str::trim)
            .filter(|session_id| !session_id.is_empty())
        {
            args.push("--resume".to_string());
            args.push(session_id.to_string());
        }

        // LC headless 工具 allowlist(冻结):MCP/内建工具控制进入 argv 投影,
        // 只列既有合法只读工具,不扩大到 MCP 写工具。
        args.push("--allowedTools".to_string());
        args.push(projection::CLAUDE_LC_ALLOWED_TOOLS.to_string());

        if let Some(crate::cross_cutting::streaming_provider::ProviderToolPolicy {
            intent:
                crate::cross_cutting::streaming_provider::ToolPolicyIntent::DenyFileWriteBuiltins,
        }) = tool_policy
        {
            args.extend(deny_file_write_builtins_tokens());
        }

        args.push("--permission-prompt-tool=stdio".to_string());

        args
    }

    fn parse_stream_text_delta(value: &Value) -> Option<String> {
        if value.get("type")?.as_str()? == "stream_event" {
            let event = value.get("event")?;
            if event.get("type")?.as_str()? == "content_block_delta" {
                let delta = event.get("delta")?;
                if delta.get("type")?.as_str()? == "text_delta" {
                    let text = delta.get("text")?.as_str()?;
                    if !text.is_empty() {
                        return Some(text.to_string());
                    }
                }
            }
        }
        None
    }

    fn parse_assistant_text(value: &Value) -> Option<String> {
        if value.get("type")?.as_str()? != "assistant" {
            return None;
        }

        let content = value.get("message")?.get("content")?.as_array()?;
        let text = content
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");

        if text.is_empty() { None } else { Some(text) }
    }

    fn assistant_text_delta(assistant_text: &str, emitted_text: &str) -> Option<String> {
        if assistant_text.is_empty() || assistant_text == emitted_text {
            return None;
        }
        if emitted_text.is_empty() {
            return Some(assistant_text.to_string());
        }
        if let Some(suffix) = assistant_text.strip_prefix(emitted_text) {
            if suffix.is_empty() {
                return None;
            }
            return Some(suffix.to_string());
        }
        if emitted_text.ends_with(assistant_text) {
            return None;
        }
        Some(assistant_text.to_string())
    }

    fn parse_tool_use_from_assistant(value: &Value) -> Option<Vec<ToolUseBlock>> {
        if value.get("type")?.as_str()? != "assistant" {
            return None;
        }
        let content = value.get("message")?.get("content")?.as_array()?;
        let tool_uses: Vec<ToolUseBlock> = content
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
            .filter_map(|item| {
                Some(ToolUseBlock {
                    id: item.get("id")?.as_str()?.to_string(),
                    name: item.get("name")?.as_str()?.to_string(),
                    input: item.get("input").cloned().unwrap_or(Value::Null),
                })
            })
            .collect();
        if tool_uses.is_empty() {
            None
        } else {
            Some(tool_uses)
        }
    }

    fn parse_tool_result(value: &Value) -> Option<Vec<ToolResultBlock>> {
        if value.get("type")?.as_str()? != "user" {
            return None;
        }
        let content = value.get("message")?.get("content")?.as_array()?;
        let results: Vec<ToolResultBlock> = content
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_result"))
            .filter_map(|item| {
                let tool_use_id = item.get("tool_use_id")?.as_str()?.to_string();
                let is_error = item
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let output = match item.get("content") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Array(arr)) => arr
                        .iter()
                        .filter_map(|block| block.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => String::new(),
                };
                Some(ToolResultBlock {
                    tool_use_id,
                    output,
                    is_error,
                })
            })
            .collect();
        if results.is_empty() {
            None
        } else {
            Some(results)
        }
    }

    fn parse_control_request(value: &Value) -> Option<ClaudePermissionRequest> {
        if value.get("type")?.as_str()? != "control_request" {
            return None;
        }

        let request = value.get("request")?;
        if request.get("subtype")?.as_str()? != "can_use_tool" {
            return None;
        }

        let input = request.get("input").unwrap_or(&Value::Null);
        let command = input.get("command").and_then(Value::as_str);
        let description = input
            .get("description")
            .and_then(Value::as_str)
            .or(command)
            .unwrap_or("Claude Code tool request");

        Some(ClaudePermissionRequest {
            request_id: value.get("request_id")?.as_str()?.to_string(),
            tool_use_id: value
                .get("tool_use_id")
                .or_else(|| request.get("tool_use_id"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            tool_name: request
                .get("tool_name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            description: description.to_string(),
            input: input.clone(),
        })
    }

    async fn write_control_response(
        stdin: &Arc<Mutex<ChildStdin>>,
        request_id: &str,
        approved: bool,
        reason: Option<String>,
    ) -> Result<(), ProviderAdapterError> {
        let behavior = if approved { "allow" } else { "deny" };
        let payload = Self::control_response_payload(
            request_id,
            json!({
                "behavior": behavior,
                "message": reason,
            }),
        );
        tool::write_json_line(stdin, &payload).await
    }

    async fn write_choice_control_response(
        stdin: &Arc<Mutex<ChildStdin>>,
        request_id: &str,
        original_input: &Value,
        answers: serde_json::Map<String, Value>,
    ) -> Result<(), ProviderAdapterError> {
        let mut updated_input = original_input.clone();
        if let Some(obj) = updated_input.as_object_mut() {
            obj.insert("answers".to_string(), Value::Object(answers));
        }
        let payload = Self::control_response_payload(
            request_id,
            json!({
                "behavior": "allow",
                "updatedInput": updated_input,
            }),
        );
        tool::write_json_line(stdin, &payload).await
    }

    fn control_response_payload(request_id: &str, response: Value) -> Value {
        json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": response,
            }
        })
    }

    async fn write_initial_messages(
        stdin: &Arc<Mutex<ChildStdin>>,
        input: &StreamingProviderInput,
    ) -> Result<(), ProviderAdapterError> {
        tool::write_json_line(
            stdin,
            &json!({
                "type": "control_request",
                "request": {
                    "subtype": "initialize",
                },
            }),
        )
        .await?;
        tool::write_json_line(
            stdin,
            &json!({
                "type": "control_request",
                "request": {
                    "subtype": "set_permission_mode",
                    "mode": permission_mode_for_claude(&input.permission_mode),
                },
            }),
        )
        .await?;
        tool::write_json_line(
            stdin,
            &json!({
                "type": "user",
                "message": {
                    "role": "user",
                    "content": input.prompt,
                },
            }),
        )
        .await
    }

    /// D③（诊断强化）：失败路径的 stderr 有界快照（≤2000B，UTF-8 字符边界
    /// 安全截断），供错误 details/stderr 携带 claude 子进程死因（MCP 连接错
    /// 误、`[claude-code:...]` 日志行等）。只读快照，不影响 stderr 任务回收。
    async fn bounded_stderr_snapshot(stderr_output: &Arc<Mutex<String>>) -> String {
        const CLAUDE_POLICY_STDERR_SNAPSHOT_MAX_BYTES: usize = 2000;
        let snapshot = stderr_output.lock().await.clone();
        if snapshot.len() <= CLAUDE_POLICY_STDERR_SNAPSHOT_MAX_BYTES {
            return snapshot;
        }
        // 保留尾部：致命错误（panic/MCP 失败等）通常在 stderr 末尾，超限时丢头部保尾部。
        let mut start = snapshot.len() - CLAUDE_POLICY_STDERR_SNAPSHOT_MAX_BYTES;
        while start < snapshot.len() && !snapshot.is_char_boundary(start) {
            start += 1;
        }
        snapshot[start..].to_string()
    }
}

/// 会话流收尾（策略与非策略路径共用）：读取 claude 流并处理终态（child
/// wait/kill 链、stderr 任务回收与终态事件）。
///
/// P1-7 round 2（controller 裁决）：策略路径把「子进程所有权+初始写入+握手+
/// provider_start 写入」留在 `start()` 内完成，成功后才把 child 与续读 reader
/// 移交本收尾任务；失败路径由 `start()` 直接持有 child 同步 `kill()`+`wait()`
/// 后返回错误（对齐 codex/pi 先例），本任务不再承担失败窗口的终止责任。
/// Task 9a:LC 启动的完整 resume 审计字面量(投影摘要 + v2 resume 材料)。
/// action evidence 摘要与 `capability_projection_digest` 同源(action row 的
/// projection_digest);resume 指纹为五参 `from_envelope` 全上下文计算。
pub(crate) fn lc_start_audit_literal(
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

/// B1-1 接线片:spawn 点把真实组长 PID 回填到本次 launch 的登记
/// (`backfill_spawned_identity` 读 `/proc/<pid>/stat` starttime 做 PID
/// 复用防护)。组长身份取自 spawn 时冻结的 `process_group(0)` leader。
fn backfill_launch_identity(
    ownership: &TrustedLaunchOwnership,
    reservation: &ResourceReservation,
    child: &ManagedProcessChild,
) -> Result<(), ProviderAdapterError> {
    let pid = child.spawn_leader_pid().ok_or_else(|| {
        ProviderAdapterError::provider_unavailable(
            "claude launch ownership: spawned child has no leader pid",
        )
    })?;
    ownership.backfill_spawned_identity(reservation, pid)
}

#[allow(clippy::too_many_arguments)]
async fn run_claude_session_tail(
    stdout_reader: impl tokio::io::AsyncRead + Unpin,
    stdin: Arc<Mutex<ChildStdin>>,
    bridge: ApprovalBridge,
    event_tx: mpsc::Sender<ProviderEvent>,
    cancel: CancellationToken,
    structured_output_contract: Option<StructuredOutputContract>,
    usage_role: &'static str,
    mut child: ManagedProcessChild,
    stderr_output: Arc<Mutex<String>>,
    stderr_task: tokio::task::JoinHandle<()>,
    ownership: Option<TrustedLaunchOwnership>,
) {
    // r19 根修+oracle 残留②收口(fix 轮 2):CLI 子进程退出必须终结事件流。
    // stdout EOF 可能被「同进程组内仍存活、继承了管道写端的进程」无限推迟
    // (CLI 工具子进程形态),泵读取与 child.wait() 经共享 exit-watch
    // supervisor 竞速:子进程先退出时对进程组补 SIGKILL 释放管道持有者,再
    // 给泵有界排空窗口(正常完成的 result 行已在管道缓冲内);窗口耗尽以
    // 「子进程先于泵终态退出」合成错误 fail-closed(→ Failed 事件 → 同步桥
    // 秒级返回),不依赖外层 stage 超时兜底。
    let result = match crate::cross_cutting::process_manager::run_session_tail_with_exit_watch(
        stream::read_claude_stream(
            stdout_reader,
            stdin,
            bridge,
            event_tx.clone(),
            cancel,
            structured_output_contract,
            usage_role,
        ),
        &mut child,
        crate::cross_cutting::process_manager::PROVIDER_CHILD_EXIT_STREAM_DRAIN,
        crate::cross_cutting::process_manager::provider_child_exit_error,
    )
    .await
    {
        crate::cross_cutting::process_manager::SessionTailExit::Stream(result) => result,
        crate::cross_cutting::process_manager::SessionTailExit::ChildExitBeforeTerminal {
            error,
        } => Err(error),
    };
    match result {
        Ok(ClaudeStreamOutcome::Aborted) => {
            stderr_task.abort();
            stream::terminate_aborted_child(&mut child).await;
            let _ = stderr_task.await;
        }
        Ok(outcome) => {
            // oracle 残留②:终态 join 一律有界(红线 a)——stdout 已 EOF 而子
            // 进程未死、或仅 stderr 被组内进程持有时,无界 wait/stderr join
            // 会推迟乃至吞掉 EofWithoutResult 的终态 Failed。组清理后 join,
            // 窗口耗尽放弃 join(fail-closed 文案沿用 bounded 形态)。
            let status = match crate::cross_cutting::process_manager::bounded_child_wait(
                &mut child,
                crate::cross_cutting::process_manager::PROVIDER_CHILD_EXIT_STREAM_DRAIN,
            )
            .await
            {
                Some(status) => Ok(status),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "bounded child join elapsed before exit status",
                )),
            };
            let _ = crate::cross_cutting::process_manager::bounded_task_join(
                stderr_task,
                crate::cross_cutting::process_manager::PROVIDER_CHILD_EXIT_STREAM_DRAIN,
            )
            .await;
            if outcome == ClaudeStreamOutcome::EofWithoutResult {
                let stderr = stderr_output.lock().await.clone();
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "provider".to_string(),
                        kind: ProviderExecutionEventKind::Provider,
                        status: ProviderExecutionEventStatus::Failed,
                        title: "Claude Code provider failed".to_string(),
                        detail: Some("exited without result".to_string()),
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
                        message: tool::format_exit_failure(status, stderr),
                    })
                    .await;
            }
        }
        Err(error) => {
            let _ = child.start_kill();
            let _ = event_tx
                .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                .await;
            let _ = event_tx
                .send(ProviderEvent::Execution(ProviderExecutionEvent {
                    event_id: "provider".to_string(),
                    kind: ProviderExecutionEventKind::Provider,
                    status: ProviderExecutionEventStatus::Failed,
                    title: "Claude Code provider failed".to_string(),
                    detail: Some(error.details.clone()),
                    command: None,
                    cwd: None,
                    output: None,
                    exit_code: None,
                }))
                .await;
            let _ = event_tx
                .send(ProviderEvent::Failed {
                    message: error.details,
                })
                .await;
            let _ = crate::cross_cutting::process_manager::bounded_child_wait(
                &mut child,
                crate::cross_cutting::process_manager::PROVIDER_CHILD_EXIT_STREAM_DRAIN,
            )
            .await;
            let _ = crate::cross_cutting::process_manager::bounded_task_join(
                stderr_task,
                crate::cross_cutting::process_manager::PROVIDER_CHILD_EXIT_STREAM_DRAIN,
            )
            .await;
        }
    }
    // B1-1:会话真实收口(子进程退出/进程组静止/取消收口完成)后注销——
    // **不在协议 Completed 点注销**;静止不可证时保留 durable 记录,
    // 由 scope 级 quiesce/恢复驱动停等接管。
    if let Some(ownership) = ownership {
        ownership.release_after_session_tail().await;
    }
}

/// 冻结 trait 合同面:raw/validated 两路都转交 inherent 执行体,使 B1-1
/// 受信归属能在**同一 spawn 点**完成预留→spawn→回填→收口注销。
#[async_trait::async_trait]
impl StreamingProviderAdapter for ClaudeCodeProvider {
    /// direct/legacy 直调:无归属请求(零登记),行为与既有裸 start 一致。
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.start_inner(input, cancel, None).await
    }

    /// B1-1(波 2):raw start 的受信归属入口。与 trait 默认方法同义——
    /// `None` = 无归属请求(保持裸 start 语义);`Some` = 预留(失败零
    /// spawn)→ spawn → 回填真实 PID+starttime → 会话真实收口后注销。
    async fn start_owned(
        &self,
        input: StreamingProviderInput,
        ownership: Option<TrustedLaunchOwnership>,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        match ownership {
            None => self.start(input, cancel).await,
            Some(ownership) => self.start_inner(input, cancel, Some(ownership)).await,
        }
    }

    /// LC validated 启动:载体随 validated 输入的 crate 私有绑定进入
    /// inherent 执行体的 spawn 点(同一预留/回填/收口注销生命周期)。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.start_validated_inner(validated, cancel).await
    }

    /// Task 3.2(B0a2):LC 只读 facts 观测(执行体在 inherent impl)。
    async fn prepare_lc_facts(
        &self,
        input: &StreamingProviderInput,
        launch: &crate::product::logical_codebase::provider_gateway::ValidatedSessionLaunchPolicy,
        cancel: CancellationToken,
    ) -> Result<
        crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts,
        ProviderAdapterError,
    > {
        self.prepare_lc_facts_inner(input, launch, cancel).await
    }
}
