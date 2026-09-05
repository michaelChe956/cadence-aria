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

/// DenyFileWriteBuiltins 的 codex canonical 参数（三联动冻结：`sandbox=read-only` +
/// `approvalPolicy=on-request`；argv/参数原文按出现顺序、大小写保留）。
pub fn deny_file_write_builtins_tokens() -> Vec<String> {
    vec![
        "sandbox=read-only".to_string(),
        "approvalPolicy=on-request".to_string(),
    ]
}

/// thread/start 与 thread/resume 共用的启动参数单一来源（三联动冻结，GC5）：
/// - 策略 input（`tool_policy.is_some()`）同时给出 `sandbox:"read-only"` 与
///   `approvalPolicy:"on-request"`（策略会话禁写禁执行，审批走分类规则）；
/// - Coder（`None`）保持 `danger-full-access` 与既有 permission mode 映射
///   （Auto→never / Supervised→on-request）。
pub(crate) fn codex_launch_params(input: &StreamingProviderInput) -> serde_json::Value {
    if input.tool_policy.is_some() {
        json!({
            "cwd": input.working_dir.clone(),
            "approvalPolicy": "on-request",
            "sandbox": "read-only",
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

/// 策略会话即时审批决策（GC6 冻结）：commandExecution/fileChange 一律拒绝并
/// 审计；所有会话 MCP 一律 accept 并审计。决策以结构化事件形式暴露
/// （`CodexApprovalDecisionEvent`，内存出口；durable 接线在 Task 3.2）。
pub(crate) fn decide_for_policy(category: CodexApprovalCategory) -> CodexApprovalResponse {
    match category {
        CodexApprovalCategory::McpToolCall => CodexApprovalResponse::Accept,
        // 未知形态在 session 层由 decide_unknown 先行处理；此处 fail-closed 拒绝。
        CodexApprovalCategory::CommandExecution
        | CodexApprovalCategory::FileChange
        | CodexApprovalCategory::Unknown { .. } => CodexApprovalResponse::Decline,
    }
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
        let mut resume_params = codex_launch_params(&input);
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
        resume_response
            .pointer("/thread/id")
            .or_else(|| resume_response.pointer("/id"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or_else(|| Some(session_id.to_string()))
    } else {
        let thread_response = peer
            .request_with_timeout(
                json!({
                    "jsonrpc": "2.0",
                    "method": "thread/start",
                    "params": codex_launch_params(&input),
                }),
                CODEX_RPC_REQUEST_TIMEOUT,
            )
            .await?;
        thread_response
            .pointer("/thread/id")
            .or_else(|| thread_response.pointer("/id"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
    };
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
    let tool_policy_session = input.tool_policy.is_some();
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

        // fileChange item 通知进入缓存，供后续 requestApproval 经 item id 关联 diff。
        if let Some((item_id, summary)) = parse_file_change_summary(&incoming) {
            waiting_for_resume_progress = false;
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
                // 策略会话即时决策：exec/fileChange 拒绝、MCP accept，决策经事件通道
                // 送出会话循环（C1 真实出口；durable 落盘在 Task 3.2 的 sink 注入）。
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
