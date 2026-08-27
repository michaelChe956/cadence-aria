use super::*;

/// spec-design-dialog-revision T6：HumanConfirm/ReviewDecision 已从 Story/Design 流程退役，
/// 存量会话恢复时迁移回 AuthorConfirm（保留产物与消息，review verdict 留在消息流）。
/// 仅迁移 stage，不触碰 timeline/持久化存储（恢复语义由消息流保留，懒迁移随下次持久化落盘）。
pub(crate) fn recover_story_design_retired_stage_fallback(
    mut session: WorkspaceSession,
) -> WorkspaceSession {
    let retired = matches!(
        session.stage,
        WorkspaceStage::HumanConfirm | WorkspaceStage::ReviewDecision
    );
    if retired
        && matches!(
            session.workspace_type,
            WorkspaceType::Story | WorkspaceType::Design
        )
    {
        session.stage = WorkspaceStage::AuthorConfirm;
    }
    session
}

fn recover_complete_artifact_misclassified_as_text_fallback(
    checkpoint_store: &CheckpointStore,
    lifecycle_store: &LifecycleStore,
    session: &mut WorkspaceSession,
    timeline_nodes: &mut Vec<TimelineNode>,
    active_node_id: &mut Option<String>,
    artifact_versions: &mut Vec<ArtifactVersion>,
) -> Result<bool, String> {
    if !matches!(
        session.workspace_type,
        WorkspaceType::Story | WorkspaceType::Design
    ) {
        return Ok(false);
    }
    let Some(source_node_id) = active_node_id.clone() else {
        return Ok(false);
    };
    let Some(source_node_index) = timeline_nodes.iter().position(|node| {
        node.node_id == source_node_id
            && matches!(
                node.node_type,
                TimelineNodeType::AuthorRun | TimelineNodeType::Revision
            )
            && node.status == TimelineNodeStatus::Paused
            && node
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("等待用户选择"))
    }) else {
        return Ok(false);
    };
    let Some(assistant_message_index) = session
        .messages
        .iter()
        .rposition(|message| message.role == "assistant")
    else {
        return Ok(false);
    };
    let full_content = &session.messages[assistant_message_index].content;
    if detect_author_choice_request(full_content, &session.workspace_type).is_some() {
        return Ok(false);
    }
    let artifact_markdown = extract_artifact_content(full_content);
    if !content_has_complete_workspace_artifact(&artifact_markdown, &session.workspace_type) {
        return Ok(false);
    }

    let payload = ArtifactPayload::Markdown {
        markdown: artifact_markdown.clone(),
        diff: None,
    };
    let mut recovered_artifact_versions = artifact_versions.clone();
    let current_version = recovered_artifact_versions
        .iter()
        .find(|version| version.is_current && version.markdown() == artifact_markdown);
    let version = if let Some(current) = current_version {
        current.version
    } else {
        for version in &mut recovered_artifact_versions {
            version.is_current = false;
        }
        let version = recovered_artifact_versions
            .iter()
            .map(|version| version.version)
            .max()
            .unwrap_or(0)
            + 1;
        recovered_artifact_versions.push(ArtifactVersion {
            version,
            payload: payload.clone(),
            generated_by: session.author_provider.clone(),
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            source_node_id: source_node_id.clone(),
        });
        version
    };

    lifecycle_store
        .ensure_version(AppendSpecVersionInput {
            project_id: session.project_id.clone(),
            issue_id: session.issue_id.clone(),
            entity_id: session.entity_id.clone(),
            markdown: artifact_markdown.clone(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .map_err(|error| format!("ensure recovered artifact spec version failed: {error}"))?;

    let now = chrono::Utc::now().to_rfc3339();
    let mut recovered_timeline_nodes = timeline_nodes.clone();
    let source_node = &mut recovered_timeline_nodes[source_node_index];
    source_node.status = TimelineNodeStatus::Completed;
    source_node.summary = Some("已恢复完整 artifact".to_string());
    source_node.completed_at = Some(now.clone());
    source_node.artifact_ref = Some("artifact_current".to_string());
    let author_confirm_node_id = format!("timeline_node_{:03}", recovered_timeline_nodes.len() + 1);
    recovered_timeline_nodes.push(TimelineNode {
        node_id: author_confirm_node_id.clone(),
        node_type: TimelineNodeType::AuthorConfirm,
        agent: None,
        stage: WsWorkspaceStage::AuthorConfirm,
        round: None,
        status: TimelineNodeStatus::Active,
        title: "Author 结果确认".to_string(),
        summary: Some("已恢复误判为文本选择的完整 artifact".to_string()),
        started_at: now.clone(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: Some("artifact_current".to_string()),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: session.author_provider.clone(),
            reviewer: session.reviewer_provider.clone(),
            review_rounds: session.review_rounds,
            permission_modes: session.permission_modes.clone(),
        },
        retry: None,
    });

    lifecycle_store
        .save_artifact_versions(&session.session_id, &recovered_artifact_versions)
        .map_err(|error| format!("save recovered artifact versions failed: {error}"))?;
    lifecycle_store
        .update_workspace_session_status(
            &session.session_id,
            WorkspaceSessionStatus::WaitingForHuman,
        )
        .map_err(|error| format!("save recovered workspace status failed: {error}"))?;

    let artifact_ref = ArtifactRef {
        artifact_id: format!("artifact_version_{version:03}"),
        version,
    };
    let message_index = u32::try_from(session.messages.len()).map_err(|_| {
        "workspace message count overflow during text fallback recovery".to_string()
    })?;
    let checkpoints = checkpoint_store
        .list_checkpoints(&session.session_id)
        .map_err(|error| format!("load checkpoints for text fallback recovery failed: {error}"))?;
    let checkpoint_id = if let Some(checkpoint) = checkpoints.iter().find(|checkpoint| {
        checkpoint.message_index == message_index
            && checkpoint.stage == WorkspaceStage::AuthorConfirm.as_str()
            && checkpoint
                .artifact_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.markdown_or_empty() == artifact_markdown)
    }) {
        checkpoint.id.clone()
    } else {
        checkpoint_store
            .create_checkpoint(
                &session.session_id,
                message_index,
                Some(&payload),
                WorkspaceStage::AuthorConfirm.as_str(),
            )
            .map_err(|error| format!("save recovered artifact checkpoint failed: {error}"))?
            .id
    };
    match lifecycle_store.load_node_detail(&session.session_id, &source_node_id) {
        Ok(mut detail) => {
            detail.status = TimelineNodeStatus::Completed;
            detail.ended_at = Some(now);
            detail.artifact_ref = Some(artifact_ref);
            lifecycle_store
                .save_node_detail(&session.session_id, &source_node_id, &detail)
                .map_err(|error| format!("save recovered node detail failed: {error}"))?;
        }
        Err(ProductStoreError::NotFound { .. }) => {}
        Err(error) => return Err(format!("load recovered node detail failed: {error}")),
    }
    lifecycle_store
        .save_timeline_nodes(&session.session_id, &recovered_timeline_nodes)
        .map_err(|error| format!("commit recovered timeline nodes failed: {error}"))?;

    session.messages[assistant_message_index].checkpoint_id = Some(checkpoint_id);
    session.artifact = Some(payload);
    session.stage = WorkspaceStage::AuthorConfirm;
    *artifact_versions = recovered_artifact_versions;
    *timeline_nodes = recovered_timeline_nodes;
    *active_node_id = Some(author_confirm_node_id);
    Ok(true)
}

impl WorkspaceEngine {
    pub fn new(
        checkpoint_store: Arc<CheckpointStore>,
        event_tx: mpsc::Sender<EngineEvent>,
        session: WorkspaceSession,
    ) -> Self {
        let (timeline_nodes, active_node_id) = initial_timeline(&session);
        Self {
            checkpoint_store,
            lifecycle_store: None,
            event_tx,
            session,
            cancel: CancellationToken::new(),
            timeline_nodes,
            active_node_id,
            artifact_versions: Vec::new(),
            latest_review_verdict: None,
            pending_revision_context: None,
            pending_author_choice: None,
            active_run_id: None,
            stream_buffers: HashMap::new(),
            work_item_plan_author_retry_count: 0,
            work_item_plan_revision_retry_count: 0,
            work_item_draft_repair_states: HashMap::new(),
            outline_revision_recovery_error: None,
            outline_revision_crash_after: None,
            plan_repair_crash_after: None,
            plan_repair_snapshot: None,
            #[cfg(test)]
            policy_route_before_persist: None,
            logical_provider_gateway: None,
        }
    }

    pub fn new_persistent(
        checkpoint_store: Arc<CheckpointStore>,
        lifecycle_store: LifecycleStore,
        event_tx: mpsc::Sender<EngineEvent>,
        mut session: WorkspaceSession,
    ) -> Self {
        let plan_repair_transition_recovery_error = recover_plan_repair_transition(
            &lifecycle_store,
            &session.project_id,
            &session.issue_id,
            &session.session_id,
        )
        .err()
        .map(|error| format!("plan repair journal recovery failed: {error:?}"));
        let outline_revision_recovery_error = recover_work_item_plan_outline_revision_transaction(
            &lifecycle_store,
            &session.project_id,
            &session.issue_id,
            &session.entity_id,
            &session.session_id,
        )
        .err();
        let persisted_timeline_nodes = lifecycle_store
            .load_timeline_nodes_for_issue_session(
                &session.project_id,
                &session.issue_id,
                &session.session_id,
            )
            .unwrap_or_default();
        let mut persisted_artifact_versions = lifecycle_store
            .list_artifact_versions_for_issue_session(
                &session.project_id,
                &session.issue_id,
                &session.session_id,
            )
            .unwrap_or_default();
        if !persisted_artifact_versions.is_empty() {
            session.artifact = persisted_artifact_versions
                .iter()
                .rev()
                .find(|version| version.is_current)
                .map(|version| version.payload.clone());
        }
        let (mut timeline_nodes, mut active_node_id) = if persisted_timeline_nodes.is_empty() {
            initial_timeline(&session)
        } else {
            let active_node_id = active_timeline_node_id(&persisted_timeline_nodes);
            if let Some(stage) = active_node_id
                .as_ref()
                .and_then(|node_id| {
                    persisted_timeline_nodes
                        .iter()
                        .find(|node| &node.node_id == node_id)
                })
                .map(|node| workspace_stage_from_ws_stage(&node.stage))
            {
                session.stage = stage;
            }
            (persisted_timeline_nodes, active_node_id)
        };
        if let Err(error) = recover_complete_artifact_misclassified_as_text_fallback(
            &checkpoint_store,
            &lifecycle_store,
            &mut session,
            &mut timeline_nodes,
            &mut active_node_id,
            &mut persisted_artifact_versions,
        ) {
            tracing::warn!(%error, "failed to recover complete artifact from text fallback state");
        }
        if let Err(error) = recover_work_item_plan_outline_review_schema_fallback(
            &lifecycle_store,
            &mut session,
            &mut timeline_nodes,
            &mut active_node_id,
        ) {
            tracing::warn!(%error, "failed to recover WorkItemPlan outline review schema fallback");
        }
        // 存量 Story/Design 会话若停留在已退役的 HumanConfirm/ReviewDecision 阶段，恢复时迁移回 AuthorConfirm。
        session = recover_story_design_retired_stage_fallback(session);
        let latest_review_verdict = latest_review_verdict_from_node_details(
            &lifecycle_store,
            &session.project_id,
            &session.issue_id,
            &session.session_id,
            &timeline_nodes,
        )
        .or_else(|| {
            latest_review_verdict_from_messages(&session.messages, &session.workspace_type)
        });
        let pending_author_choice =
            recover_pending_author_choice(&session, active_node_id.as_deref(), &timeline_nodes);
        let mut plan_repair_snapshot = load_plan_repair_snapshot_fail_closed(
            &lifecycle_store,
            &session,
            &mut timeline_nodes,
            plan_repair_transition_recovery_error,
        );
        if let Some(snapshot) = plan_repair_snapshot.as_mut() {
            snapshot.timeline_nodes = timeline_nodes.clone();
            session.stage = match snapshot.stage {
                PlanRepairSessionStage::AwaitingConfirmation
                | PlanRepairSessionStage::Published
                | PlanRepairSessionStage::AmendmentConflict
                | PlanRepairSessionStage::AmendmentApplyFailed => WorkspaceStage::HumanConfirm,
                PlanRepairSessionStage::Completed | PlanRepairSessionStage::Failed => {
                    active_node_id = None;
                    WorkspaceStage::Completed
                }
                PlanRepairSessionStage::Triaging
                | PlanRepairSessionStage::AuthoringRevision
                | PlanRepairSessionStage::ValidatingContract
                | PlanRepairSessionStage::GeneratingProjections
                | PlanRepairSessionStage::PlanReview
                | PlanRepairSessionStage::ApplyingAmendment => WorkspaceStage::Running,
            };
        }
        Self {
            checkpoint_store,
            lifecycle_store: Some(lifecycle_store),
            event_tx,
            session,
            cancel: CancellationToken::new(),
            timeline_nodes,
            active_node_id,
            artifact_versions: persisted_artifact_versions,
            latest_review_verdict,
            pending_revision_context: None,
            pending_author_choice,
            active_run_id: None,
            stream_buffers: HashMap::new(),
            work_item_plan_author_retry_count: 0,
            work_item_plan_revision_retry_count: 0,
            work_item_draft_repair_states: HashMap::new(),
            outline_revision_recovery_error,
            outline_revision_crash_after: None,
            plan_repair_crash_after: None,
            plan_repair_snapshot,
            #[cfg(test)]
            policy_route_before_persist: None,
            logical_provider_gateway: None,
        }
    }

    /// 为已停止的 auto WorkItemPlan 创建新的 interactive 后继运行。
    /// 原运行只作为 durable 审计来源，不会被此操作改写；WS/UI 操作入口属于阶段 3。
    pub fn takeover_stopped_needs_human(
        &self,
    ) -> Result<crate::product::models::WorkspaceSessionRecord, String> {
        let store = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        store
            .takeover_stopped_needs_human(&self.session.session_id)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn outline_revision_recovery_error(&self) -> Option<&str> {
        self.outline_revision_recovery_error.as_deref()
    }

    /// 注入逻辑代码库 provider gateway(Task 11)。Web 接入 task 为逻辑代码库 issue 构造
    /// gateway 后调用此方法,使 planning author stream 经 gateway 启动;未调用时引擎
    /// 保留直接 `provider.start` 路径。
    pub fn with_logical_provider_gateway(
        mut self,
        gateway: Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
    ) -> Self {
        self.logical_provider_gateway = Some(gateway);
        self
    }

    pub fn session(&self) -> &WorkspaceSession {
        &self.session
    }

    /// 克隆既有 `logical_provider_gateway` 字段。非空即逻辑会话(Task 11 注入)。
    pub fn logical_provider_gateway(
        &self,
    ) -> Option<Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>> {
        self.logical_provider_gateway.clone()
    }

    /// 逻辑会话的 planning launch 标识:(project_id, run cwd)。
    ///
    /// 仅当注入 `logical_provider_gateway` 且 `session.repository_path` 存在时返回
    /// `Some`。`cwd` 取既有字段 `session.repository_path`(不新造数据源)——对逻辑
    /// WorkItemPlan 该字段是选中成员 checkout(见 gateway_start.rs 的 deferred 说明)。
    pub fn logical_planning_launch(&self) -> Option<(String, std::path::PathBuf)> {
        self.logical_provider_gateway.as_ref()?;
        let cwd = self.session.repository_path.clone()?;
        Some((self.session.project_id.clone(), cwd))
    }

    /// Story/Design author/revision/review prompt 注入用的路由引用上下文。
    ///
    /// 逻辑会话(已注入 gateway 且有 planning cwd)时经 gateway `validate` 冻结
    /// `PlanningReadOnly` envelope 后派生 `Logical`;无 gateway/无 cwd/validate 失败
    /// 一律回落 `Legacy`(与改造前 `_legacy()` 字节一致)。
    ///
    /// 只用于 prompt 路由引用分流,不改变 provider 启动路径(Task 4 范围)。
    pub(crate) fn routing_reference_context(
        &self,
    ) -> crate::product::cadence_skills::routing_reference::RoutingReferenceContext {
        use crate::product::cadence_skills::routing_reference::{
            RoutingReferenceContext, routing_reference_context_from_policy,
        };
        use crate::product::logical_codebase::{PolicyTarget, ProviderRef, SessionLaunchRequest};

        let Some(gateway) = self.logical_provider_gateway() else {
            return RoutingReferenceContext::Legacy;
        };
        let Some((project_id, working_dir)) = self.logical_planning_launch() else {
            return RoutingReferenceContext::Legacy;
        };
        // 镜像 factory 的 canonicalize 语义:aggregate_root target worktree 用
        // canonicalize 后形态,失败回退原值(resolver 会再 canonicalize 复核)。
        let target_worktree =
            std::fs::canonicalize(&working_dir).unwrap_or_else(|_| working_dir.clone());
        let request = SessionLaunchRequest::planning(
            project_id,
            ProviderRef::claude_code("cap_managed_snapshot"),
            PolicyTarget::aggregate_root(target_worktree),
            vec![working_dir],
            "sha256:managed-config-artifact",
        );
        match gateway.validate(request) {
            Ok(policy) => routing_reference_context_from_policy(&policy),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "workspace_engine routing_reference_context: gateway validate failed, falling back to Legacy"
                );
                RoutingReferenceContext::Legacy
            }
        }
    }

    pub fn pending_author_choice_request_message(&self) -> Option<WsOutMessage> {
        let pending = self.pending_author_choice.as_ref()?;
        Some(WsOutMessage::ChoiceRequest {
            id: pending.id.clone(),
            prompt: pending.prompt.clone(),
            options: pending
                .options
                .iter()
                .map(|option| ChoiceOption {
                    id: option.id.clone(),
                    label: option.label.clone(),
                    description: option.description.clone(),
                })
                .collect(),
            allow_multiple: false,
            allow_free_text: true,
            questions: vec![ChoiceQuestion {
                id: "default".to_string(),
                prompt: pending.prompt.clone(),
                options: pending
                    .options
                    .iter()
                    .map(|option| ChoiceOption {
                        id: option.id.clone(),
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect(),
                allow_multiple: false,
                allow_free_text: true,
            }],
            source: ChoiceRequestSource::TextFallback.as_str().to_string(),
        })
    }

    pub(crate) fn provider_resume_session_id(
        &self,
        role: ProviderConversationRole,
        provider: &ProviderName,
    ) -> Option<String> {
        provider_conversation_session_id(&self.session.provider_conversations, &role, provider)
            .or_else(|| {
                self.lifecycle_store.as_ref().and_then(|store| {
                    store
                        .get_workspace_session(&self.session.session_id)
                        .ok()
                        .and_then(|session| {
                            provider_conversation_session_id(
                                &session.provider_conversations,
                                &role,
                                provider,
                            )
                        })
                })
            })
    }

    pub(crate) async fn record_provider_session(
        &mut self,
        role: ProviderConversationRole,
        provider: ProviderName,
        provider_session_id: Option<String>,
        node_id: Option<String>,
    ) {
        let Some(provider_session_id) = provider_session_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
        else {
            return;
        };
        let now = chrono::Utc::now().to_rfc3339();
        if let Some(existing) = self
            .session
            .provider_conversations
            .iter_mut()
            .find(|conversation| conversation.role == role && conversation.provider == provider)
        {
            existing.provider_session_id = provider_session_id;
            existing.updated_at = now;
            existing.last_node_id = node_id;
        } else {
            self.session
                .provider_conversations
                .push(ProviderConversationRef {
                    role,
                    provider,
                    provider_session_id,
                    updated_at: now,
                    last_node_id: node_id,
                });
        }
        if let Some(store) = &self.lifecycle_store {
            let _ = store.replace_workspace_provider_conversations(
                &self.session.session_id,
                self.session.provider_conversations.clone(),
            );
        }
    }

    pub fn current_stage(&self) -> WorkspaceStage {
        self.session.stage.clone()
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn start_new_run_token(&mut self) -> CancellationToken {
        self.cancel = CancellationToken::new();
        self.cancel.clone()
    }

    pub fn use_run_token(&mut self, cancel: CancellationToken) {
        self.cancel = cancel;
    }

    pub fn mark_active_run_started(&mut self, run_id: impl Into<String>) {
        self.active_run_id = Some(run_id.into());
    }

    pub fn mark_active_run_finished(&mut self, run_id: &str) {
        if self.active_run_id.as_deref() == Some(run_id) {
            self.active_run_id = None;
            if matches!(
                self.active_node_type(),
                Some(TimelineNodeType::WorkItemDraftRun | TimelineNodeType::WorkItemBatchRun)
            ) {
                self.work_item_draft_repair_states.clear();
            }
        }
    }

    pub fn active_run_id(&self) -> Option<&str> {
        self.active_run_id.as_deref()
    }

    pub fn active_timeline_node_id(&self) -> Option<String> {
        self.active_node_id.clone()
    }

    pub(crate) fn active_node_type(&self) -> Option<TimelineNodeType> {
        let active_node_id = self.active_node_id.as_deref()?;
        self.timeline_nodes
            .iter()
            .find(|node| node.node_id == active_node_id)
            .map(|node| node.node_type.clone())
    }

    pub async fn take_pending_author_choice_prompt(
        &mut self,
        id: &str,
        selected_option_ids: Vec<String>,
        free_text: Option<String>,
    ) -> Result<String, PendingAuthorChoiceError> {
        let Some(pending) = self.pending_author_choice.as_ref() else {
            return Err(PendingAuthorChoiceError::NotFound { id: id.to_string() });
        };
        if pending.id != id {
            return Err(PendingAuthorChoiceError::IdMismatch {
                expected: pending.id.clone(),
                actual: id.to_string(),
            });
        }

        let mut selected_labels = Vec::new();
        for selected_id in &selected_option_ids {
            let Some(option) = pending
                .options
                .iter()
                .find(|option| option.id == *selected_id)
            else {
                return Err(PendingAuthorChoiceError::OptionUnmatched {
                    id: selected_id.clone(),
                });
            };
            selected_labels.push(option.label.clone());
        }

        let free_text = free_text.and_then(|text| {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let pending = self
            .pending_author_choice
            .take()
            .expect("pending author choice present");
        if let Some(node_id) = pending.source_node_id.as_deref() {
            self.update_timeline_node(
                node_id,
                TimelineNodeStatus::Completed,
                Some("已收到用户选择".to_string()),
            )
            .await;
        }

        let mut prompt = String::new();
        prompt.push_str("用户回答了 author 的确认问题：\n");
        prompt.push_str(&format!("问题：{}\n", pending.prompt));
        if !selected_labels.is_empty() {
            prompt.push_str("选择：\n");
            for label in selected_labels {
                prompt.push_str(&format!("- {label}\n"));
            }
        }
        if let Some(free_text) = free_text {
            prompt.push_str(&format!("补充：{free_text}\n"));
        }
        prompt.push_str(
            "\n请基于该回答继续生成完整候选产物；如果仍有必须由用户确认的问题，请继续发起选择请求，不要进入 reviewer。",
        );
        if self.session.workspace_type == WorkspaceType::Design {
            self.append_design_author_artifact_contract(&mut prompt, false);
        }
        Ok(prompt)
    }

    pub async fn append_context_note(&mut self, content: String) -> Result<TimelineNode, String> {
        let msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();
        self.session.messages.push(SessionMessage {
            id: msg_id,
            role: "user".to_string(),
            content: content.clone(),
            checkpoint_id: None,
            created_at: now,
        });
        if let Some(store) = &self.lifecycle_store {
            store
                .append_workspace_message(
                    &self.session.session_id,
                    "user".to_string(),
                    content.clone(),
                )
                .map_err(|error| format!("persist context note failed: {error}"))?;
        }
        Ok(self
            .append_completed_timeline_event(
                TimelineNodeType::ContextNote,
                WorkspaceStage::PrepareContext,
                "上下文补充".to_string(),
                Some(content),
                TimelineNodeStatus::Completed,
                false,
            )
            .await)
    }

    pub async fn start_generation(
        &mut self,
        provider_config: ProviderConfigSnapshot,
        reviewer_enabled: bool,
    ) -> Result<(TimelineNode, WsOutMessage), String> {
        let mut locked_snapshot = provider_config;
        locked_snapshot.permission_modes.author = permission_mode_for_provider(
            &locked_snapshot.author,
            locked_snapshot.permission_modes.author.clone(),
        );
        if let Some(reviewer) = &locked_snapshot.reviewer {
            locked_snapshot.permission_modes.reviewer = permission_mode_for_provider(
                reviewer,
                locked_snapshot.permission_modes.reviewer.clone(),
            );
        }
        // Capture the raw snapshot reviewer BEFORE the disabled-review clear below:
        // provisional must retain the original selection even when reviewer_enabled=false
        // (design.md §3, provisional 恢复闭环); reviewer_provider/review_rounds are still cleared.
        let provisional_reviewer = locked_snapshot.reviewer.clone();
        if !reviewer_enabled {
            locked_snapshot.reviewer = None;
            locked_snapshot.review_rounds = 0;
        }

        self.session.author_provider = locked_snapshot.author.clone();
        self.session.reviewer_provider = locked_snapshot.reviewer.clone();
        self.session.review_rounds = locked_snapshot.review_rounds;
        self.session.permission_modes = locked_snapshot.permission_modes.clone();

        self.session.provisional_reviewer_provider = provisional_reviewer;
        self.session.reviewer_enabled_at_start = Some(reviewer_enabled);
        if let Some(store) = &self.lifecycle_store {
            store
                .update_workspace_session_provisional_reviewer(
                    &self.session.session_id,
                    self.session.provisional_reviewer_provider.clone(),
                    self.session.reviewer_enabled_at_start,
                )
                .map_err(|error| format!("persist provisional reviewer failed: {error}"))?;
        }

        if let Some(store) = &self.lifecycle_store {
            let reviewer_provider = locked_snapshot
                .reviewer
                .clone()
                .unwrap_or_else(|| locked_snapshot.author.clone());
            store
                .update_workspace_session_providers(
                    &self.session.session_id,
                    locked_snapshot.author.clone(),
                    reviewer_provider,
                )
                .map_err(|error| format!("persist provider lock failed: {error}"))?;
            store
                .update_workspace_session_permission_modes(
                    &self.session.session_id,
                    locked_snapshot.permission_modes.clone(),
                )
                .map_err(|error| format!("persist permission mode lock failed: {error}"))?;
            store
                .update_workspace_session_status(
                    &self.session.session_id,
                    WorkspaceSessionStatus::Running,
                )
                .map_err(|error| format!("persist workspace status failed: {error}"))?;
        }

        self.complete_active_node(Some("上下文已确认".to_string()))
            .await;
        let node = self
            .append_completed_timeline_event(
                TimelineNodeType::StartGeneration,
                WorkspaceStage::PrepareContext,
                "开始生成".to_string(),
                None,
                TimelineNodeStatus::Completed,
                true,
            )
            .await;
        self.transition_stage(WorkspaceStage::Running).await;

        let locked = WsOutMessage::ProviderLocked {
            snapshot: locked_snapshot,
            locked_at: chrono::Utc::now().to_rfc3339(),
        };
        Ok((node, locked))
    }

    pub async fn append_aborted_by_disconnect(
        &mut self,
        last_active_run_id: String,
    ) -> Result<TimelineNode, String> {
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(
                &node_id,
                TimelineNodeStatus::Failed,
                Some("连接断开，运行已中止".to_string()),
            )
            .await;
        }
        self.active_run_id = None;
        Ok(self
            .append_completed_timeline_event(
                TimelineNodeType::AbortedByDisconnect,
                WorkspaceStage::PrepareContext,
                "运行因断开中止".to_string(),
                Some(format!("last_active_run_id: {last_active_run_id}")),
                TimelineNodeStatus::Failed,
                true,
            )
            .await)
    }

    pub async fn transition_to_prepare_context_after_disconnect(&mut self) {
        self.active_run_id = None;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Open,
            );
        }
        self.transition_stage(WorkspaceStage::PrepareContext).await;
    }

    pub async fn recover_stale_active_run_after_disconnect(&mut self) {
        if !matches!(
            self.session.stage,
            WorkspaceStage::Running | WorkspaceStage::CrossReview | WorkspaceStage::Revision
        ) {
            return;
        }

        let already_recorded = self
            .timeline_nodes
            .last()
            .is_some_and(|node| node.node_type == TimelineNodeType::AbortedByDisconnect);
        if !already_recorded {
            let run_id = self
                .active_run_id
                .clone()
                .unwrap_or_else(|| "stale-connection".to_string());
            let _ = self.append_aborted_by_disconnect(run_id).await;
        }
        self.transition_to_prepare_context_after_disconnect().await;
    }
}
