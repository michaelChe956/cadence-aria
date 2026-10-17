use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::local_usage::read_default_codex_usage;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ChoiceRequestData, ChoiceRequestSource, CodexApprovalCategory, CodexApprovalDecisionEvent,
    CodexApprovalResponse, CodexProtocolWarningEvent, CodexSessionTerminatedEvent,
    ProviderCompletion, ProviderEvent, ProviderExecutionEvent, ProviderExecutionEventKind,
    ProviderExecutionEventStatus, ProviderPermissionMode, ProviderStatus, RiskLevel,
    StreamingProviderInput, UsageReportData,
};
use crate::cross_cutting::tool_policy_audit::{
    ApprovalDecisionAudit, DurableToolPolicyEvent, ProtocolWarningAudit, SessionTerminatedAudit,
    ToolPolicyAuditSink,
};

use super::{
    CODEX_DEFAULT_SANDBOX_MODE, CODEX_RESUME_STALL_ERROR, CODEX_RESUME_STALL_TIMEOUT,
    CODEX_RPC_REQUEST_TIMEOUT, emit_request_user_input_protocol_error, is_turn_completed,
    parse_agent_message_text, parse_approval_request, parse_codex_usage, parse_execution_event,
    parse_failure, parse_file_change_summary, parse_user_input_request, provider_error,
    send_provider_event, write_approval_decision, write_approval_response,
    write_user_input_response,
};

const CODEX_EMPTY_OUTPUT_ERROR: &str = "provider_empty_output";
const CODEX_EMPTY_OUTPUT_RETRY_PROMPT: &str =
    "Your previous reply was empty. Please reply again with your complete output.";

/// codex 在 tool-policy canonical 序列中的 provider 名（CLI 名常量）。
pub const TOOL_POLICY_PROVIDER_NAME: &str = "codex";

/// DenyFileWriteBuiltins（Author 岗位纪律）的 codex direct 冻结片段（T13
/// 替换旧 read-only canonical）：`sandbox=danger-full-access` +
/// `approvalPolicy=on-request`，argv/参数原文按出现顺序、大小写保留。
/// 纪律是**检测型**（fileChange 事实 → 事件 → durable → 中断 → 本轮
/// Failed），不以 FileChange 审批 Decline 作强制防写；LC 侧维护独立冻结
/// 序列（`projection::CODEX_LC_WIRE_CONTROL_TOKENS`），不与本片段复用。
pub fn deny_file_write_builtins_tokens() -> Vec<String> {
    vec![
        "sandbox=danger-full-access".to_string(),
        "approvalPolicy=on-request".to_string(),
    ]
}

/// thread/start 与 thread/resume 共用的启动参数单一来源（T13 冻结）：
/// - 岗位纪律 input（`tool_policy.is_some()`）给出 `sandbox:"danger-full-access"`
///   与 `approvalPolicy:"on-request"`（danger 基线 + 检测型岗位纪律；审批走
///   分类规则，危险命令 Accept 与 Claude/Pi 对齐，不用 never 自禁）；
/// - Coder/Reviewer（`None`）保持 `danger-full-access` 与既有 permission mode
///   映射（Auto→never / Supervised→on-request，原生 approval）。
pub(crate) fn codex_launch_params(input: &StreamingProviderInput) -> serde_json::Value {
    // 批2 Task 20:approvalPolicy 只按岗位纪律会话(Author deny 投影)恒
    // on-request;ReviewerVerify 窗口合同/其余合同 run 走原生权限档映射
    //（Reviewer/Coder danger 原生 approval,不因携带合同而收紧）。
    let discipline_session = input.tool_policy.as_ref().is_some_and(|policy| {
        matches!(
            policy.intent,
            crate::cross_cutting::streaming_provider::ToolPolicyIntent::DenyFileWriteBuiltins
        )
    });
    if discipline_session {
        json!({
            "cwd": input.working_dir.clone(),
            "approvalPolicy": "on-request",
            "sandbox": CODEX_DEFAULT_SANDBOX_MODE,
        })
    } else {
        json!({
            "cwd": input.working_dir.clone(),
            "approvalPolicy": match input.permission_mode {
                ProviderPermissionMode::Auto => "never",
                ProviderPermissionMode::Supervised => "on-request",
            },
            "sandbox": CODEX_DEFAULT_SANDBOX_MODE,
        })
    }
}

/// LC 全相位 danger 启动参数(T12,REQ-LCG-04 改版):thread/start 与
/// thread/resume 共用,只能来自已投影的 `CodexSandboxProjection`
/// (sandbox 恒 danger-full-access;approval=独立映射值;`cwd`=投影冻结的
/// 协议 cwd,Coding=target,只读=root),不以 raw 输入的 working_dir 覆盖。
/// direct `codex_launch_params` 原签名/分支保留,仅供 direct 启动消费。
pub(crate) fn codex_lc_launch_params(
    lc: &super::projection::CodexSandboxProjection,
) -> serde_json::Value {
    json!({
        "cwd": lc.protocol_cwd(),
        "approvalPolicy": lc.approval_policy(),
        "sandbox": lc.mode().wire_text(),
    })
}

/// LC 会话握手(Task 5):initialize/initialized + thread/resume|thread/start,
/// 启动参数来自受限投影(与 direct 握手共用 thread/start|resume 流程与
/// F1 resume id 确认语义;thread/start 与 thread/resume 共享同投影)。
pub(crate) async fn codex_lc_session_handshake<W>(
    peer: &JsonRpcPeer<W>,
    input: &StreamingProviderInput,
    lc: &super::projection::CodexSandboxProjection,
) -> Result<CodexSessionHandshake, ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    codex_handshake_with_launch_params(peer, input, codex_lc_launch_params(lc)).await
}

/// 岗位纪律会话即时审批决策（T13 定稿）：commandExecution=Accept（danger
/// 基线下危险命令与 Claude/Pi 对齐，不用 never 产生跨 provider 不对称）、
/// MCP=Accept；fileChange 在会话循环由检测型纪律先行处置（事件→durable→
/// 中断→本轮 Failed），此处 Decline 仅作 fail-closed 兜底，不作强制防写
/// 依据。决策以结构化事件形式暴露（`CodexApprovalDecisionEvent`）。
pub(crate) fn decide_for_policy(category: CodexApprovalCategory) -> CodexApprovalResponse {
    match category {
        CodexApprovalCategory::McpToolCall | CodexApprovalCategory::CommandExecution => {
            CodexApprovalResponse::Accept
        }
        // 未知形态在 session 层由 decide_unknown 先行处理；此处 fail-closed 拒绝。
        CodexApprovalCategory::FileChange | CodexApprovalCategory::Unknown { .. } => {
            CodexApprovalResponse::Decline
        }
    }
}

/// 纪律决策的原因码（D7 `approval_decision.reason_code` 冻结枚举）：决策归属的
/// 策略语义，不读自然语言 reason。
pub(crate) fn policy_decision_reason_code(category: CodexApprovalCategory) -> &'static str {
    match category {
        CodexApprovalCategory::McpToolCall => "policy_allows_mcp",
        CodexApprovalCategory::CommandExecution => "policy_allows_command",
        // fileChange 的真实处置是检测型中断（author_file_change_detected）；
        // 该原因码仅在兜底决策路径出现。
        CodexApprovalCategory::FileChange => {
            crate::cross_cutting::tool_policy_audit::AUTHOR_FILE_CHANGE_DETECTED
        }
        CodexApprovalCategory::Unknown { .. } => "unsupported_approval_kind",
    }
}

/// 策略事件双路径出口（Task 3.2）：结构化事件先经事件通道送出会话循环（可观测），
/// 再经 run-bound sink durable 落盘（GC11）；append 失败返回错误，沿既有 kill 链
/// 终止会话并将 run 判失败。
fn audit_policy_event(
    sink: Option<&std::sync::Arc<dyn ToolPolicyAuditSink>>,
    event: DurableToolPolicyEvent,
) -> Result<(), ProviderAdapterError> {
    if let Some(sink) = sink {
        sink.append_bound(event).map_err(|error| {
            provider_error(format!("codex policy audit append failed: {error}"))
        })?;
    }
    Ok(())
}

/// 会话 tool-policy canonical digest（决策审计归属指纹，D7 `policy_digest`）：
/// LC 会话用启动侧已算的 LC 冻结 digest（与 provider_start 同源，批2
/// Task 20/RevB2T13 P3——不同序列不可等值 join）；direct 会话从 input
/// 策略确定性重算（GC8）。
fn policy_digest_for_input(
    input: &crate::cross_cutting::streaming_provider::StreamingProviderInput,
    lc_override: Option<&str>,
) -> String {
    if let Some(digest) = lc_override.map(str::trim).filter(|digest| !digest.is_empty()) {
        return digest.to_string();
    }
    input
        .tool_policy
        .as_ref()
        .and_then(|policy| {
            crate::cross_cutting::streaming_provider::canonical_tool_policy(
                TOOL_POLICY_PROVIDER_NAME,
                policy,
            )
            .map(|canonical| canonical.digest)
            .ok()
        })
        .unwrap_or_default()
}

/// Author 检测型岗位纪律（T13）：事件流检得 fileChange 事实（item 通知，
/// 无审批请求亦然；或 fileChange 审批请求）→ 双路径出口后中断当前 turn。
/// 次序冻结：①事件通道 `ToolPolicyWarning(author_file_change_detected)` →
/// ②durable `file_change_detected`（含处置事实；append 失败返回该错误，
/// 沿既有 kill 链 fail-closed）→ ③调用方以其返回值中断 turn → ④supervisor
/// kill child 并终态 Failed——本轮输出不可采信。🔴 不宣称回滚已发生写入或
/// terminal/MCP 预防（danger 基线下这是检测/对账合同，不是物理防写）。
async fn fail_turn_on_author_file_change(
    event_tx: &mpsc::Sender<ProviderEvent>,
    cancel: &CancellationToken,
    sink: Option<&std::sync::Arc<dyn ToolPolicyAuditSink>>,
    input: &StreamingProviderInput,
    item_id: &str,
    summary: &str,
    source_method: &str,
    lc_policy_digest: Option<&str>,
) -> ProviderAdapterError {
    let warning = CodexProtocolWarningEvent {
        reason_code: crate::cross_cutting::tool_policy_audit::AUTHOR_FILE_CHANGE_DETECTED
            .to_string(),
        method: source_method.to_string(),
        occurrence: 1,
    };
    if let Err(error) =
        send_provider_event(event_tx, ProviderEvent::ToolPolicyWarning(warning), cancel).await
    {
        return error;
    }
    if let Err(error) = audit_policy_event(
        sink,
        DurableToolPolicyEvent::FileChangeDetected(
            crate::cross_cutting::tool_policy_audit::FileChangeDetectedAudit {
                item_id: item_id.to_string(),
                summary: summary.to_string(),
                source_method: source_method.to_string(),
                disposition:
                    crate::cross_cutting::tool_policy_audit::AUTHOR_FILE_CHANGE_DISPOSITION
                        .to_string(),
                policy_digest: policy_digest_for_input(input, lc_policy_digest),
            },
        ),
    ) {
        return error;
    }
    tracing::warn!(
        target: "codex_provider",
        item_id,
        source_method,
        "author fileChange detected; interrupting turn and failing the run"
    );
    provider_error(format!(
        "codex session terminated: {}",
        crate::cross_cutting::tool_policy_audit::AUTHOR_FILE_CHANGE_DETECTED
    ))
}

/// 未知审批形态的确定性应答（GC6）：未知 elicitation 返回 JSON-RPC error
/// `-32601` 并带 data（不静默）；未知 item 返回 `{{"decision":"decline"}}`。
/// `reason` 不作为分类依据，仅出现在应答 data 的固定枚举 `unsupported_approval_kind`。
pub(crate) fn decide_unknown(method: &str, occurrence: u32) -> CodexApprovalResponse {
    tracing::debug!(
        target: "codex_provider",
        method,
        occurrence,
        "unclassified codex approval form"
    );
    if method == "mcpServer/elicitation/request" {
        CodexApprovalResponse::ElicitationError {
            code: -32601,
            data: serde_json::json!({
                "codex_approval_kind": "unknown",
                "reason": "unsupported_approval_kind",
            }),
        }
    } else {
        CodexApprovalResponse::Decline
    }
}

/// 同一会话连续未知形态风暴阈值：`>= 3` 次终止并记录
/// `reason_code=unknown_approval_storm`（GC6）。
pub(crate) fn unknown_storm_reason_after(occurrence: u32) -> Option<&'static str> {
    (occurrence >= 3).then_some("unknown_approval_storm")
}

async fn start_codex_turn<W>(
    peer: &JsonRpcPeer<W>,
    thread_id: &str,
    prompt: &str,
) -> Result<String, ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let turn_response = peer
        .request_with_timeout(
            json!({
                "jsonrpc": "2.0",
                "method": "turn/start",
                "params": {
                    "threadId": thread_id,
                    "input": [
                        {
                            "type": "text",
                            "text": prompt,
                        }
                    ],
                },
            }),
            CODEX_RPC_REQUEST_TIMEOUT,
        )
        .await?;
    Ok(turn_response
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .unwrap_or("turn")
        .to_string())
}

/// 既有会话入口（非策略/直接调用）：自行握手后进入会话循环。
pub(crate) async fn run_codex_session<W>(
    peer: JsonRpcPeer<W>,
    bridge: ApprovalBridge,
    event_tx: mpsc::Sender<ProviderEvent>,
    input: StreamingProviderInput,
    cancel: CancellationToken,
) -> Result<(), ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let handshake = codex_session_handshake(&peer, &input).await?;
    run_codex_session_loop(peer, bridge, event_tx, input, cancel, handshake, None).await
}

/// codex 的 adapter dialect 常量（GC9 冻结：`codex-app-server-rpc`）。
pub const CODEX_POLICY_DIALECT: &str = "codex-app-server-rpc";

/// 会话握手结果：resume 原生 thread id 语义 + 协商出的 thread id。fresh
/// `thread/start` 协议未返回非空 id 时为 `None`（与非策略历史行为一致；策略路径
/// 要求必得 id，否则 fail-closed）；resume 则必须由 `thread/resume` 应答确认
/// 与请求一致的 id，否则握手直接失败（F1 最终审：不得田请求 id 冒充确认）。
#[derive(Debug, Clone)]
pub(crate) struct CodexSessionHandshake {
    pub(crate) resume_session_id: Option<String>,
    pub(crate) thread_id: Option<String>,
}

/// 握手应答中的原生 thread id（F1 最终审）：仅当应答明确返回非空字符串 id
/// （`/thread/id` 或 `/id`）时为 `Some`；缺失/全空白/类型错误（非字符串）均为
/// `None`——请求方携带的旧 id 不得回退冒充应答确认。
fn response_thread_id(response: &Value) -> Option<String> {
    response
        .pointer("/thread/id")
        .or_else(|| response.pointer("/id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToString::to_string)
}

/// codex 会话握手（direct）：`initialize`/`initialized` +
/// `thread/resume`|`thread/start`，由 `run_codex_session`（非策略/直接调用）
/// 与 `CodexProvider::start`（策略路径，有界前置）共用。启动参数始终经
/// `codex_launch_params`（三联动冻结）。
pub(crate) async fn codex_session_handshake<W>(
    peer: &JsonRpcPeer<W>,
    input: &StreamingProviderInput,
) -> Result<CodexSessionHandshake, ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    codex_handshake_with_launch_params(peer, input, codex_launch_params(input)).await
}

/// 握手共享内核（Task 5）：initialize/initialized + thread/resume|thread/start
/// 的流程与 F1 resume id 确认语义在 direct/LC 两条路径间唯一；唯一差异是
/// 启动参数来源（direct=`codex_launch_params(input)`；LC=受限投影 params，
/// thread/start 与 thread/resume 共享同投影）。
async fn codex_handshake_with_launch_params<W>(
    peer: &JsonRpcPeer<W>,
    input: &StreamingProviderInput,
    launch_params: serde_json::Value,
) -> Result<CodexSessionHandshake, ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let _ = peer
        .request_with_timeout(
            json!({
                "jsonrpc": "2.0",
                "method": "initialize",
                "params": {
                    "clientInfo": {
                        "name": "cadence-aria",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                },
            }),
            CODEX_RPC_REQUEST_TIMEOUT,
        )
        .await?;
    peer.send(json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {},
    }))
    .await?;

    let resume_session_id = input
        .resume_provider_session_id
        .as_deref()
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
        .map(ToString::to_string);

    let thread_id = if let Some(session_id) = resume_session_id.as_deref() {
        let mut resume_params = launch_params;
        resume_params
            .as_object_mut()
            .expect("codex launch params are a JSON object")
            .insert("threadId".to_string(), json!(session_id));
        let resume_response = peer
            .request_with_timeout(
                json!({
                    "jsonrpc": "2.0",
                    "method": "thread/resume",
                    "params": resume_params,
                }),
                CODEX_RPC_REQUEST_TIMEOUT,
            )
            .await?;
        // F1（最终审）：resume 的原生 thread id 只能来自 provider 应答本身——
        // 缺失/全空白/类型错误，或与请求 id 不一致，均为握手失败（调用侧沿
        // kill 链同步终止子进程）；不得回退请求中的旧 session id 冒充确认。
        match response_thread_id(&resume_response) {
            Some(id) if id == session_id => Some(id),
            _ => {
                return Err(provider_error(format!(
                    "codex thread/resume response did not confirm the requested thread id \
                     {session_id} (missing, blank, non-string, or mismatched thread id)"
                )));
            }
        }
    } else {
        let thread_response = peer
            .request_with_timeout(
                json!({
                    "jsonrpc": "2.0",
                    "method": "thread/start",
                    "params": launch_params,
                }),
                CODEX_RPC_REQUEST_TIMEOUT,
            )
            .await?;
        // fresh：仅当应答明确返回非空合法 id 时为 Some；策略路径的必得 id 由
        // `CodexProvider::start` 兜底 fail-closed（非策略历史行为保持 None）。
        response_thread_id(&thread_response)
    };
    Ok(CodexSessionHandshake {
        resume_session_id,
        thread_id,
    })
}

/// 会话循环（握手后）：turn/start + 事件循环。策略路径由 `start` 前置握手后
/// 直接进入本循环；非策略路径由 `run_codex_session` 自行握手后进入。
pub(crate) async fn run_codex_session_loop<W>(
    peer: JsonRpcPeer<W>,
    bridge: ApprovalBridge,
    event_tx: mpsc::Sender<ProviderEvent>,
    input: StreamingProviderInput,
    cancel: CancellationToken,
    handshake: CodexSessionHandshake,
    lc_policy_digest: Option<String>,
) -> Result<(), ProviderAdapterError>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    // 批2 Task 20（RevB2T13 P3）：LC 会话传入启动侧已算的 LC 冻结
    // digest——决策审计（file_change_detected/approval 等 `policy_digest`）
    // 与 provider_start 同源可 join；direct 会话 None=按 input 实算
    // direct canonical digest（既有语义）。
    let lc_policy_digest = lc_policy_digest.filter(|digest| !digest.trim().is_empty());
    let CodexSessionHandshake {
        resume_session_id,
        thread_id,
    } = handshake;
    let turn_thread_id = thread_id.clone().unwrap_or_default();

    let mut turn_id = start_codex_turn(&peer, &turn_thread_id, &input.prompt).await?;
    send_provider_event(
        &event_tx,
        ProviderEvent::StatusChanged(ProviderStatus::Running),
        &cancel,
    )
    .await?;
    send_provider_event(
        &event_tx,
        ProviderEvent::Execution(ProviderExecutionEvent {
            event_id: format!("turn_{turn_id}"),
            kind: ProviderExecutionEventKind::Turn,
            status: ProviderExecutionEventStatus::Started,
            title: "Turn started".to_string(),
            detail: None,
            command: None,
            cwd: Some(input.working_dir.display().to_string()),
            output: None,
            exit_code: None,
        }),
        &cancel,
    )
    .await?;

    let mut full_output = String::new();
    let mut streamed_agent_message_items = HashSet::new();
    // GC6：fileChange 审批的 diff 信息经 item id 关联缓存 item（不读 reason）；
    // 同一会话连续未知审批形态计数，`>=3` 次终止。
    let mut file_change_summaries: HashMap<String, String> = HashMap::new();
    let mut unknown_approval_occurrence: u32 = 0;
    // 批2 Task 20(RevB2T13 ⚠2 收窄):岗位纪律=Author 的 deny 投影唯一;
    // ReviewerVerify 窗口合同/其它 intent 不触发检测型纪律(fileChange
    // 中断纪律属 Author 岗位,Reviewer 写入由平台窗口对账承担)。
    let tool_policy_session = matches!(
        input.tool_policy.as_ref().map(|policy| &policy.intent),
        Some(crate::cross_cutting::streaming_provider::ToolPolicyIntent::DenyFileWriteBuiltins)
    );
    let mut empty_output_retry_used = false;
    let timeout_secs = input.timeout_secs.max(1);
    let timeout = tokio::time::sleep(Duration::from_secs(timeout_secs));
    tokio::pin!(timeout);
    let resume_stall_timeout = tokio::time::sleep(CODEX_RESUME_STALL_TIMEOUT);
    tokio::pin!(resume_stall_timeout);
    let mut waiting_for_resume_progress = resume_session_id.is_some();
    loop {
        let incoming = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(provider_error("Codex provider cancelled"));
            }
            _ = &mut timeout => {
                return Err(ProviderAdapterError::timeout(
                    full_output.clone(),
                    String::new(),
                    timeout_secs.saturating_mul(1000),
                ));
            }
            _ = &mut resume_stall_timeout, if waiting_for_resume_progress => {
                let session_id = resume_session_id.as_deref().unwrap_or("unknown");
                return Err(provider_error(format!(
                    "{CODEX_RESUME_STALL_ERROR} for thread {session_id}"
                )));
            }
            incoming = peer.next_incoming() => incoming.ok_or_else(|| {
                provider_error("Codex app-server stream ended before completion")
            })?,
        };

        if let Some(message) = parse_agent_message_text(&incoming) {
            waiting_for_resume_progress = false;
            if message.completed && streamed_agent_message_items.contains(&message.item_id) {
                continue;
            }
            streamed_agent_message_items.insert(message.item_id);
            full_output.push_str(&message.content);
            send_provider_event(
                &event_tx,
                ProviderEvent::TextDelta {
                    content: message.content,
                },
                &cancel,
            )
            .await?;
            continue;
        }

        if let Some(event) = parse_execution_event(&incoming) {
            waiting_for_resume_progress = false;
            send_provider_event(&event_tx, ProviderEvent::Execution(event), &cancel).await?;
            continue;
        }

        // fileChange item 通知：岗位纪律（Author）会话=T13 检测型触发（无审批
        // 请求亦然：先事件→durable→中断 turn，本轮 Failed/不可采信）；其余
        // 会话仅缓存摘要，供后续 requestApproval 经 item id 关联 diff。
        if let Some((item_id, summary, source_method)) = parse_file_change_summary(&incoming) {
            waiting_for_resume_progress = false;
            if tool_policy_session {
                return Err(fail_turn_on_author_file_change(
                    &event_tx,
                    &cancel,
                    input.audit_sink.as_ref(),
                    &input,
                    &item_id,
                    &summary,
                    &source_method,
                    lc_policy_digest.as_deref(),
                )
                .await);
            }
            file_change_summaries.insert(item_id, summary);
            continue;
        }

        if let Some(mut request) = parse_approval_request(&incoming) {
            waiting_for_resume_progress = false;
            // GC6：fileChange 描述来自缓存 item（item id 关联），不使用自然语言 reason。
            if request.category == CodexApprovalCategory::FileChange
                && let Some(summary) = file_change_summaries.get(&request.request_id)
            {
                request.description.clone_from(summary);
            }

            if let CodexApprovalCategory::Unknown { method } = request.category.clone() {
                unknown_approval_occurrence += 1;
                // 结构化 protocol_warning 事件：经事件通道送出会话循环（C1 真实出口）；
                // durable 落盘在 Task 3.2 的 sink 注入。
                let warning = CodexProtocolWarningEvent {
                    reason_code: "unsupported_approval_kind".to_string(),
                    method: method.clone(),
                    occurrence: unknown_approval_occurrence,
                };
                send_provider_event(
                    &event_tx,
                    ProviderEvent::ToolPolicyWarning(warning.clone()),
                    &cancel,
                )
                .await?;
                // durable 落盘（双路径）：append 失败沿 kill 链终止。
                audit_policy_event(
                    input.audit_sink.as_ref(),
                    DurableToolPolicyEvent::ProtocolWarning(ProtocolWarningAudit {
                        reason_code: warning.reason_code.clone(),
                        method: warning.method.clone(),
                        occurrence: warning.occurrence,
                    }),
                )?;
                tracing::warn!(
                    target: "codex_provider",
                    reason_code = %warning.reason_code,
                    method = %warning.method,
                    occurrence = warning.occurrence,
                    "unclassified codex approval form"
                );
                let response = decide_unknown(&method, unknown_approval_occurrence);
                write_approval_decision(&peer, request.rpc_id.clone(), &response).await?;
                if let Some(reason_code) = unknown_storm_reason_after(unknown_approval_occurrence) {
                    // 结构化 session_terminated 事件：先经事件通道送出再终止会话
                    // （C1 真实出口；durable 落盘在 Task 3.2）。
                    let termination = CodexSessionTerminatedEvent {
                        reason_code: reason_code.to_string(),
                    };
                    send_provider_event(
                        &event_tx,
                        ProviderEvent::ToolPolicyTerminated(termination),
                        &cancel,
                    )
                    .await?;
                    // durable 落盘（双路径）：append 失败沿 kill 链终止。
                    audit_policy_event(
                        input.audit_sink.as_ref(),
                        DurableToolPolicyEvent::SessionTerminated(SessionTerminatedAudit {
                            reason_code: reason_code.to_string(),
                        }),
                    )?;
                    tracing::warn!(
                        target: "codex_provider",
                        reason_code,
                        "terminating codex session after unknown approval storm"
                    );
                    return Err(provider_error(format!(
                        "codex session terminated: {reason_code}"
                    )));
                }
                continue;
            }

            // 已知审批形态打断连续未知计数。
            unknown_approval_occurrence = 0;

            if tool_policy_session {
                // T13：fileChange 审批请求（danger 下通常不出现；若出现即是
                // fileChange 事实）先走既有决策双路径合法应答 decline（不静默
                // 无应答；Decline 不是防写依据），随后检测纪律中断 turn。
                if matches!(request.category, CodexApprovalCategory::FileChange) {
                    let decision_event = CodexApprovalDecisionEvent {
                        request_id: request.request_id.clone(),
                        category: request.category.audit_text(),
                        decision: CodexApprovalResponse::Decline.audit_text(),
                    };
                    send_provider_event(
                        &event_tx,
                        ProviderEvent::ToolPolicyDecision(decision_event.clone()),
                        &cancel,
                    )
                    .await?;
                    audit_policy_event(
                        input.audit_sink.as_ref(),
                        DurableToolPolicyEvent::ApprovalDecision(ApprovalDecisionAudit {
                            request_id: decision_event.request_id.clone(),
                            category: decision_event.category.to_string(),
                            server_name: request.server_name.clone(),
                            tool_name: request.tool_name.clone(),
                            decision: decision_event.decision.to_string(),
                            reason_code: policy_decision_reason_code(request.category.clone())
                                .to_string(),
                            policy_digest: policy_digest_for_input(&input, lc_policy_digest.as_deref()),
                        }),
                    )?;
                    write_approval_decision(
                        &peer,
                        request.rpc_id.clone(),
                        &CodexApprovalResponse::Decline,
                    )
                    .await?;
                    return Err(fail_turn_on_author_file_change(
                        &event_tx,
                        &cancel,
                        input.audit_sink.as_ref(),
                        &input,
                        &request.request_id,
                        &request.description,
                    "item/fileChange/requestApproval",
                    lc_policy_digest.as_deref(),
                    )
                    .await);
                }
                // 岗位纪律即时决策：commandExecution/MCP accept，决策经事件通道
                // 送出会话循环（C1 真实出口）。
                let response = decide_for_policy(request.category.clone());
                let decision_event = CodexApprovalDecisionEvent {
                    request_id: request.request_id.clone(),
                    category: request.category.audit_text(),
                    decision: response.audit_text(),
                };
                send_provider_event(
                    &event_tx,
                    ProviderEvent::ToolPolicyDecision(decision_event.clone()),
                    &cancel,
                )
                .await?;
                // durable 落盘（双路径）：append 失败沿 kill 链终止。D7 冻结字段：
                // server_name/tool_name 仅 MCP 形态携带；reason_code/policy_digest
                // 为决策归属的语义原因与会话策略指纹。
                audit_policy_event(
                    input.audit_sink.as_ref(),
                    DurableToolPolicyEvent::ApprovalDecision(ApprovalDecisionAudit {
                        request_id: decision_event.request_id.clone(),
                        category: decision_event.category.to_string(),
                        server_name: request.server_name.clone(),
                        tool_name: request.tool_name.clone(),
                        decision: decision_event.decision.to_string(),
                        reason_code: policy_decision_reason_code(request.category.clone())
                            .to_string(),
                        policy_digest: policy_digest_for_input(&input, lc_policy_digest.as_deref()),
                    }),
                )?;
                tracing::info!(
                    target: "codex_provider",
                    request_id = %decision_event.request_id,
                    category = decision_event.category,
                    decision = decision_event.decision,
                    "codex policy approval decision"
                );
                write_approval_decision(&peer, request.rpc_id.clone(), &response).await?;
                continue;
            }

            match request.category {
                CodexApprovalCategory::McpToolCall => {
                    // Coder 会话 MCP 同样 accept 并审计（既有 execution event 通道）。
                    let audit_detail = format!(
                        "{}: {}",
                        request.server_name.as_deref().unwrap_or("mcp"),
                        request.description
                    );
                    send_provider_event(
                        &event_tx,
                        ProviderEvent::Execution(ProviderExecutionEvent {
                            event_id: format!("mcp_approval_{}", request.request_id),
                            kind: ProviderExecutionEventKind::Provider,
                            status: ProviderExecutionEventStatus::Completed,
                            title: "MCP tool call approved".to_string(),
                            detail: Some(audit_detail),
                            command: None,
                            cwd: None,
                            output: Some(
                                serde_json::json!({
                                    "auto_accepted": true,
                                    "codex_approval_kind": "mcp_tool_call",
                                    "request_id": request.request_id,
                                })
                                .to_string(),
                            ),
                            exit_code: None,
                        }),
                        &cancel,
                    )
                    .await?;
                    write_approval_decision(
                        &peer,
                        request.rpc_id.clone(),
                        &CodexApprovalResponse::Accept,
                    )
                    .await?;
                }
                CodexApprovalCategory::CommandExecution | CodexApprovalCategory::FileChange => {
                    // Coder 的 exec/fileChange 维持既有 ApprovalBridge 上抛链。
                    let decision = bridge
                        .request_tool(
                            request.tool_name.as_deref().unwrap_or("codex_tool"),
                            &request.description,
                            RiskLevel::High,
                            cancel.clone(),
                        )
                        .await?;
                    write_approval_response(&peer, request.rpc_id, decision.approved).await?;
                }
                CodexApprovalCategory::Unknown { .. } => {
                    unreachable!("unknown approval forms are handled before policy dispatch")
                }
            }
            continue;
        }

        if let Some(request) = parse_user_input_request(&incoming) {
            waiting_for_resume_progress = false;
            let decision = match bridge
                .request_choice(
                    ChoiceRequestData {
                        id: request.id.clone(),
                        prompt: request.prompt.clone(),
                        options: request.options.clone(),
                        allow_multiple: false,
                        allow_free_text: request.allow_free_text,
                        questions: request.questions.clone(),
                        source: ChoiceRequestSource::RequestUserInput,
                    },
                    cancel.clone(),
                )
                .await
            {
                Ok(decision) => decision,
                Err(error) => {
                    emit_request_user_input_protocol_error(
                        &event_tx,
                        "choice bridge",
                        &request.question_id,
                        &error.details,
                    )
                    .await;
                    return Err(error);
                }
            };
            if let Err(error) =
                write_user_input_response(&peer, request.rpc_id, &request.question_id, decision)
                    .await
            {
                emit_request_user_input_protocol_error(
                    &event_tx,
                    "response write",
                    &request.question_id,
                    &error.details,
                )
                .await;
                return Err(error);
            }
            continue;
        }

        if is_turn_completed(&incoming) {
            if full_output.trim().is_empty() {
                if empty_output_retry_used {
                    return Err(ProviderAdapterError::provider_empty_output(format!(
                        "{CODEX_EMPTY_OUTPUT_ERROR}: Codex turn {turn_id} completed without \
                         agent output after one bounded retry"
                    )));
                }
                empty_output_retry_used = true;
                tracing::warn!(
                    target: "codex_provider",
                    thread_id = %turn_thread_id,
                    turn_id = %turn_id,
                    "Codex turn completed with empty output; retrying once in-session"
                );
                send_provider_event(
                    &event_tx,
                    ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: format!("turn_{turn_id}_empty_output_retry"),
                        kind: ProviderExecutionEventKind::Turn,
                        status: ProviderExecutionEventStatus::Running,
                        title: "Turn empty output retry".to_string(),
                        detail: Some(
                            "Codex turn completed with empty output; retrying once".to_string(),
                        ),
                        command: None,
                        cwd: Some(input.working_dir.display().to_string()),
                        output: None,
                        exit_code: None,
                    }),
                    &cancel,
                )
                .await?;
                full_output.clear();
                // 重试是新的一轮流式交付：去重集合必须随之清空，否则重试 turn 复用
                // 已登记 item id 时，恢复内容会被当作重复 item 丢弃。
                streamed_agent_message_items.clear();
                turn_id = start_codex_turn(&peer, &turn_thread_id, CODEX_EMPTY_OUTPUT_RETRY_PROMPT)
                    .await?;
                continue;
            }
            // 协议层 usage 优先；当前 Codex 版本不携带时，以 threadId 精确匹配
            // ~/.codex/sessions 的 rollout 并读取最近 token_count。读取失败不影响 turn。
            let usage_role = UsageReportData::role_text(&input.role);
            let report = parse_codex_usage(&incoming, usage_role).or_else(|| {
                (!turn_thread_id.is_empty())
                    .then(|| read_default_codex_usage(&turn_thread_id, usage_role))
                    .flatten()
            });
            if let Some(report) = report {
                send_provider_event(&event_tx, ProviderEvent::UsageReport(report), &cancel).await?;
            }
            send_provider_event(
                &event_tx,
                ProviderEvent::Execution(ProviderExecutionEvent {
                    event_id: format!("turn_{turn_id}"),
                    kind: ProviderExecutionEventKind::Turn,
                    status: ProviderExecutionEventStatus::Completed,
                    title: "Turn completed".to_string(),
                    detail: None,
                    command: None,
                    cwd: Some(input.working_dir.display().to_string()),
                    output: None,
                    exit_code: None,
                }),
                &cancel,
            )
            .await?;
            send_provider_event(
                &event_tx,
                ProviderEvent::StatusChanged(ProviderStatus::Completed),
                &cancel,
            )
            .await?;
            let completion = ProviderCompletion::from_output(
                full_output,
                input.structured_output_contract.as_ref(),
                thread_id,
            );
            send_provider_event(&event_tx, ProviderEvent::Completed(completion), &cancel).await?;
            return Ok(());
        }

        if let Some(message) = parse_failure(&incoming) {
            return Err(provider_error(message));
        }
    }
}
