use super::*;

mod stream;
mod timeline;

pub(crate) fn build_node_detail_summary(detail: &NodeDetail) -> NodeDetailSummary {
    let prompt = detail.prompt.as_deref().unwrap_or("");
    let stream = if !detail.streaming_content.is_empty() {
        detail.streaming_content.as_str()
    } else {
        detail
            .messages
            .last()
            .and_then(|message| message.get("content"))
            .and_then(|content| content.as_str())
            .unwrap_or("")
    };
    let has_large_event_output = detail.execution_events.iter().any(|event| {
        event
            .get("output")
            .and_then(|output| output.as_str())
            .is_some_and(|output| output.chars().count() > SUMMARY_PREVIEW_CHARS)
    });

    NodeDetailSummary {
        node_id: detail.node_id.clone(),
        node_type: serialized_string(&detail.node_type),
        status: serialized_string(&detail.status),
        agent_role: detail.agent_role.as_ref().map(serialized_string),
        provider_name: detail
            .provider
            .as_ref()
            .map(|provider| provider.name.clone()),
        prompt_size: prompt.len(),
        prompt_preview: detail.prompt.as_ref().map(|prompt| preview(prompt)),
        stream_size: stream.len(),
        stream_preview: (!stream.is_empty()).then(|| preview(stream)),
        execution_event_count: detail.execution_events.len(),
        has_large_outputs: prompt.chars().count() > SUMMARY_PREVIEW_CHARS
            || stream.chars().count() > SUMMARY_PREVIEW_CHARS
            || has_large_event_output,
        artifact_ref: detail
            .artifact_ref
            .as_ref()
            .map(|artifact| format!("{}/v{}", artifact.artifact_id, artifact.version)),
        started_at: detail.started_at.clone(),
        ended_at: detail.ended_at.clone(),
    }
}

pub(crate) fn build_session_state_node_detail(mut detail: NodeDetail) -> NodeDetail {
    detail.prompt = None;
    detail.messages.clear();
    if detail.streaming_content.chars().count() > SUMMARY_PREVIEW_CHARS {
        detail.streaming_content = preview(&detail.streaming_content);
    }
    detail.execution_events = session_state_execution_event_summaries(detail.execution_events);
    detail.permission_events.clear();
    detail
}

pub(crate) fn session_state_execution_event_summaries(
    events: Vec<serde_json::Value>,
) -> Vec<serde_json::Value> {
    events
        .into_iter()
        .map(|mut event| {
            if let Some(object) = event.as_object_mut() {
                object.insert("output".to_string(), serde_json::Value::Null);
            }
            event
        })
        .collect()
}

pub(crate) fn build_artifact_version_summary(version: &ArtifactVersion) -> ArtifactVersionSummary {
    let (markdown_size, markdown_preview) = match &version.payload {
        ArtifactPayload::Markdown { markdown, .. } => (markdown.len(), preview(markdown)),
        ArtifactPayload::WorkItemPlanCandidate { candidate } => {
            // For the candidate variant, `markdown_size`/`markdown_preview` reuse the
            // summary schema fields for compatibility: the size is the JSON length of
            // the candidate and the preview is the title of the first work item (or the
            // plan id as a fallback). In a future iteration we may rename these fields.
            let size = serde_json::to_string(candidate).map_or(0, |s| s.len());
            let preview_text = candidate
                .work_items
                .first()
                .map(|item| item.title.as_str())
                .unwrap_or(candidate.plan.id.as_str())
                .to_string();
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate } => {
            let size = serde_json::to_string(outline_candidate).map_or(0, |s| s.len());
            let preview_text = outline_candidate.outline.strategy_summary.clone();
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemPlanContextBlocker { context_blocker } => {
            let size = serde_json::to_string(context_blocker).map_or(0, |s| s.len());
            let preview_text = context_blocker.exploration_summary.clone();
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemDraftCandidate { draft_candidate } => {
            let size = serde_json::to_string(draft_candidate).map_or(0, |s| s.len());
            let preview_text = format!(
                "{}: {}",
                draft_candidate.draft_record.outline_id,
                draft_candidate.draft_record.candidate.title
            );
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemBatchState { batch_state } => {
            let size = serde_json::to_string(batch_state).map_or(0, |s| s.len());
            let preview_text = format!(
                "{}: {:?} ({} drafts)",
                batch_state.batch_id,
                batch_state.batch_status,
                batch_state.draft_records.len()
            );
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemPlanCompileReport { compile_report } => {
            let size = serde_json::to_string(compile_report).map_or(0, |s| s.len());
            let preview_text = format!(
                "{}: {:?} ({} work items)",
                compile_report.compile_id,
                compile_report.status,
                compile_report.work_item_ids.len()
            );
            (size, preview(&preview_text))
        }
    };
    ArtifactVersionSummary {
        version: version.version,
        generated_by: version.generated_by.clone(),
        reviewed_by: version.reviewed_by.clone(),
        review_verdict: version.review_verdict.clone(),
        confirmed_by: version.confirmed_by.clone(),
        is_current: version.is_current,
        created_at: version.created_at.clone(),
        source_node_id: version.source_node_id.clone(),
        markdown_size,
        markdown_preview,
    }
}

pub(crate) fn provider_conversation_session_id(
    conversations: &[ProviderConversationRef],
    role: &ProviderConversationRole,
    provider: &ProviderName,
) -> Option<String> {
    conversations
        .iter()
        .find(|conversation| &conversation.role == role && &conversation.provider == provider)
        .map(|conversation| conversation.provider_session_id.clone())
        .filter(|id| !id.trim().is_empty())
}

pub(crate) fn initial_timeline(session: &WorkspaceSession) -> (Vec<TimelineNode>, Option<String>) {
    if !session.messages.is_empty() {
        return (Vec::new(), None);
    }

    let node = TimelineNode {
        node_id: "timeline_node_001".to_string(),
        node_type: TimelineNodeType::PrepareContext,
        agent: None,
        stage: ws_stage(&WorkspaceStage::PrepareContext),
        round: None,
        status: TimelineNodeStatus::Active,
        title: "准备上下文".to_string(),
        summary: Some("等待补充上下文".to_string()),
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: session.author_provider.clone(),
            reviewer: session.reviewer_provider.clone(),
            review_rounds: session.review_rounds,
        },
        retry: None,
    };
    let active_node_id = Some(node.node_id.clone());
    (vec![node], active_node_id)
}

pub(crate) fn active_timeline_node_id(nodes: &[TimelineNode]) -> Option<String> {
    if let Some(node) = nodes.last()
        && node.node_type == TimelineNodeType::Completed
        && node.status == TimelineNodeStatus::Completed
    {
        return Some(node.node_id.clone());
    }

    nodes
        .iter()
        .rev()
        .find(|node| {
            matches!(
                node.status,
                TimelineNodeStatus::Active | TimelineNodeStatus::Paused
            )
        })
        .map(|node| node.node_id.clone())
}

pub(crate) fn recover_pending_author_choice(
    session: &WorkspaceSession,
    active_node_id: Option<&str>,
    timeline_nodes: &[TimelineNode],
) -> Option<PendingAuthorChoice> {
    let active_node_id = active_node_id?;
    let active_node = timeline_nodes
        .iter()
        .find(|node| node.node_id == active_node_id)?;
    if active_node.node_type != TimelineNodeType::AuthorRun
        || active_node.status != TimelineNodeStatus::Paused
        || !active_node
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("等待用户选择"))
    {
        return None;
    }

    let assistant_message = session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")?;
    let (prompt, options) =
        detect_author_choice_request(&assistant_message.content, &session.workspace_type)?;
    Some(PendingAuthorChoice {
        id: format!("author_choice_{}", assistant_message.id),
        prompt,
        options,
        source_node_id: Some(active_node_id.to_string()),
    })
}

pub(crate) fn latest_review_verdict_from_messages(
    messages: &[SessionMessage],
) -> Option<ReviewVerdict> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == "reviewer")
        .map(|message| WorkspaceEngine::parse_review_verdict(&message.content))
}

pub(crate) fn review_complete_event_from_verdict(
    node_id: String,
    round: u32,
    verdict: &ReviewVerdict,
) -> EngineEvent {
    EngineEvent::ReviewComplete {
        node_id,
        round,
        verdict: verdict.verdict.clone(),
        comments: verdict.comments.clone(),
        summary: verdict.summary.clone(),
        findings: verdict.findings.clone(),
        review_gate: verdict.review_gate.clone(),
        work_item_plan_review: verdict.work_item_plan_review.clone(),
    }
}

impl WorkspaceEngine {
    pub fn build_session_state(&self) -> WsOutMessage {
        let messages: Vec<WsMessageDto> = self
            .session
            .messages
            .iter()
            .map(|m| WsMessageDto {
                id: m.id.clone(),
                role: m.role.clone(),
                content: m.content.clone(),
                checkpoint_id: m.checkpoint_id.clone(),
                created_at: m.created_at.clone(),
            })
            .collect();

        let checkpoints: Vec<WsCheckpointDto> = self
            .checkpoint_store
            .list_checkpoints(&self.session.session_id)
            .unwrap_or_default()
            .into_iter()
            .map(|cp| WsCheckpointDto {
                id: cp.id,
                message_index: cp.message_index,
                stage: cp.stage,
                created_at: cp.created_at,
            })
            .collect();

        let mut timeline_node_details = HashMap::new();
        let mut timeline_node_summaries = HashMap::new();
        if let Some(store) = self.lifecycle_store.as_ref()
            && let Ok(ids) = store.list_node_detail_ids(&self.session.session_id)
        {
            let timeline_node_ids = self
                .timeline_nodes
                .iter()
                .map(|node| node.node_id.as_str())
                .collect::<HashSet<_>>();
            for id in ids {
                let Ok(detail) = store.load_node_detail(&self.session.session_id, &id) else {
                    continue;
                };
                timeline_node_summaries.insert(id.clone(), build_node_detail_summary(&detail));
                if self.session.workspace_type == WorkspaceType::WorkItemPlan
                    && timeline_node_ids.contains(id.as_str())
                {
                    timeline_node_details.insert(id, build_session_state_node_detail(detail));
                }
            }
        }
        let artifact_version_summaries = self
            .artifact_versions
            .iter()
            .map(build_artifact_version_summary)
            .collect();

        WsOutMessage::SessionState {
            session_id: self.session.session_id.clone(),
            workspace_type: self.session.workspace_type.clone(),
            stage: self.session.stage.as_str().to_string(),
            superpowers_enabled: self.session.superpowers_enabled,
            openspec_enabled: self.session.openspec_enabled,
            messages,
            checkpoints,
            artifact: self.session.artifact.clone(),
            providers: WsProviderConfig {
                author: self.session.author_provider.clone(),
                reviewer: self.session.reviewer_provider.clone(),
            },
            timeline_nodes: self.timeline_nodes.clone(),
            active_node_id: self.active_node_id.clone(),
            artifact_versions: Vec::new(),
            artifact_version_summaries,
            timeline_node_details,
            timeline_node_summaries,
            active_run_id: self.active_run_id.clone(),
        }
    }
}
