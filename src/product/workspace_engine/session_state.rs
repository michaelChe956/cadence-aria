use super::*;
pub(crate) use super::sc_failure_artifact_summary::build_artifact_version_summary;
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

include!("session_state_parts/historical_review.inc.rs");
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
        // A4（SC 失败修复包）只在严格发布回执通过后进入公开视图：pending
        // （缺回执）不出现，坏回执/摘要显式投影为 policy_diagnostics——不吞成
        // 空列表成功，普通 entries 保留。内存 raw 版本表不被过滤结果覆盖。
        let (published_versions, publication_diagnostic) = self.sc_published_versions();
        let mut policy_diagnostics = self.session.policy_diagnostics.clone();
        if let Some(diagnostic) = publication_diagnostic {
            policy_diagnostics.push(diagnostic);
        }
        let artifact_version_summaries = published_versions
            .iter()
            .map(build_artifact_version_summary)
            .collect();
        let artifact_versions = if self.session.workspace_type == WorkspaceType::WorkItemPlan {
            published_versions
                .iter()
                .filter(|version| {
                    matches!(
                        version.payload,
                        ArtifactPayload::WorkItemPlanProjection { .. }
                            | ArtifactPayload::WorkItemProjection { .. }
                            | ArtifactPayload::WorkItemRevisionHistory { .. }
                            | ArtifactPayload::ProjectionValidation { .. }
                            | ArtifactPayload::PlanAmendmentManifest { .. }
                            | ArtifactPayload::ScFailureRepairPackage { .. }
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
            human_gate_turn_rejection: self.latest_human_gate_turn_rejection_projection(),
            candidate_compile_failure: self.candidate_compile_failure_projection(),
            repair_reservation: self.session.repair_reservation.clone().map(Box::new),
            policy_diagnostics,
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
            // REQ-HGC-04：引擎自建帧不判「继续自动化」入口（引擎不知 durable
            // enrollment）——对外出口（manager/router）以 durable 事实注入；
            // 缺席即 None（未知，前端不给假指引）。
            plan_continue_available: None,
            // A1c（Task 2.2）：durable LC 恢复纯投影——直取 runtime session
            // 镜像的 durable 字段（2.1 from_record 逐字段透传），引擎不写
            // 恢复业务；对外出口（manager/router）不二次覆写。
            lc_resume_recovery: self
                .session
                .lc_resume_recovery
                .as_ref()
                .map(LcResumeRecoveryDto::from_record),
            structural_governance: self
                .session
                .structural_governance_state
                .as_ref()
                .map(|state| {
                    crate::web::workspace_ws_types::project_structural_governance_snapshot(
                        state,
                        self.session.human_gate_snapshot.as_ref(),
                        &self.session.provider_start_ledger,
                        self.session.work_item_plan_source_revision_ref.as_deref(),
                        self.session.plan_candidate_ir_ref.as_deref(),
                        self.session.mechanical_report_ref.as_deref(),
                    )
                }),
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
        ProviderConversationRole, ProviderName, WorkItemDraftStatus, WorkspaceRolePermissionModes,
        WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
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
            author_rewrite_budget: None,
            policy_diagnostics: Vec::new(),
            provider_start_ledger: Vec::new(),
            sc_generation_teaching_windows: Vec::new(),
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
            structural_governance_state: None,
            review_identity_id: String::new(),
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
    struct TempDirGuard(#[allow(dead_code)] TempDir); // 字段仅保活,永不读取。

    #[test]
    fn anchor_t03_build_session_state_projects_durable_recovery() {
        // durable 字段（from_record 透传）→ SessionState 纯投影。
        let (engine, _tmp_guard) =
            anchor_t03_engine(anchor_t03_record(Some(anchor_t03_recovery())));
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

/// SG7a（plan v1.1 Task SG7a Step 1）：durable→runtime→SessionState 真实
/// 生产链五态锚点（engine 域）——真实 store CAS 物化聚合状态，经
/// from_record／引擎纯构造 build_session_state／live sync helper／reload
/// 逐字段相等；manager production `load_durable_workspace_projection`
/// 同值链在 manager 域锚定（durable_projection.rs cfgtests），不手工填
/// wire。
#[cfg(test)]
mod sg_07a_tests {
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::json_store::write_json;
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::lifecycle_store::inputs::CreateWorkspaceSessionInput;
    use crate::product::models::{
        HumanGateReservation, HumanGateTurn, HumanGateTurnStatus, ProviderName,
        ResolvedStructureFact, StructuralAttemptReason, StructuralCheckpoint,
        StructuralCheckpointStatus, StructuralGovernanceState, StructuralGrantPhase,
        StructuralRevisionGrant, UserBudgetReceipt, UserBudgetReceiptKind, WorkspaceSessionRecord,
        WorkspaceSessionStatus, WorkspaceType,
    };
    use crate::product::work_item_plan_policy::{
        HumanGateSnapshot, HumanReason, ProviderStartLedgerEntry, WorkItemPlanFlowKind,
    };
    use crate::product::workspace_engine::{WorkspaceEngine, WorkspaceSession};
    use crate::web::workspace_ws_types::{
        StructuralGovernanceSnapshot, WsOutMessage, project_structural_governance_snapshot,
    };
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    struct Sg07aFixture {
        _tmp: TempDir,
        paths: ProductAppPaths,
        lifecycle: LifecycleStore,
        birth: WorkspaceSessionRecord,
    }

    fn sg_07a_now() -> String {
        "2026-10-14T00:00:00Z".to_string()
    }

    fn sg_07a_record_path(
        paths: &ProductAppPaths,
        record: &WorkspaceSessionRecord,
    ) -> std::path::PathBuf {
        paths
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id))
    }

    /// SG5.3/SG6a.1 同款 fixture 注入：read-modify-write 保留既有 CAS 事实
    ///（hold/ledger/预算），只补 grant/phase/checkpoint 等尚无生产 producer
    /// 的耐久形状（producer 归 SG6a.3）。
    fn sg_07a_mutate<F>(fixture: &Sg07aFixture, mutate: F) -> WorkspaceSessionRecord
    where
        F: FnOnce(&mut WorkspaceSessionRecord),
    {
        let mut record = fixture
            .lifecycle
            .get_workspace_session(&fixture.birth.id)
            .expect("latest record");
        mutate(&mut record);
        write_json(&sg_07a_record_path(&fixture.paths, &record), &record)
            .expect("persist fixture mutation");
        record
    }

    /// SC WorkItemPlan 会话 + 人工门快照（预算 3、accepted 从 0 起）——
    /// 结构/预算 CAS 的真实前置；三 refs 在 birth 即固化（后续 CAS 沿用）。
    fn sg_07a_sc_fixture(project_id: &str) -> Sg07aFixture {
        let tmp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        let mut record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: project_id.to_string(),
                issue_id: "issue_sg07a".to_string(),
                entity_id: "plan_sg07a".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: None,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("durable session record");
        record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
        record.status = WorkspaceSessionStatus::WaitingForHuman;
        record.human_gate_snapshot = Some(HumanGateSnapshot {
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: 0,
            manual_repairs_remaining: 3,
            trigger: HumanReason::NativeHumanRequired,
            resumable: false,
            accepted_feedback_turns: Some(0),
            candidate_recovery: None,
        });
        record.work_item_plan_source_revision_ref = Some("sg07a-source-ref".to_string());
        record.plan_candidate_ir_ref = Some("sg07a-ir-ref".to_string());
        record.mechanical_report_ref = Some("sg07a-report-ref".to_string());
        write_json(&sg_07a_record_path(&paths, &record), &record).expect("persist birth");
        Sg07aFixture {
            _tmp: tmp,
            paths,
            lifecycle,
            birth: record,
        }
    }

    fn sg_07a_turn(turn_id: &str, command_id: &str, session_id: &str) -> HumanGateTurn {
        HumanGateTurn {
            turn_id: turn_id.to_string(),
            session_id: session_id.to_string(),
            command_id: command_id.to_string(),
            feedback_text: "修复互依赖环".to_string(),
            status: HumanGateTurnStatus::Reserved,
            attempt_no: 0,
            budget_reserved: 1,
            source_hash: "sha256:sg07a-failing-source".to_string(),
            result_artifact_ref: None,
            failure_class: None,
            created_at: sg_07a_now(),
            updated_at: sg_07a_now(),
        }
    }

    /// 真实 store CAS：接受反馈 → 物化聚合状态 + 活动 hold + 预算/ledger
    /// 预留（SG4a 权威链，非注入）。
    fn sg_07a_reserve(
        fixture: &Sg07aFixture,
        expected: &WorkspaceSessionRecord,
        turn_id: &str,
        command_id: &str,
        start_key: &str,
    ) -> WorkspaceSessionRecord {
        let turn = sg_07a_turn(turn_id, command_id, &expected.id);
        let reservation = HumanGateReservation {
            command_id: command_id.to_string(),
            turn_id: turn_id.to_string(),
            provider_start_idempotency_key: start_key.to_string(),
            reserved_at: sg_07a_now(),
        };
        let (next, _) = fixture
            .lifecycle
            .compare_and_reserve_human_gate_turn(expected, turn, reservation)
            .expect("reserve human gate turn");
        next
    }

    fn sg_07a_receipt(
        kind: UserBudgetReceiptKind,
        turn_id: &str,
        command_id: &str,
    ) -> UserBudgetReceipt {
        UserBudgetReceipt {
            kind,
            turn_id: turn_id.to_string(),
            command_id: command_id.to_string(),
            attempt_reason: Some(StructuralAttemptReason::HumanRevision),
            source_revision_ref: Some("sg07a-source-ref".to_string()),
            plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
            structure_report_ref: Some("sg07a-report-ref".to_string()),
            context_digest: Some("sha256:sg07a-context".to_string()),
            reason: Some("sg07a five-state fixture".to_string()),
            settled_at: sg_07a_now(),
        }
    }

    fn sg_07a_grant(
        turn_id: &str,
        command_id: &str,
        phase: StructuralGrantPhase,
        attempt_no: u32,
    ) -> StructuralRevisionGrant {
        StructuralRevisionGrant {
            grant_id: format!("structural_grant_{turn_id}"),
            authorization_hash: "sha256:sg07a-authorization".to_string(),
            turn_id: Some(turn_id.to_string()),
            command_id: Some(command_id.to_string()),
            candidate_source_revision_ref: Some("sg07a-source-ref".to_string()),
            plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
            mechanical_report_ref: Some("sg07a-report-ref".to_string()),
            phase: Some(phase),
            attempt_no,
            prompt_ref: if attempt_no >= 1 {
                Some("sg07a-frozen-prompt-ref".to_string())
            } else {
                None
            },
            prompt_digest: if attempt_no >= 1 {
                Some("sha256:sg07a-prompt".to_string())
            } else {
                None
            },
            instruction_digest: if attempt_no >= 1 {
                Some("sha256:sg07a-instruction".to_string())
            } else {
                None
            },
            failing_source_hash: if attempt_no >= 1 {
                Some("sha256:sg07a-failing-source".to_string())
            } else {
                None
            },
            review_identity: None,
            granted_at: sg_07a_now(),
        }
    }

    /// manager 生产纯投影同源的引擎构造（ProjectionOnly 零恢复写）。
    fn sg_07a_projection_engine(
        paths: &ProductAppPaths,
        record: &WorkspaceSessionRecord,
    ) -> WorkspaceEngine {
        let lifecycle = LifecycleStore::new(paths.clone());
        let (event_tx, _event_rx) = mpsc::channel(1);
        let checkpoint_store = Arc::new(CheckpointStore::new(
            paths.issue_lifecycle_root(&record.project_id, &record.issue_id),
        ));
        let session = WorkspaceSession::from_record(record.clone());
        WorkspaceEngine::new_persistent_for_projection(
            checkpoint_store,
            lifecycle,
            event_tx,
            session,
        )
        .expect("projection engine")
    }

    fn sg_07a_snapshot(frame: &WsOutMessage) -> Option<StructuralGovernanceSnapshot> {
        match frame {
            WsOutMessage::SessionState {
                structural_governance,
                ..
            } => structural_governance.clone(),
            other => panic!("expected SessionState, got {other:?}"),
        }
    }

    /// 指定 record 的引擎纯构造（ProjectionOnly 零恢复写）→
    /// build_session_state 快照。
    fn sg_07a_built_snapshot(
        fixture: &Sg07aFixture,
        record: &WorkspaceSessionRecord,
    ) -> Option<StructuralGovernanceSnapshot> {
        let engine = sg_07a_projection_engine(&fixture.paths, record);
        sg_07a_snapshot(&engine.build_session_state())
    }

    /// 每态同值链：from_record 镜像 → 引擎纯构造 build → live sync
    /// helper → reload，逐字段相等＋快照事实与 durable
    /// 单源对账（accepted 沿 gate、actual 沿 ledger、refs 沿 record）。
    fn sg_07a_assert_chain(fixture: &Sg07aFixture, latest: &WorkspaceSessionRecord) {
        // ① from_record 显式逐字段镜像。
        let runtime = WorkspaceSession::from_record(latest.clone());
        assert_eq!(
            runtime.structural_governance_state, latest.structural_governance_state,
            "from_record 必须显式移入结构聚合镜像"
        );
        assert_eq!(runtime.human_gate_snapshot, latest.human_gate_snapshot);
        assert_eq!(runtime.provider_start_ledger, latest.provider_start_ledger);
        assert_eq!(
            runtime.work_item_plan_source_revision_ref,
            latest.work_item_plan_source_revision_ref
        );
        assert_eq!(runtime.plan_candidate_ir_ref, latest.plan_candidate_ir_ref);
        assert_eq!(runtime.mechanical_report_ref, latest.mechanical_report_ref);

        // ② 引擎纯构造（ProjectionOnly）→ build_session_state。
        let engine = sg_07a_projection_engine(&fixture.paths, latest);
        let built = sg_07a_snapshot(&engine.build_session_state())
            .expect("build_session_state must project structural governance");

        // ③ live CAS→sync：旧镜像 runtime 经 sync helper 吸收同 scope 最新
        //    record（结构 state＋同次三 refs＋原 gate/ledger）。
        let mut synced = WorkspaceSession::from_record(fixture.birth.clone());
        synced
            .sync_structural_governance_from_record(latest)
            .expect("same-scope sync");
        assert_eq!(
            synced.structural_governance_state, latest.structural_governance_state,
            "sync helper 必须复制结构聚合状态"
        );
        assert_eq!(synced.human_gate_snapshot, latest.human_gate_snapshot);
        assert_eq!(synced.provider_start_ledger, latest.provider_start_ledger);
        assert_eq!(
            synced.work_item_plan_source_revision_ref,
            latest.work_item_plan_source_revision_ref
        );
        assert_eq!(synced.plan_candidate_ir_ref, latest.plan_candidate_ir_ref);
        assert_eq!(synced.mechanical_report_ref, latest.mechanical_report_ref);

        // ④ reload_session_from_record（保留 repository_path 模式）同值。
        let mut live_engine = sg_07a_projection_engine(&fixture.paths, &fixture.birth);
        live_engine.reload_session_from_record(latest.clone());
        let reloaded = sg_07a_snapshot(&live_engine.build_session_state())
            .expect("reload + build must project structural governance");
        assert_eq!(built, reloaded, "reload 后的快照必须与纯构造同值");

        // ⑤ manager production 纯投影（最新 durable record）的同值链在
        //    manager 域内锚定（durable_projection.rs cfgtests
        //    `sg_07a_live_cas_and_reload_match_latest_durable_projection`，
        //    workspace_session::manager 模块对 engine 域私有）。
        // ⑥ 快照事实与 durable 单源逐字段对账。
        let state = latest
            .structural_governance_state
            .as_ref()
            .expect("five states always carry aggregate state");
        assert_eq!(built.user_budget_total, state.user_budget_total);
        assert_eq!(built.available_user_budget, state.available_user_budget());
        assert_eq!(built.active_hold_count, state.active_hold_count());
        assert_eq!(built.user_holds, state.user_holds);
        assert_eq!(built.user_receipts, state.user_receipts);
        assert_eq!(built.structure_checkpoints, state.structure_checkpoints);
        assert_eq!(
            built.resolved_structure_facts,
            state.resolved_structure_facts
        );
        assert_eq!(
            built.current_grant.as_ref().map(|grant| grant.attempt_no),
            state.current_grant.as_ref().map(|grant| grant.attempt_no)
        );
        assert_eq!(
            built.current_grant.as_ref().and_then(|grant| grant.phase),
            state.current_grant.as_ref().and_then(|grant| grant.phase)
        );
        assert_eq!(
            built.source_ref.as_deref(),
            latest.work_item_plan_source_revision_ref.as_deref()
        );
        assert_eq!(
            built.plan_candidate_ir_ref.as_deref(),
            latest.plan_candidate_ir_ref.as_deref()
        );
        assert_eq!(
            built.structure_report_ref.as_deref(),
            latest.mechanical_report_ref.as_deref()
        );
        assert_eq!(
            built.context_digest.as_deref(),
            state.current_context_digest.as_deref()
        );
        assert_eq!(
            built.accepted_feedback_turns,
            latest
                .human_gate_snapshot
                .as_ref()
                .and_then(|gate| gate.accepted_feedback_turns),
            "accepted 单源沿 HumanGateSnapshot"
        );
        assert_eq!(
            built.actual_provider_starts,
            latest
                .provider_start_ledger
                .iter()
                .filter(|entry| entry.is_actual_launch_observation())
                .count() as u32,
            "actual start 单源沿 provider_start_ledger actual 观察"
        );
    }

    #[test]
    fn sg_07a_from_record_snapshot_five_states() {
        // 态①：session Running + 活动 hold + grant attempt1（AuthorRunning）。
        let fixture = sg_07a_sc_fixture("project_sg07a_five");
        let _reserved = sg_07a_reserve(
            &fixture,
            &fixture.birth,
            "turn_sg07a_1",
            "cmd_sg07a_1",
            "sg07a:start:turn1",
        );
        let running = sg_07a_mutate(&fixture, |record| {
            record.status = WorkspaceSessionStatus::Running;
            let state = record
                .structural_governance_state
                .get_or_insert_with(StructuralGovernanceState::default);
            state.current_grant = Some(sg_07a_grant(
                "turn_sg07a_1",
                "cmd_sg07a_1",
                StructuralGrantPhase::AuthorRunning,
                1,
            ));
            state.current_context_digest = Some("sha256:sg07a-context".to_string());
        });
        sg_07a_assert_chain(&fixture, &running);
        {
            let snapshot = sg_07a_built_snapshot(&fixture, &running).expect("state ① snapshot");
            assert_eq!(snapshot.user_budget_total, 3);
            assert_eq!(snapshot.available_user_budget, 2, "3 − 0 计费 − 1 hold");
            assert_eq!(snapshot.active_hold_count, 1);
            assert_eq!(snapshot.user_receipts.len(), 0);
            let grant = snapshot.current_grant.expect("state ① grant");
            assert_eq!(grant.attempt_no, 1);
            assert_eq!(grant.phase, Some(StructuralGrantPhase::AuthorRunning));
            assert_eq!(snapshot.accepted_feedback_turns, Some(1));
            assert_eq!(
                snapshot.actual_provider_starts, 0,
                "预留占位条目不是 actual start"
            );
        }

        // 态②：Reviewing + delivered_charge 结算、turn 仍 Running。
        let reviewing = sg_07a_mutate(&fixture, |record| {
            let state = record
                .structural_governance_state
                .get_or_insert_with(StructuralGovernanceState::default);
            state.current_grant = Some(sg_07a_grant(
                "turn_sg07a_2",
                "cmd_sg07a_2",
                StructuralGrantPhase::Reviewing,
                1,
            ));
        });
        let delivered = fixture
            .lifecycle
            .settle_structural_user_budget(
                &reviewing,
                "turn_sg07a_1",
                sg_07a_receipt(
                    UserBudgetReceiptKind::DeliveredCharge,
                    "turn_sg07a_1",
                    "cmd_sg07a_1",
                ),
            )
            .expect("settle delivered charge");
        sg_07a_assert_chain(&fixture, &delivered);
        {
            let snapshot = sg_07a_built_snapshot(&fixture, &delivered).expect("state ② snapshot");
            assert_eq!(snapshot.available_user_budget, 2, "3 − 1 delivered");
            assert_eq!(snapshot.active_hold_count, 0);
            assert_eq!(snapshot.user_receipts.len(), 1);
            assert_eq!(
                snapshot.user_receipts[0].kind,
                UserBudgetReceiptKind::DeliveredCharge
            );
            let grant = snapshot.current_grant.expect("state ② grant");
            assert_eq!(grant.phase, Some(StructuralGrantPhase::Reviewing));
        }

        // 态③：结构耗尽 released_structural——预算全额恢复（不实扣）。
        let fixture3 = sg_07a_sc_fixture("project_sg07a_release");
        let reserved3 = sg_07a_reserve(
            &fixture3,
            &fixture3.birth,
            "turn_sg07a_r",
            "cmd_sg07a_r",
            "sg07a:start:rel",
        );
        let released = fixture3
            .lifecycle
            .settle_structural_user_budget(
                &reserved3,
                "turn_sg07a_r",
                sg_07a_receipt(
                    UserBudgetReceiptKind::ReleasedStructural,
                    "turn_sg07a_r",
                    "cmd_sg07a_r",
                ),
            )
            .expect("settle released structural");
        sg_07a_assert_chain(&fixture3, &released);
        {
            let snapshot = sg_07a_built_snapshot(&fixture3, &released).expect("state ③ snapshot");
            assert_eq!(snapshot.available_user_budget, 3, "released 不实扣");
            assert_eq!(snapshot.active_hold_count, 0);
            assert_eq!(snapshot.user_receipts.len(), 1);
            assert_eq!(
                snapshot.user_receipts[0].kind,
                UserBudgetReceiptKind::ReleasedStructural
            );
        }

        // 态④：provider/取消非结构失败扣次——available 2、delivered 0。
        let fixture4 = sg_07a_sc_fixture("project_sg07a_failure");
        let reserved4 = sg_07a_reserve(
            &fixture4,
            &fixture4.birth,
            "turn_sg07a_f",
            "cmd_sg07a_f",
            "sg07a:start:fail",
        );
        let failed = fixture4
            .lifecycle
            .settle_structural_user_budget(
                &reserved4,
                "turn_sg07a_f",
                sg_07a_receipt(
                    UserBudgetReceiptKind::NonstructuralFailureCharge,
                    "turn_sg07a_f",
                    "cmd_sg07a_f",
                ),
            )
            .expect("settle nonstructural failure charge");
        sg_07a_assert_chain(&fixture4, &failed);
        {
            let snapshot = sg_07a_built_snapshot(&fixture4, &failed).expect("state ④ snapshot");
            assert_eq!(snapshot.available_user_budget, 2, "3 − 1 failure charge");
            assert_eq!(snapshot.active_hold_count, 0);
            assert!(
                snapshot.user_receipts.iter().all(
                    |receipt| receipt.kind == UserBudgetReceiptKind::NonstructuralFailureCharge
                )
            );
        }

        // 态⑤：grant 终态（Resolved）+ 无活动授权 + 同 scope checkpoint
        // 历史/已解决事实仍在 + actual start 证据在 ledger。
        let fixture5 = sg_07a_sc_fixture("project_sg07a_terminal");
        let _reserved5 = sg_07a_reserve(
            &fixture5,
            &fixture5.birth,
            "turn_sg07a_t",
            "cmd_sg07a_t",
            "sg07a:start:term",
        );
        let terminal = sg_07a_mutate(&fixture5, |record| {
            let state = record
                .structural_governance_state
                .get_or_insert_with(StructuralGovernanceState::default);
            state.current_grant = Some(sg_07a_grant(
                "turn_sg07a_t",
                "cmd_sg07a_t",
                StructuralGrantPhase::Resolved,
                2,
            ));
            state.structure_checkpoints = vec![StructuralCheckpoint {
                checkpoint_id: "sg07a-checkpoint-1".to_string(),
                source_revision_ref: Some("sg07a-source-ref".to_string()),
                plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
                structure_report_ref: Some("sg07a-report-ref".to_string()),
                artifact_version: Some(2),
                source_hash: Some("sha256:sg07a-failing-source".to_string()),
                context_digest: Some("sha256:sg07a-context".to_string()),
                authorization_hash: Some("sha256:sg07a-authorization".to_string()),
                receipt_ref: None,
                review_scope: Some("sg07a-review-scope".to_string()),
                status: StructuralCheckpointStatus::Superseded,
                recorded_at: sg_07a_now(),
            }];
            state.resolved_structure_facts = vec![ResolvedStructureFact {
                predicate: "dependency_cycle_resolved".to_string(),
                evidence_refs: vec!["sg07a-report-ref".to_string()],
                source_hash: Some("sha256:sg07a-failing-source".to_string()),
                resolved_at: sg_07a_now(),
            }];
            state.current_structure_report_ref = Some("sg07a-report-ref".to_string());
            state.current_context_digest = Some("sha256:sg07a-context".to_string());
            // actual start 观察（provider+started_at 齐备）——区别于态①的
            // 预留占位条目。
            record.provider_start_ledger.push(ProviderStartLedgerEntry {
                provider_start_idempotency_key: "sg07a:start:actual".to_string(),
                started: true,
                provider: Some("fake".to_string()),
                started_at: Some(sg_07a_now()),
                provider_attempt_key: None,
            });
        });
        sg_07a_assert_chain(&fixture5, &terminal);
        {
            let snapshot = sg_07a_built_snapshot(&fixture5, &terminal).expect("state ⑤ snapshot");
            let grant = snapshot.current_grant.expect("terminal grant kept");
            assert_eq!(grant.phase, Some(StructuralGrantPhase::Resolved));
            assert_eq!(grant.attempt_no, 2);
            assert_eq!(
                snapshot.structure_checkpoints.len(),
                1,
                "checkpoint 历史仍在"
            );
            assert_eq!(snapshot.resolved_structure_facts.len(), 1);
            assert_eq!(snapshot.actual_provider_starts, 1, "actual start 沿 ledger");
            assert_eq!(snapshot.available_user_budget, 2, "3 − 0 计费 − 1 hold");
        }

        // 负例（旧记录）：无聚合状态 → SessionState 快照省略（wire 兼容，
        // 无新许可）；同 ledger 的旧 started 布尔不冒充 actual。
        let fixture_old = sg_07a_sc_fixture("project_sg07a_old");
        let old = sg_07a_mutate(&fixture_old, |record| {
            record.structural_governance_state = None;
            record.provider_start_ledger.push(ProviderStartLedgerEntry {
                provider_start_idempotency_key: "sg07a:start:legacy".to_string(),
                started: true,
                provider: None,
                started_at: None,
                provider_attempt_key: None,
            });
        });
        let runtime_old = WorkspaceSession::from_record(old.clone());
        assert_eq!(runtime_old.structural_governance_state, None);
        let engine_old = sg_07a_projection_engine(&fixture_old.paths, &old);
        match engine_old.build_session_state() {
            WsOutMessage::SessionState {
                structural_governance: None,
                ..
            } => {}
            other => panic!("old record must omit structural governance, got {other:?}"),
        }
        assert!(sg_07a_built_snapshot(&fixture_old, &old).is_none());

        // 纯投影函数自身的单源契约（不受 runtime 接线影响的守卫断言）。
        let state = running.structural_governance_state.clone().unwrap();
        let projected = project_structural_governance_snapshot(
            &state,
            running.human_gate_snapshot.as_ref(),
            &running.provider_start_ledger,
            running.work_item_plan_source_revision_ref.as_deref(),
            running.plan_candidate_ir_ref.as_deref(),
            running.mechanical_report_ref.as_deref(),
        );
        assert_eq!(projected.available_user_budget, 2);
        assert_eq!(projected.accepted_feedback_turns, Some(1));
        assert_eq!(projected.actual_provider_starts, 0);
        assert_eq!(projected.source_ref.as_deref(), Some("sg07a-source-ref"));
    }
}
