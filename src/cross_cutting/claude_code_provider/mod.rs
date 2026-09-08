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
use crate::cross_cutting::streaming_provider::{
    ChoiceAnswerData, ChoiceOptionData, ChoiceQuestionData, ChoiceRequestData, ChoiceRequestSource,
    ProviderEvent, ProviderExecutionEvent, ProviderExecutionEventKind,
    ProviderExecutionEventStatus, ProviderPermissionMode, ProviderSession, ProviderStatus,
    ProviderVersionSupplier, RiskLevel, StreamingProviderAdapter, StreamingProviderInput,
    UsageReportData, canonical_tool_policy, validate_tool_policy_for_role,
};
use crate::cross_cutting::structured_output::StructuredOutputContract;
use crate::cross_cutting::tool_policy_audit::{
    DurableToolPolicyEvent, ProviderStartAudit, ToolPolicyAuditSink,
};

mod ask_user_question;
mod stream;
mod tool;

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
    use std::collections::BTreeMap;
    use tokio_util::sync::CancellationToken;

    let request = BoundedCommandRequest {
        executable: command.to_string_lossy().into_owned(),
        argv: vec!["--version".to_string()],
        working_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
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
        }
    }

    /// 注入策略会话 provider 版本 supplier（fixture/测试 seam）。
    pub fn with_version_supplier(mut self, supplier: ProviderVersionSupplier) -> Self {
        self.version_supplier = Some(supplier);
        self
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
        eprintln!(
            "[aria-choice-diag] claude writing control_response request_id={} answer_keys={:?}",
            request_id,
            answers.keys().cloned().collect::<Vec<_>>()
        );
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
) {
    let result = stream::read_claude_stream(
        stdout_reader,
        stdin,
        bridge,
        event_tx.clone(),
        cancel,
        structured_output_contract,
        usage_role,
    )
    .await;
    match result {
        Ok(ClaudeStreamOutcome::Aborted) => {
            stderr_task.abort();
            stream::terminate_aborted_child(&mut child).await;
            let _ = stderr_task.await;
        }
        Ok(outcome) => {
            let status = child.wait().await;
            let _ = stderr_task.await;
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
            let _ = child.wait().await;
            let _ = stderr_task.await;
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ClaudeCodeProvider {
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
        // 策略上下文（Task 3.2/3.3，spawn 前）：版本解析（supplier seam 优先，默认
        // 真实 `--version` 探测+进程内缓存，不可得 fail-closed）与 resume 冻结三元组
        // 比对（记录缺失或 digest/version/dialect 任一不一致 → 追加 superseded 终止
        // 审计并新建会话）。
        // GC9：resume 记录缺失与 drift 同路径处置（清除 resume id、全新会话）；
        // 「标记 superseded」仅带内 ToolPolicyWarning（🔴 无旧文件可写，不伪造
        // durable 文件），在事件通道建立后送出。
        let mut superseded_record_missing = false;
        let mut policy_context: Option<(
            std::sync::Arc<dyn ToolPolicyAuditSink>,
            String,
            String,
            String,
        )> = None;
        if let Some(policy) = input.tool_policy.as_ref() {
            let sink = input.audit_sink.clone().ok_or_else(|| {
                ProviderAdapterError::parse_error(
                    "claude policy session: audit sink is required for policy sessions",
                    String::new(),
                    String::new(),
                )
            })?;
            let provider_version = match self.version_supplier.clone() {
                Some(supplier) => supplier().map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("claude policy session: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?,
                None => crate::cross_cutting::streaming_provider::cached_cli_version(
                    &self.command,
                    probe_claude_version(&self.command, CLAUDE_VERSION_PROBE_TIMEOUT),
                )
                .await
                .map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("claude policy session: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?,
            };
            let canonical =
                canonical_tool_policy(TOOL_POLICY_PROVIDER_NAME, policy).map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("claude policy session: {error}"),
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
                        format!("claude policy session: resume lookup failed: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?;
                let current = ProviderStartAudit {
                    workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
                    provider_session_id: resume_id.clone(),
                    tool_policy_canonical_digest: canonical.digest.clone(),
                    provider_version: provider_version.clone(),
                    adapter_dialect: CLAUDE_POLICY_DIALECT.to_string(),
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
                                "claude policy session: superseded audit append failed: {error}"
                            ),
                            String::new(),
                            String::new(),
                        )
                    })?;
                    input.resume_provider_session_id = None;
                } else if stored.is_none() {
                    // GC9：记录缺失与 drift 同路径处置——清除 resume id、以全新会话
                    // （fresh init 握手 + 新 run 的 provider_start）启动；「标记 superseded」
                    // 仅带内 ToolPolicyWarning：无旧文件可写，🔴 不得伪造无
                    // provider_start 首行的 durable 文件（首行不变量优先）。
                    input.resume_provider_session_id = None;
                    superseded_record_missing = true;
                }
            }
            // P1-8：durable 审计 role 保留真实 AdapterRole 序列化值（usage 展示层
            // 归一仅在 read_claude_stream 的 usage 上报使用）。
            let role_text =
                crate::cross_cutting::streaming_provider::adapter_role_text(&input.role)
                    .to_string();
            policy_context = Some((sink, provider_version, canonical.digest, role_text));
        }
        let args = self.build_args(
            input.resume_provider_session_id.as_deref(),
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

        let stdin = Arc::new(Mutex::new(process.stdin));
        let stdout = process.stdout;
        let stderr = process.stderr;
        let mut child = process.child;
        let (event_tx, event_rx) = mpsc::channel(32);
        let bridge = ApprovalBridge::new(input.permission_mode.clone(), event_tx.clone());
        let commands = bridge.command_sender();
        let structured_output_contract = input.structured_output_contract.clone();

        let resume_native_id = if input.tool_policy.is_some() {
            input
                .resume_provider_session_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
        } else {
            None
        };

        let _ = event_tx
            .send(ProviderEvent::StatusChanged(ProviderStatus::Starting))
            .await;
        let _ = event_tx
            .send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "provider".to_string(),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Started,
                title: "Claude Code provider started".to_string(),
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

        // workspace 会话 id（D6 冻结字段）在 input 移入后台任务前捕获，供
        // provider_start 审计落盘使用。
        let workspace_session_id = input.workspace_session_id.clone().unwrap_or_default();

        // 策略会话（P1-7 round 2 裁决）：「子进程所有权+初始写入+握手+provider_start
        // 写入」留在 start() 内有界完成（握手出后台任务的重构即要求本身）；成功后
        // 才把 child 移交会话收尾任务；任一失败由 start() 直接持有 child 同步
        // kill()+wait()（exit status 回收）后返回错误（对齐 codex/pi 先例）；
        // 有界 await 仅作 stderr 任务善后。事件通道随会话未返回而消亡，失败路径
        // 不再投递死信事件。
        if let Some((sink, provider_version, tool_policy_digest, role_text)) = policy_context {
            let stderr_output = Arc::new(Mutex::new(String::new()));
            let stderr_output_for_task = Arc::clone(&stderr_output);
            let stderr_task = tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut output = stderr_output_for_task.lock().await;
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&line);
                }
            });

            // 有界完成「初始写入→Running→握手→provider_start」：初始 stdin 写不可
            // 取消（不 select cancel，如 task.rs write_json_line 的 write_all），
            // 以外层超时放弃 future 并走 kill 链。
            let bound = stream::CLAUDE_POLICY_HANDSHAKE_TIMEOUT.saturating_mul(3);
            let outcome = tokio::time::timeout(bound, async {
                Self::write_initial_messages(&stdin, &input)
                    .await
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!(
                                "claude policy session: initial write failed: {}",
                                error.details
                            ),
                            String::new(),
                            String::new(),
                        )
                    })?;
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Running))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "turn".to_string(),
                        kind: ProviderExecutionEventKind::Turn,
                        status: ProviderExecutionEventStatus::Started,
                        title: "Turn started".to_string(),
                        detail: None,
                        command: None,
                        cwd: Some(input.working_dir.display().to_string()),
                        output: None,
                        exit_code: None,
                    }))
                    .await;
                let (reader, native_id) = match resume_native_id.clone() {
                    // resume 已知：native id 即 resume id，不等 init。
                    Some(id) => (tokio::io::BufReader::new(stdout), id),
                    None => {
                        let (reader, session_id) = stream::wait_for_claude_init(stdout, &cancel)
                            .await
                            .map_err(|error| {
                                ProviderAdapterError::parse_error(
                                    format!(
                                        "claude policy session: handshake failed: {}",
                                        error.details
                                    ),
                                    String::new(),
                                    String::new(),
                                )
                            })?;
                        (reader, session_id)
                    }
                };
                let audit_event = DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                    provider: TOOL_POLICY_PROVIDER_NAME.to_string(),
                    role: role_text.clone(),
                    workspace_session_id: workspace_session_id.clone(),
                    provider_session_id: native_id.clone(),
                    tool_policy_canonical_digest: tool_policy_digest.clone(),
                    argv: args.clone(),
                    sandbox: None,
                    approval_policy: None,
                    provider_version: provider_version.clone(),
                    adapter_dialect: CLAUDE_POLICY_DIALECT.to_string(),
                });
                // F2（最终审）：审计写入前再验 provider_session_id 非空——空白
                // 原生会话 id 不得进入 durable 审计（fresh/resume 两路同验）。
                if native_id.trim().is_empty() {
                    return Err(ProviderAdapterError::parse_error(
                        "claude policy session: native session id is blank before audit write",
                        String::new(),
                        String::new(),
                    ));
                }
                sink.append_bound(audit_event).map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!(
                            "claude policy session: provider_start audit append failed: {error}"
                        ),
                        String::new(),
                        String::new(),
                    )
                })?;
                Ok::<_, ProviderAdapterError>((reader, native_id))
            })
            .await;

            let (reader, native_id) = match outcome {
                Ok(Ok(prepared)) => prepared,
                Ok(Err(mut error)) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    let _ = tokio::time::timeout(bound, stderr_task).await;
                    // D③（诊断强化）：失败附 stderr——claude 子进程死因可见，
                    // 有界快照（≤2000B）同时并入 details 与 stderr 字段。
                    let stderr_snapshot = Self::bounded_stderr_snapshot(&stderr_output).await;
                    if !stderr_snapshot.is_empty() {
                        if error.stderr.is_empty() {
                            error.stderr = stderr_snapshot.clone();
                        }
                        error.details.push_str(&format!(
                            "\nclaude stderr (last {} bytes): {stderr_snapshot}",
                            stderr_snapshot.len()
                        ));
                    }
                    return Err(error);
                }
                Err(_elapsed) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    let _ = tokio::time::timeout(bound, stderr_task).await;
                    // D③：同上——超时路径也携带 stderr 快照。
                    let stderr_snapshot = Self::bounded_stderr_snapshot(&stderr_output).await;
                    let details = if stderr_snapshot.is_empty() {
                        "provider command timed out".to_string()
                    } else {
                        format!(
                            "provider command timed out\nclaude stderr (last {} bytes): {stderr_snapshot}",
                            stderr_snapshot.len()
                        )
                    };
                    return Err(ProviderAdapterError::timeout_with_details(
                        details,
                        String::new(),
                        stderr_snapshot,
                        stream::CLAUDE_POLICY_HANDSHAKE_TIMEOUT.as_millis() as u64,
                    ));
                }
            };
            // 成功：child 与续读 reader 移交会话收尾任务（流读取+终态处理）。
            let usage_role = UsageReportData::role_text(&input.role);
            tokio::spawn(async move {
                run_claude_session_tail(
                    reader,
                    stdin,
                    bridge,
                    event_tx,
                    cancel,
                    structured_output_contract,
                    usage_role,
                    child,
                    stderr_output,
                    stderr_task,
                )
                .await;
            });
            return Ok(ProviderSession {
                native_session_id: Some(native_id),
                events: event_rx,
                commands,
            });
        }

        // 非策略/Coder 路径：既有行为零变化——初始写入与流读取由会话任务完成，
        // 失败终止与终态事件沿用任务内 kill 链（child 归任务持有）。
        tokio::spawn(async move {
            let stderr_output = Arc::new(Mutex::new(String::new()));
            let stderr_output_for_task = Arc::clone(&stderr_output);
            let stderr_task = tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut output = stderr_output_for_task.lock().await;
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&line);
                }
            });

            if let Err(error) = Self::write_initial_messages(&stdin, &input).await {
                let _ = child.start_kill();
                let status = child.wait().await;
                let _ = stderr_task.await;
                let stderr = tool::combine_stderr(stderr_output.lock().await.clone(), error.stderr);
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "provider".to_string(),
                        kind: ProviderExecutionEventKind::Provider,
                        status: ProviderExecutionEventStatus::Failed,
                        title: "Claude Code provider failed".to_string(),
                        detail: Some(error.details),
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
                return;
            }
            let _ = event_tx
                .send(ProviderEvent::StatusChanged(ProviderStatus::Running))
                .await;
            let _ = event_tx
                .send(ProviderEvent::Execution(ProviderExecutionEvent {
                    event_id: "turn".to_string(),
                    kind: ProviderExecutionEventKind::Turn,
                    status: ProviderExecutionEventStatus::Started,
                    title: "Turn started".to_string(),
                    detail: None,
                    command: None,
                    cwd: Some(input.working_dir.display().to_string()),
                    output: None,
                    exit_code: None,
                }))
                .await;

            // 非策略路径：无策略握手，直接续读流；终态处理交共用收尾函数。
            run_claude_session_tail(
                tokio::io::BufReader::new(stdout),
                stdin,
                bridge,
                event_tx,
                cancel,
                structured_output_contract,
                UsageReportData::role_text(&input.role),
                child,
                stderr_output,
                stderr_task,
            )
            .await;
        });

        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands,
        })
    }
}
