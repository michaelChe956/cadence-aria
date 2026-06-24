use super::*;

pub(crate) fn workspace_stage_from_ws_stage(stage: &WsWorkspaceStage) -> WorkspaceStage {
    match stage {
        WsWorkspaceStage::PrepareContext => WorkspaceStage::PrepareContext,
        WsWorkspaceStage::Running => WorkspaceStage::Running,
        WsWorkspaceStage::AuthorConfirm => WorkspaceStage::AuthorConfirm,
        WsWorkspaceStage::CrossReview => WorkspaceStage::CrossReview,
        WsWorkspaceStage::ReviewDecision => WorkspaceStage::ReviewDecision,
        WsWorkspaceStage::Revision => WorkspaceStage::Revision,
        WsWorkspaceStage::HumanConfirm => WorkspaceStage::HumanConfirm,
        WsWorkspaceStage::Completed => WorkspaceStage::Completed,
    }
}

pub(crate) fn ws_stage(stage: &WorkspaceStage) -> WsWorkspaceStage {
    match stage {
        WorkspaceStage::PrepareContext => WsWorkspaceStage::PrepareContext,
        WorkspaceStage::Running => WsWorkspaceStage::Running,
        WorkspaceStage::AuthorConfirm => WsWorkspaceStage::AuthorConfirm,
        WorkspaceStage::CrossReview => WsWorkspaceStage::CrossReview,
        WorkspaceStage::ReviewDecision => WsWorkspaceStage::ReviewDecision,
        WorkspaceStage::Revision => WsWorkspaceStage::Revision,
        WorkspaceStage::HumanConfirm => WsWorkspaceStage::HumanConfirm,
        WorkspaceStage::Completed => WsWorkspaceStage::Completed,
    }
}

pub(crate) fn provider_type_for_name(provider: &ProviderName) -> ProviderType {
    match provider {
        ProviderName::ClaudeCode => ProviderType::ClaudeCode,
        ProviderName::Codex => ProviderType::Codex,
        ProviderName::Fake => ProviderType::Fake,
    }
}

pub(crate) fn provider_name_text(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::ClaudeCode => "claude_code",
        ProviderName::Codex => "codex",
        ProviderName::Fake => "fake",
    }
}

pub(crate) fn risk_level_text(risk_level: &RiskLevel) -> &'static str {
    match risk_level {
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
    }
}

pub(crate) fn execution_event_json(event: &ProviderExecutionEvent) -> serde_json::Value {
    serde_json::json!({
        "event_id": event.event_id,
        "kind": execution_event_kind_text(&event.kind),
        "status": execution_event_status_text(&event.status),
        "title": event.title,
        "detail": event.detail,
        "command": event.command,
        "cwd": event.cwd,
        "output": event.output,
        "exit_code": event.exit_code,
    })
}

pub(crate) fn upsert_execution_event_json(
    events: &mut Vec<serde_json::Value>,
    event: serde_json::Value,
) {
    let Some(event_id) = event
        .get("event_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
    else {
        events.push(event);
        return;
    };

    if let Some(existing) = events.iter_mut().find(|existing| {
        existing.get("event_id").and_then(serde_json::Value::as_str) == Some(event_id.as_str())
    }) {
        *existing = event;
        return;
    }

    events.push(event);
}

pub(crate) fn provider_prompt_event(
    node_id: &str,
    prompt: String,
    detail: &'static str,
) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: format!("{node_id}_prompt"),
        kind: ProviderExecutionEventKind::Output,
        status: ProviderExecutionEventStatus::Started,
        title: "Provider Prompt".to_string(),
        detail: Some(detail.to_string()),
        command: None,
        cwd: None,
        output: Some(prompt),
        exit_code: None,
    }
}

pub(crate) fn execution_event_from_tool_call(call: ProviderToolCall) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: call.id,
        kind: ProviderExecutionEventKind::Command,
        status: ProviderExecutionEventStatus::Started,
        title: call.tool_name,
        detail: Some(format_tool_call_input(&call.input)),
        command: extract_tool_command(&call.input),
        cwd: None,
        output: None,
        exit_code: None,
    }
}

pub(crate) fn execution_event_from_tool_result(
    result: ProviderToolResult,
    title: String,
    command: Option<String>,
) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: result.tool_use_id,
        kind: ProviderExecutionEventKind::Command,
        status: if result.is_error {
            ProviderExecutionEventStatus::Failed
        } else {
            ProviderExecutionEventStatus::Completed
        },
        title,
        detail: None,
        command,
        cwd: None,
        output: Some(result.output),
        exit_code: if result.is_error { Some(1) } else { Some(0) },
    }
}

pub(crate) fn format_tool_call_input(input: &serde_json::Value) -> String {
    serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string())
}

pub(crate) fn extract_tool_command(input: &serde_json::Value) -> Option<String> {
    let command = input.get("command").or_else(|| input.get("cmd"))?;
    if let Some(command) = command.as_str() {
        return Some(command.to_string());
    }
    command.as_array().and_then(|parts| {
        parts
            .iter()
            .map(serde_json::Value::as_str)
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join(" "))
            .filter(|command| !command.trim().is_empty())
    })
}

pub(crate) fn execution_event_kind_text(kind: &ProviderExecutionEventKind) -> &'static str {
    match kind {
        ProviderExecutionEventKind::Provider => "provider",
        ProviderExecutionEventKind::Turn => "turn",
        ProviderExecutionEventKind::Command => "command",
        ProviderExecutionEventKind::Output => "output",
        ProviderExecutionEventKind::Artifact => "artifact",
    }
}

pub(crate) fn execution_event_status_text(status: &ProviderExecutionEventStatus) -> &'static str {
    match status {
        ProviderExecutionEventStatus::Started => "started",
        ProviderExecutionEventStatus::Running => "running",
        ProviderExecutionEventStatus::WaitingApproval => "waiting_approval",
        ProviderExecutionEventStatus::Completed => "completed",
        ProviderExecutionEventStatus::Failed => "failed",
        ProviderExecutionEventStatus::Aborted => "aborted",
    }
}

pub(crate) fn workspace_stage_for_status(status: &WorkspaceSessionStatus) -> WorkspaceStage {
    match status {
        WorkspaceSessionStatus::Open => WorkspaceStage::PrepareContext,
        WorkspaceSessionStatus::Running => WorkspaceStage::Running,
        WorkspaceSessionStatus::WaitingForHuman | WorkspaceSessionStatus::ChangeRequested => {
            WorkspaceStage::HumanConfirm
        }
        WorkspaceSessionStatus::Confirmed => WorkspaceStage::Completed,
        WorkspaceSessionStatus::BlockedProviderUnavailable | WorkspaceSessionStatus::Terminated => {
            WorkspaceStage::Completed
        }
    }
}

pub(crate) fn workspace_status_for_stage(stage: &WorkspaceStage) -> WorkspaceSessionStatus {
    match stage {
        WorkspaceStage::PrepareContext => WorkspaceSessionStatus::Open,
        WorkspaceStage::Running
        | WorkspaceStage::CrossReview
        | WorkspaceStage::ReviewDecision
        | WorkspaceStage::Revision => WorkspaceSessionStatus::Running,
        WorkspaceStage::AuthorConfirm | WorkspaceStage::HumanConfirm => {
            WorkspaceSessionStatus::WaitingForHuman
        }
        WorkspaceStage::Completed => WorkspaceSessionStatus::Confirmed,
    }
}

pub(crate) fn latest_artifact_from_messages(
    messages: &[WorkspaceMessageRecord],
) -> Option<ArtifactPayload> {
    messages
        .iter()
        .rev()
        .find(|message| matches!(message.role.as_str(), "assistant" | "provider"))
        .map(|message| ArtifactPayload::Markdown {
            markdown: extract_artifact_content(&message.content),
            diff: None,
        })
}
