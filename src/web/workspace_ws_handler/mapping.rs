use super::*;

pub(crate) fn map_revision_path(
    path: RevisionPath,
    extra_context: Option<String>,
) -> (String, Option<String>) {
    match path {
        RevisionPath::Revise => ("continue".to_string(), None),
        RevisionPath::ReviseWithContext => ("continue_with_context".to_string(), extra_context),
        RevisionPath::SkipToHuman => ("human_intervene".to_string(), None),
    }
}

pub(crate) fn ws_permission_risk_level(risk_level: RiskLevel) -> WsPermissionRiskLevel {
    match risk_level {
        RiskLevel::Low => WsPermissionRiskLevel::Low,
        RiskLevel::Medium => WsPermissionRiskLevel::Medium,
        RiskLevel::High => WsPermissionRiskLevel::High,
    }
}

pub(crate) fn ws_choice_option(option: ChoiceOptionData) -> ChoiceOption {
    ChoiceOption {
        id: option.id,
        label: option.label,
        description: option.description,
    }
}

pub(crate) fn ws_provider_status(status: ProviderStatus) -> WsProviderStatus {
    match status {
        ProviderStatus::Starting => WsProviderStatus::Starting,
        ProviderStatus::Running => WsProviderStatus::Running,
        ProviderStatus::WaitingApproval => WsProviderStatus::WaitingApproval,
        ProviderStatus::Completed => WsProviderStatus::Completed,
        ProviderStatus::Failed => WsProviderStatus::Failed,
        ProviderStatus::Aborted => WsProviderStatus::Aborted,
    }
}

pub(crate) fn ws_execution_event(
    event: ProviderExecutionEvent,
    node_id: Option<String>,
    agent: Option<crate::product::models::ProviderName>,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: event.event_id,
        node_id,
        agent,
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

pub(crate) fn build_work_item_plan_generate_request(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
) -> Result<GenerateWorkItemsRequest, String> {
    let session = engine.session();
    let plan = lifecycle
        .get_issue_work_item_plan(&session.project_id, &session.issue_id, &session.entity_id)
        .map_err(|e| format!("load plan failed: {e}"))?;
    let provider_name_string = |name: &ProviderName| -> Result<String, String> {
        serde_json::to_value(name)
            .map_err(|e| format!("serialize provider name failed: {e}"))
            .and_then(|v| {
                v.as_str()
                    .map(ToString::to_string)
                    .ok_or_else(|| format!("provider name is not a string: {v}"))
            })
    };
    let revision_feedback = if plan.validator_findings.is_empty() {
        None
    } else {
        let feedback = plan
            .validator_findings
            .iter()
            .map(|finding| {
                format!(
                    "- [{}][{}] {} (work items: {})",
                    finding.severity.as_str(),
                    finding.code,
                    finding.message,
                    finding.work_item_ids.join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        Some(feedback)
    };
    Ok(GenerateWorkItemsRequest {
        title: plan.id.clone(),
        story_spec_ids: plan.source_story_spec_ids.clone(),
        design_spec_ids: plan.source_design_spec_ids.clone(),
        include_integration_tests: Some(plan.options.include_integration_tests),
        include_e2e_tests: Some(plan.options.include_e2e_tests),
        force_frontend_backend_split: Some(plan.options.force_frontend_backend_split),
        require_execution_plan_confirm: Some(plan.options.require_execution_plan_confirm),
        author_provider: Some(provider_name_string(&session.author_provider)?),
        reviewer_provider: session
            .reviewer_provider
            .as_ref()
            .map(provider_name_string)
            .transpose()?,
        review_rounds: Some(session.review_rounds),
        superpowers_enabled: Some(session.superpowers_enabled),
        openspec_enabled: Some(session.openspec_enabled),
        revision_feedback,
    })
}

pub(crate) fn load_work_item_plan_outline_context_resolutions(
    app_paths: &ProductAppPaths,
    session: &WorkspaceSessionRecord,
    request: &GenerateWorkItemsRequest,
    lifecycle: &LifecycleStore,
    issue: &crate::product::models::IssueRecord,
) -> Result<Vec<OutlineContextBlockerResolution>, String> {
    let store = WorkItemPlanStore::new(app_paths.clone());
    let capabilities =
        design_context_capabilities_for_request(lifecycle, request, issue).map_err(|error| {
            format!(
                "extract design context capabilities failed: {}",
                error.message
            )
        })?;
    let gaps = design_context_gaps(&capabilities);
    let now = chrono::Utc::now().to_rfc3339();
    let mut index = store
        .load_outline_context_index(&session.project_id, &session.issue_id, &session.entity_id)
        .map_err(|error| format!("load outline context index failed: {error}"))?
        .unwrap_or_else(|| OutlineContextIndex {
            project_id: session.project_id.clone(),
            issue_id: session.issue_id.clone(),
            plan_id: session.entity_id.clone(),
            generation_round_id: "outline_stage".to_string(),
            blocker_resolutions: Vec::new(),
            design_context_gaps: Vec::new(),
            design_context_capabilities: capabilities.clone(),
            updated_at: now.clone(),
        });
    index.design_context_capabilities = capabilities;
    index.design_context_gaps = gaps;
    index.updated_at = now;
    store
        .save_outline_context_index(&index)
        .map_err(|error| format!("save outline context index failed: {error}"))?;
    Ok(index.blocker_resolutions)
}

pub(crate) fn spawn_engine_event_forward_task(
    mut engine_rx: mpsc::Receiver<EngineEvent>,
    outbound_tx: mpsc::Sender<OutboundControl>,
    session_id: String,
    workspace_runs: WorkspaceRunRegistry,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = engine_rx.recv().await {
            let ws_msg = match event {
                EngineEvent::StreamChunk {
                    role,
                    content,
                    node_id,
                } => WsOutMessage::StreamChunk {
                    role,
                    content,
                    node_id,
                },
                EngineEvent::MessageComplete {
                    message_id,
                    checkpoint_id,
                    node_id,
                } => WsOutMessage::MessageComplete {
                    message_id,
                    checkpoint_id,
                    node_id,
                },
                EngineEvent::StageChange { stage } => WsOutMessage::StageChange { stage },
                EngineEvent::ArtifactUpdate { version, payload } => {
                    WsOutMessage::ArtifactUpdate { version, payload }
                }
                EngineEvent::PermissionRequest {
                    id,
                    tool_name,
                    description,
                    risk_level,
                } => WsOutMessage::PermissionRequest {
                    id,
                    tool_name,
                    description,
                    risk_level: ws_permission_risk_level(risk_level),
                },
                EngineEvent::ChoiceRequest {
                    id,
                    prompt,
                    options,
                    allow_multiple,
                    allow_free_text,
                    source,
                } => {
                    eprintln!(
                        "[aria-choice-diag] ws outbound choice_request session={} id={} source={} options={} prompt_chars={}",
                        session_id,
                        id,
                        source.as_str(),
                        options.len(),
                        prompt.chars().count()
                    );
                    if source != ChoiceRequestSource::TextFallback {
                        let _ = workspace_runs
                            .register_choice(&session_id, id.clone())
                            .await;
                    }
                    WsOutMessage::ChoiceRequest {
                        id,
                        prompt,
                        options: options.into_iter().map(ws_choice_option).collect(),
                        allow_multiple,
                        allow_free_text,
                        source: source.as_str().to_string(),
                    }
                }
                EngineEvent::ProviderStatus { status } => WsOutMessage::ProviderStatus {
                    status: ws_provider_status(status),
                },
                EngineEvent::ExecutionEvent {
                    event,
                    node_id,
                    agent,
                } => WsOutMessage::ExecutionEvent {
                    event: ws_execution_event(event, node_id, agent),
                },
                EngineEvent::TimelineNodeCreated { node } => {
                    WsOutMessage::TimelineNodeCreated { node }
                }
                EngineEvent::TimelineNodeUpdated {
                    node_id,
                    status,
                    summary,
                    completed_at,
                } => WsOutMessage::TimelineNodeUpdated {
                    node_id,
                    status,
                    summary,
                    completed_at,
                },
                EngineEvent::ReviewComplete {
                    node_id,
                    round,
                    verdict,
                    comments,
                    summary,
                    findings,
                    review_gate,
                    work_item_plan_review,
                } => WsOutMessage::ReviewComplete {
                    node_id,
                    round,
                    verdict,
                    comments,
                    summary,
                    findings,
                    review_gate,
                    work_item_plan_review,
                },
                EngineEvent::ReviewDecisionRequired {
                    node_id,
                    round,
                    options,
                } => WsOutMessage::ReviewDecisionRequired {
                    node_id,
                    round,
                    options,
                },
                EngineEvent::Error { message } => WsOutMessage::Error { message },
                EngineEvent::ProtocolError {
                    code,
                    message,
                    context,
                } => WsOutMessage::ProtocolError {
                    code,
                    message,
                    context,
                },
                EngineEvent::PermissionTimeout {
                    permission_id,
                    node_id,
                } => WsOutMessage::ProtocolError {
                    code: "PERMISSION_TIMEOUT".to_string(),
                    message: format!("Permission request {permission_id} timed out"),
                    context: Some(serde_json::json!({
                        "permission_id": permission_id,
                        "node_id": node_id,
                    })),
                },
            };
            if !send_json_outbound(&outbound_tx, &ws_msg).await {
                break;
            }
        }
    })
}
