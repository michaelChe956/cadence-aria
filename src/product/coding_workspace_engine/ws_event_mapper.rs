use super::*;

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

pub(crate) fn ws_execution_event_kind(kind: ProviderExecutionEventKind) -> WsExecutionEventKind {
    match kind {
        ProviderExecutionEventKind::Provider => WsExecutionEventKind::Provider,
        ProviderExecutionEventKind::Turn => WsExecutionEventKind::Turn,
        ProviderExecutionEventKind::Command => WsExecutionEventKind::Command,
        ProviderExecutionEventKind::Output => WsExecutionEventKind::Output,
        ProviderExecutionEventKind::Artifact => WsExecutionEventKind::Artifact,
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
