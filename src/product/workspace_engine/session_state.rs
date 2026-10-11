use super::*;
use crate::web::workspace_ws_types::{LcResumeRecoveryDto, WsPendingChoiceRequest};

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

fn should_inline_work_item_plan_detail(node_type: &TimelineNodeType) -> bool {
    matches!(
        node_type,
        TimelineNodeType::WorkItemPlanOutlineRun
            | TimelineNodeType::WorkItemPlanOutlineReview
            | TimelineNodeType::WorkItemDraftRun
            | TimelineNodeType::WorkItemDraftReview
            | TimelineNodeType::WorkItemBatchRun
            | TimelineNodeType::WorkItemBatchReview
    )
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
                draft_candidate
                    .draft_record
                    .candidate
                    .canonical_contract_candidate
                    .identity
                    .title
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
        ArtifactPayload::WorkItemPlanProjection { projection } => {
            let size = serde_json::to_string(projection).map_or(0, |s| s.len());
            (size, preview(&projection.id))
        }
        ArtifactPayload::WorkItemProjection { projection } => {
            let size = serde_json::to_string(projection).map_or(0, |s| s.len());
            (size, preview(&projection.id))
        }
        ArtifactPayload::WorkItemRevisionHistory { history } => {
            let size = serde_json::to_string(history).map_or(0, |s| s.len());
            let preview_text = format!("{} history entries", history.entries.len());
            (size, preview(&preview_text))
        }
        ArtifactPayload::ProjectionValidation { report } => {
            let size = serde_json::to_string(report).map_or(0, |s| s.len());
            let preview_text = format!("{} projection findings", report.findings.len());
            (size, preview(&preview_text))
        }
        ArtifactPayload::PlanAmendmentManifest { manifest } => {
            let size = serde_json::to_string(manifest).map_or(0, |s| s.len());
            (size, preview(&manifest.id))
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
            permission_modes: session.permission_modes.clone(),
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
    workspace_type: &WorkspaceType,
) -> Option<ReviewVerdict> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == "reviewer")
        .map(|message| {
            WorkspaceEngine::parse_review_verdict_for_workspace(&message.content, workspace_type)
        })
}

pub(crate) fn latest_review_verdict_from_node_details(
    lifecycle_store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    session_id: &str,
    timeline_nodes: &[TimelineNode],
) -> Option<ReviewVerdict> {
    timeline_nodes
        .iter()
        .rev()
        .filter(|node| {
            matches!(
                node.node_type,
                TimelineNodeType::ReviewerRun
                    | TimelineNodeType::WorkItemPlanOutlineReview
                    | TimelineNodeType::WorkItemDraftReview
                    | TimelineNodeType::WorkItemBatchReview
            )
        })
        .find_map(|node| {
            let mut detail = lifecycle_store
                .load_node_detail_for_issue_session(project_id, issue_id, session_id, &node.node_id)
                .ok()?;
            let persisted_verdict = detail.verdict.clone()?;
            let verdict = match deserialize_historical_review_verdict(persisted_verdict.clone()) {
                Ok(verdict) => verdict,
                Err(error) => {
                    tracing::warn!(
                        node_id = %node.node_id,
                        %error,
                        "persisted review verdict failed normalization; entering user triage"
                    );
                    failed_historical_review_verdict(error.to_string())
                }
            };
            let normalized_verdict =
                serde_json::to_value(&verdict).expect("ReviewVerdict serialization must succeed");
            if normalized_verdict != persisted_verdict {
                detail.verdict = Some(normalized_verdict);
                if let Err(error) =
                    lifecycle_store.save_node_detail(session_id, &node.node_id, &detail)
                {
                    tracing::warn!(
                        node_id = %node.node_id,
                        %error,
                        "failed to persist normalized review verdict"
                    );
                }
            }
            Some(verdict)
        })
}

fn failed_historical_review_verdict(error: String) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "历史审核记录无法归一化，请人工确认。".to_string(),
        summary: "历史审核记录需要人工确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: Some(StructuredOutputDiagnostic {
            code: "historical_review_normalization_failed".to_string(),
            message: error,
            repair_attempted: false,
            repair_succeeded: false,
            raw_output_preview: None,
        }),
    }
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
        structured_output_diagnostic: verdict.structured_output_diagnostic.clone(),
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
        if let Some(store) = self.lifecycle_store.as_ref() {
            if self.session.workspace_type == WorkspaceType::WorkItemPlan {
                for node in self
                    .timeline_nodes
                    .iter()
                    .filter(|node| should_inline_work_item_plan_detail(&node.node_type))
                {
                    let Ok(detail) = store.load_node_detail_for_issue_session(
                        &self.session.project_id,
                        &self.session.issue_id,
                        &self.session.session_id,
                        &node.node_id,
                    ) else {
                        continue;
                    };
                    timeline_node_details.insert(
                        node.node_id.clone(),
                        build_session_state_node_detail(detail),
                    );
                }
            } else if let Ok(ids) = store.list_node_detail_ids_for_issue_session(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.session_id,
            ) {
                for id in ids {
                    let Ok(detail) = store.load_node_detail_for_issue_session(
                        &self.session.project_id,
                        &self.session.issue_id,
                        &self.session.session_id,
                        &id,
                    ) else {
                        continue;
                    };
                    timeline_node_summaries.insert(id.clone(), build_node_detail_summary(&detail));
                }
            }
        }
        let artifact_version_summaries = self
            .artifact_versions
            .iter()
            .map(build_artifact_version_summary)
            .collect();
        let artifact_versions = if self.session.workspace_type == WorkspaceType::WorkItemPlan {
            self.artifact_versions
                .iter()
                .filter(|version| {
                    matches!(
                        version.payload,
                        ArtifactPayload::WorkItemPlanProjection { .. }
                            | ArtifactPayload::WorkItemProjection { .. }
                            | ArtifactPayload::WorkItemRevisionHistory { .. }
                            | ArtifactPayload::ProjectionValidation { .. }
                            | ArtifactPayload::PlanAmendmentManifest { .. }
                    )
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        WsOutMessage::SessionState {
            session_id: self.session.session_id.clone(),
            connection_id: None,
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
            artifact_versions,
            artifact_version_summaries,
            timeline_node_details,
            timeline_node_summaries,
            active_run_id: self.active_run_id.clone(),
            human_presentation_revisions: self.latest_human_presentation_revisions(),
            reviewer_enabled_at_start: self.session.reviewer_enabled_at_start,
            recoverable_interrupted_run: self.recoverable_interrupted_run(),
            plan_repair: self.plan_repair_snapshot.clone().map(|mut snapshot| {
                snapshot.timeline_nodes = self.timeline_nodes.clone();
                Box::new(snapshot)
            }),
            session_status: self.session.session_status.clone(),
            flow_kind: self.session.flow_kind,
            run_policy: self.session.run_policy,
            run_history: self.session.run_history.clone(),
            review_invocation_scope: self.session.review_invocation_scope.clone().map(Box::new),
            human_gate_snapshot: self.session.human_gate_snapshot.clone(),
            repair_reservation: self.session.repair_reservation.clone().map(Box::new),
            policy_diagnostics: self.session.policy_diagnostics.clone(),
            provider_start_ledger: self.session.provider_start_ledger.clone(),
            single_candidate_phase: self.session.single_candidate_phase.clone(),
            work_item_plan_source_revision_ref: self
                .session
                .work_item_plan_source_revision_ref
                .clone(),
            plan_candidate_ir_ref: self.session.plan_candidate_ir_ref.clone(),
            mechanical_report_ref: self.session.mechanical_report_ref.clone(),
            publication_provenance_ref: self.session.publication_provenance_ref.clone(),
            pending_choice_requests: self.pending_choice_requests_projection(),
            // P0 1.2：引擎自建帧默认 client/off；对外出口由 manager 以 durable
            // enrollment 事实二次覆盖，读取失败置 None 保持未知。
            automation: Some(
                crate::product::models::automation::AutomationOwnership::client_default(),
            ),
            // A1c（Task 2.2）：durable LC 恢复纯投影——直取 runtime session
            // 镜像的 durable 字段（2.1 from_record 逐字段透传），引擎不写
            // 恢复业务；对外出口（manager/router）不二次覆写。
            lc_resume_recovery: self
                .session
                .lc_resume_recovery
                .as_ref()
                .map(LcResumeRecoveryDto::from_record),
        }
    }
    /// F-27：session_state 顶层挂起 choice 全量投影，双来源统一：
    /// provider pending（drive 循环登记簿快照——run 持锁窗口经 durable 投影
    /// 临时 engine 读同一登记簿，见 provider_drive.rs 文件头注释）与
    /// TextFallback pending（引擎 `pending_author_choice`——文本提问回退，
    /// router 侧 F-24 挂起帧不登记该源，投影是其唯一恢复来源）。
    /// 与 provider_drive.rs 文本回退事件（EngineEvent::ChoiceRequest
    /// source=TextFallback）逐字段同构；id 去重后按 id 稳定排序。
    fn pending_choice_requests_projection(&self) -> Vec<WsPendingChoiceRequest> {
        let mut requests: Vec<WsPendingChoiceRequest> =
            super::provider_drive::pending_choice_requests_snapshot(&self.session.session_id)
                .into_iter()
                .map(|entry| {
                    let questions = entry.request.effective_questions();
                    WsPendingChoiceRequest {
                        id: entry.request.id,
                        prompt: entry.request.prompt.clone(),
                        options: entry
                            .request
                            .options
                            .into_iter()
                            .map(crate::web::workspace_ws_handler::ws_choice_option)
                            .collect(),
                        allow_multiple: entry.request.allow_multiple,
                        allow_free_text: entry.request.allow_free_text,
                        questions: questions
                            .into_iter()
                            .map(crate::web::workspace_ws_handler::ws_choice_question)
                            .collect(),
                        source: entry.request.source.as_str().to_string(),
                        // F-59：登记时刻与发问角色随投影带出（等待提示条数据源）。
                        created_at_ms: Some(entry.created_at_ms),
                        role: entry.role.to_string(),
                        // P0 1.3：引擎不知道 manager run 化身——由 manager
                        // 在对外出口注入；None=旧载荷/无活跃 run。
                        expected_run_id: None,
                    }
                })
                .collect();
        if let Some(author_choice) = self.pending_author_choice.as_ref() {
            let fallback = WsPendingChoiceRequest {
                id: author_choice.id.clone(),
                prompt: author_choice.prompt.clone(),
                options: author_choice
                    .options
                    .clone()
                    .into_iter()
                    .map(crate::web::workspace_ws_handler::ws_choice_option)
                    .collect(),
                allow_multiple: false,
                allow_free_text: true,
                questions: vec![crate::web::workspace_ws_handler::ws_choice_question(
                    ChoiceQuestionData {
                        id: "default".to_string(),
                        prompt: author_choice.prompt.clone(),
                        options: author_choice.options.clone(),
                        allow_multiple: false,
                        allow_free_text: true,
                    },
                )],
                source: ChoiceRequestSource::TextFallback.as_str().to_string(),
                // F-59：TextFallback 无 provider 等待界——不带 created_at_ms
                //（前端回退首见时刻）；发问方恒为 author。
                created_at_ms: None,
                role: "author".to_string(),
                expected_run_id: None,
            };
            if !requests.iter().any(|request| request.id == fallback.id) {
                requests.push(fallback);
            }
        }
        requests.sort_by(|a, b| a.id.cmp(&b.id));
        requests
    }

    /// F-25b：HTTP confirm（handler 层直写 durable）后的内存同步面。AuthorConfirm/
    /// HumanConfirm 静止态引擎内存与 durable 一致，唯一差异是该外部写入：按 record
    /// 同步 session_status/门快照并补登尾部新增消息（确认审计行），使随后的全量
    pub(crate) fn apply_external_confirm_record(&mut self, record: &WorkspaceSessionRecord) {
        self.session.session_status = record.status.clone();
        // fix round 1（k3 P2）：stage 随 status 一致收敛——与 from_record 的
        // workspace_stage_for_status 投影同口径（Confirmed→Completed），否则广播
        // confirmed+author_confirm 组合与重连 tab 的 durable 投影互相矛盾。
        self.session.stage = workspace_stage_for_status(&record.status);
        self.session.human_gate_snapshot = record.human_gate_snapshot.clone();
        let in_memory_len = self.session.messages.len();
        let durable_len = record.messages.len();
        if durable_len > in_memory_len {
            for (offset, message) in record.messages[in_memory_len..].iter().enumerate() {
                self.session.messages.push(SessionMessage {
                    id: format!("msg_{:03}", in_memory_len + offset + 1),
                    role: message.role.clone(),
                    content: message.content.clone(),
                    checkpoint_id: None,
                    created_at: message.created_at.clone(),
                });
            }
        }
    }
}

#[cfg(test)]
mod anchor_t03_tests {
    //! A1c（Task 2.2）：build_session_state 取 durable 恢复字段的红/绿锚点。
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::models::{
        LcFreshOperationResult, LcFreshOperationState, LcRecoveryBusinessContext,
        LcRecoveryDraftRef, LcRecoveryRunContext, LcResumeRecoveryRecord, LcResumeRecoveryState,
        ProviderConversationRole, ProviderName, WorkspaceRolePermissionModes,
        WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType, WorkItemDraftStatus,
    };
    use crate::product::work_item_plan_policy::{RunHistory, RunPolicy, WorkItemPlanFlowKind};
    use crate::product::workspace_engine::{WorkspaceEngine, WorkspaceSession};
    use crate::web::workspace_ws_types::WsOutMessage;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    fn anchor_t03_record(recovery: Option<LcResumeRecoveryRecord>) -> WorkspaceSessionRecord {
        WorkspaceSessionRecord {
            id: "workspace_session_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "wi_core".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            status: WorkspaceSessionStatus::Running,
            author_provider: ProviderName::Codex,
            reviewer_provider: None,
            review_rounds: 0,
            permission_modes: WorkspaceRolePermissionModes::default(),
            provisional_reviewer_provider: None,
            reviewer_enabled_at_start: None,
            superpowers_enabled: false,
            openspec_enabled: false,
            flow_kind: WorkItemPlanFlowKind::Legacy,
            run_policy: RunPolicy::Interactive,
            run_history: RunHistory::default(),
            review_invocation_scope: None,
            human_gate_snapshot: None,
            repair_reservation: None,
            human_gate_reservation: None,
            policy_diagnostics: Vec::new(),
            provider_start_ledger: Vec::new(),
            single_candidate_phase: None,
            work_item_plan_source_revision_ref: None,
            plan_candidate_ir_ref: None,
            mechanical_report_ref: None,
            publication_provenance_ref: None,
            approval_attempt_id: None,
            approved_at: None,
            compile_reservation: None,
            sc_recovery_claim: None,
            work_item_runtime_binding: None,
            work_item_child_binding: None,
            provider_conversations: Vec::new(),
            logical_launch_fingerprints: Vec::new(),
            superseded_native_sessions: Vec::new(),
            lc_resume_recovery: recovery,
            messages: Vec::new(),
            created_at: "2026-10-11T00:00:00Z".to_string(),
            updated_at: "2026-10-11T00:00:00Z".to_string(),
        }
    }

    fn anchor_t03_recovery() -> LcResumeRecoveryRecord {
        LcResumeRecoveryRecord {
            schema_version: 1,
            recovery_id: "recovery_0001".to_string(),
            failed_provider_node_id: "node_0001".to_string(),
            reason_code: "resume_fingerprint_mismatch".to_string(),
            role: ProviderConversationRole::Author,
            provider: ProviderName::Codex,
            superseded_native_session_id: Some("native_session_0001".to_string()),
            previous_fingerprint_digest: Some("sha256:secret-full".to_string()),
            previous_session_status: WorkspaceSessionStatus::Running,
            previous_stage: "running".to_string(),
            run_context: LcRecoveryRunContext::LegacyDraft {
                business: LcRecoveryBusinessContext {
                    entity_id: "wi_core".to_string(),
                    plan_id: Some("work_item_plan_0001".to_string()),
                    workspace_type: WorkspaceType::WorkItemPlan,
                    business_repository_physical_id: None,
                    business_repository_logical_id: None,
                    business_checkout_id: None,
                    business_repository_path: None,
                    source_story_ids: Vec::new(),
                    source_design_ids: Vec::new(),
                    plan_options_ref: None,
                    artifact_refs: Vec::new(),
                    review_refs: Vec::new(),
                    author_provider: ProviderName::Codex,
                    effective_reviewer_provider: None,
                    original_node_id: Some("node_0001".to_string()),
                    original_role_run_seq: Some(7),
                },
                generation_round_id: "round_0001".to_string(),
                active_outline_id: "outline_0001".to_string(),
                draft_refs: vec![LcRecoveryDraftRef {
                    draft_id: "draft_0001".to_string(),
                    status: WorkItemDraftStatus::Draft,
                    version: Some(1),
                    hash: Some("sha256:draft".to_string()),
                }],
                pending_feedback: None,
                outline_ref: None,
                candidate_ref: None,
            },
            state: LcResumeRecoveryState::Waiting,
            operation_id: Some("operation_0001".to_string()),
            fresh_node_id: None,
            fresh_native_session_id: None,
            fresh_role_run_seq: None,
            fresh_operations: vec![LcFreshOperationResult {
                operation_id: "operation_0001".to_string(),
                state: LcFreshOperationState::Failed,
                fresh_node_id: None,
                reason_code: Some("provider_prepare_failed".to_string()),
                role_run_seq: None,
                accepted_at: "2026-10-11T00:03:00Z".to_string(),
                updated_at: "2026-10-11T00:04:00Z".to_string(),
            }],
            detected_at: "2026-10-11T00:02:00Z".to_string(),
            updated_at: "2026-10-11T00:04:00Z".to_string(),
        }
    }

    // WorkItemPlan 会话的 build_session_state 走持久读侧（revision_store 等），
    // 按引擎测试惯例用隔离 TempDir 的 new_persistent 构造。
    fn anchor_t03_engine(record: WorkspaceSessionRecord) -> (WorkspaceEngine, TempDirGuard) {
        let tmp = TempDir::new().unwrap();
        let lifecycle = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let (event_tx, _event_rx) = mpsc::channel(1);
        let engine = WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(tmp.path().to_path_buf())),
            lifecycle,
            event_tx,
            WorkspaceSession::from_record(record),
        );
        (engine, TempDirGuard(tmp))
    }

    /// 防止 TempDir 过早 drop 删除引擎仍要读取的隔离目录。
    struct TempDirGuard(TempDir);

    #[test]
    fn anchor_t03_build_session_state_projects_durable_recovery() {
        // durable 字段（from_record 透传）→ SessionState 纯投影。
        let (engine, _tmp_guard) = anchor_t03_engine(anchor_t03_record(Some(anchor_t03_recovery())));
        match engine.build_session_state() {
            WsOutMessage::SessionState {
                lc_resume_recovery: Some(dto),
                ..
            } => {
                assert_eq!(dto.recovery_id, "recovery_0001");
                assert_eq!(dto.state, LcResumeRecoveryState::Waiting);
                assert_eq!(dto.reason_code, "resume_fingerprint_mismatch");
                assert_eq!(dto.provider, ProviderName::Codex);
                assert_eq!(dto.failed_node_id, "node_0001");
                assert_eq!(dto.run_kind, "legacy_draft");
                // A/B 发布纪律：waiting allowed 为空直到 B 齐。
                assert!(dto.allowed_actions.is_empty());
                let last_operation = dto.last_operation.expect("last_operation projected");
                assert_eq!(last_operation.operation_id, "operation_0001");
                assert_eq!(last_operation.state, LcFreshOperationState::Failed);
                assert_eq!(
                    last_operation.reason.as_deref(),
                    Some("provider_prepare_failed")
                );
                assert!(last_operation.fresh_node.is_none());
            }
            _ => panic!("expected SessionState"),
        }

        // 无恢复的 durable record：字段 None（wire 省略，旧客户端零变化）。
        let (plain, _tmp_guard) = anchor_t03_engine(anchor_t03_record(None));
        match plain.build_session_state() {
            WsOutMessage::SessionState {
                lc_resume_recovery: None,
                ..
            } => {}
            _ => panic!("expected SessionState"),
        }
    }
}
