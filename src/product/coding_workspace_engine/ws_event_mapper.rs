use super::*;
use crate::cross_cutting::streaming_provider::UsageReportData;

pub(crate) fn provider_start_is_not_implemented(error: &ProviderAdapterError) -> bool {
    error.stderr == "streaming provider start is not implemented"
}

pub(crate) fn ws_event_from_provider_execution(
    event: ProviderExecutionEvent,
    node_id: &str,
    provider: &ProviderName,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: event.event_id,
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: ws_execution_event_kind(event.kind),
        status: ws_execution_event_status(event.status),
        title: event.title,
        detail: event.detail,
        command: event.command,
        cwd: event.cwd,
        output: event.output,
        exit_code: event.exit_code,
    }
}

pub(crate) fn ws_event_from_tool_call(
    node_id: &str,
    provider: &ProviderName,
    call: ProviderToolCall,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: call.id,
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: WsExecutionEventKind::Command,
        status: WsExecutionEventStatus::Started,
        title: call.tool_name,
        detail: Some(format_tool_call_input(&call.input)),
        command: extract_tool_command(&call.input),
        cwd: None,
        output: None,
        exit_code: None,
    }
}

pub(crate) fn ws_event_from_tool_result(
    node_id: &str,
    provider: &ProviderName,
    title: &str,
    command: Option<String>,
    result: ProviderToolResult,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: result.tool_use_id,
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: WsExecutionEventKind::Command,
        status: if result.is_error {
            WsExecutionEventStatus::Failed
        } else {
            WsExecutionEventStatus::Completed
        },
        title: title.to_string(),
        detail: None,
        command,
        cwd: None,
        output: Some(result.output),
        exit_code: if result.is_error { Some(1) } else { Some(0) },
    }
}

pub(crate) fn ws_event_from_permission_request(
    node_id: &str,
    provider: &ProviderName,
    request: &PermissionRequestData,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: format!("permission_{}", request.id),
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: WsExecutionEventKind::Command,
        status: WsExecutionEventStatus::WaitingApproval,
        title: "Waiting for permission".to_string(),
        detail: Some(request.description.clone()),
        command: Some(request.tool_name.clone()),
        cwd: None,
        output: None,
        exit_code: None,
    }
}

pub(crate) fn ws_event_from_choice_request(
    node_id: &str,
    provider: &ProviderName,
    request: &ChoiceRequestData,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: format!("choice_{}", request.id),
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: WsExecutionEventKind::Provider,
        status: WsExecutionEventStatus::WaitingApproval,
        title: "Waiting for choice".to_string(),
        detail: Some(request.prompt.clone()),
        command: None,
        cwd: None,
        output: None,
        exit_code: None,
    }
}

pub(crate) fn ws_event_from_provider_status(
    node_id: &str,
    provider: &ProviderName,
    status: ProviderStatus,
) -> WsExecutionEvent {
    let status_text = provider_status_text(&status);
    WsExecutionEvent {
        event_id: format!("{node_id}_provider_status_{status_text}"),
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: WsExecutionEventKind::Provider,
        status: ws_status_from_provider_status(status),
        title: format!("Provider {status_text}"),
        detail: None,
        command: None,
        cwd: None,
        output: None,
        exit_code: None,
    }
}

/// 将 provider 上报的 token 用量映射为 kind=usage 的 execution event（与
/// `workspace_engine::mappings::execution_event_from_usage_report` 同构）。
///
/// `event_id` 固定为 `usage_{role}`，同 role 多次上报时消费侧按 event_id
/// upsert 覆盖为最新快照；`output` 为 `UsageReportData` 的 JSON 序列化，
/// 供 campaign driver 提取按角色 token 用量。
pub(crate) fn ws_execution_event_from_usage_report(
    report: UsageReportData,
) -> ProviderExecutionEvent {
    let role = report.role.clone();
    let output = serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string());
    ProviderExecutionEvent {
        event_id: format!("usage_{role}"),
        kind: ProviderExecutionEventKind::Usage,
        status: ProviderExecutionEventStatus::Completed,
        title: format!("{role} token usage"),
        detail: None,
        command: None,
        cwd: None,
        output: Some(output),
        exit_code: None,
    }
}

pub(crate) fn ws_execution_event_kind(kind: ProviderExecutionEventKind) -> WsExecutionEventKind {
    match kind {
        ProviderExecutionEventKind::Provider => WsExecutionEventKind::Provider,
        ProviderExecutionEventKind::Turn => WsExecutionEventKind::Turn,
        ProviderExecutionEventKind::Command => WsExecutionEventKind::Command,
        ProviderExecutionEventKind::Output => WsExecutionEventKind::Output,
        ProviderExecutionEventKind::Artifact => WsExecutionEventKind::Artifact,
        ProviderExecutionEventKind::Usage => WsExecutionEventKind::Usage,
    }
}

pub(crate) fn ws_execution_event_status(
    status: ProviderExecutionEventStatus,
) -> WsExecutionEventStatus {
    match status {
        ProviderExecutionEventStatus::Started => WsExecutionEventStatus::Started,
        ProviderExecutionEventStatus::Running => WsExecutionEventStatus::Running,
        ProviderExecutionEventStatus::WaitingApproval => WsExecutionEventStatus::WaitingApproval,
        ProviderExecutionEventStatus::Completed => WsExecutionEventStatus::Completed,
        ProviderExecutionEventStatus::Failed => WsExecutionEventStatus::Failed,
        ProviderExecutionEventStatus::Aborted => WsExecutionEventStatus::Aborted,
    }
}

pub(crate) fn ws_status_from_provider_status(status: ProviderStatus) -> WsExecutionEventStatus {
    match status {
        ProviderStatus::Starting => WsExecutionEventStatus::Started,
        ProviderStatus::Running => WsExecutionEventStatus::Running,
        ProviderStatus::WaitingApproval => WsExecutionEventStatus::WaitingApproval,
        ProviderStatus::Completed => WsExecutionEventStatus::Completed,
        ProviderStatus::Failed => WsExecutionEventStatus::Failed,
        ProviderStatus::Aborted => WsExecutionEventStatus::Aborted,
    }
}

pub(crate) fn provider_status_text(status: &ProviderStatus) -> &'static str {
    match status {
        ProviderStatus::Starting => "starting",
        ProviderStatus::Running => "running",
        ProviderStatus::WaitingApproval => "waiting_approval",
        ProviderStatus::Completed => "completed",
        ProviderStatus::Failed => "failed",
        ProviderStatus::Aborted => "aborted",
    }
}

pub(crate) fn ws_permission_risk_level(risk_level: RiskLevel) -> WsPermissionRiskLevel {
    match risk_level {
        RiskLevel::Low => WsPermissionRiskLevel::Low,
        RiskLevel::Medium => WsPermissionRiskLevel::Medium,
        RiskLevel::High => WsPermissionRiskLevel::High,
    }
}
