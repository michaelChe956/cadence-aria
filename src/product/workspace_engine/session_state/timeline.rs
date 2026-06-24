use super::*;

impl WorkspaceEngine {
    pub(crate) async fn transition_stage(&mut self, new_stage: WorkspaceStage) {
        self.session.stage = new_stage;
        let _ = self
            .event_tx
            .send(EngineEvent::StageChange {
                stage: self.session.stage.as_str().to_string(),
            })
            .await;
    }

    pub(crate) async fn finish_failed_run(&mut self) {
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Open,
            );
        }
        self.active_run_id = None;
        self.transition_stage(WorkspaceStage::PrepareContext).await;
    }

    pub async fn finish_active_run_with_failed_node(&mut self, message: impl Into<String>) {
        let message = message.into();
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(&node_id, TimelineNodeStatus::Failed, Some(message))
                .await;
        }
        self.finish_failed_run().await;
    }

    pub(crate) async fn finish_empty_assistant_output(&mut self) {
        let _ = self
            .event_tx
            .send(EngineEvent::Error {
                message: "Provider completed without assistant output".to_string(),
            })
            .await;
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(
                &node_id,
                TimelineNodeStatus::Failed,
                Some("Provider 未返回助手内容".to_string()),
            )
            .await;
        }
        self.finish_failed_run().await;
    }

    pub(crate) async fn finish_invalid_workspace_artifact(&mut self) {
        let artifact_name = workspace_type_title(&self.session.workspace_type);
        let message = format!("Provider 未返回有效的 {artifact_name} artifact");
        let _ = self
            .event_tx
            .send(EngineEvent::Error {
                message: message.clone(),
            })
            .await;
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(&node_id, TimelineNodeStatus::Failed, Some(message))
                .await;
        }
        self.finish_failed_run().await;
    }

    pub(crate) async fn finish_invalid_workspace_artifact_after_retry(&mut self) {
        let artifact_name = workspace_type_title(&self.session.workspace_type);
        let message = format!("自动续写后仍未返回有效的 {artifact_name} artifact");
        let _ = self
            .event_tx
            .send(EngineEvent::Error {
                message: message.clone(),
            })
            .await;
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(&node_id, TimelineNodeStatus::Failed, Some(message))
                .await;
        }
        self.finish_failed_run().await;
    }

    pub(crate) fn workspace_requires_artifact_gate(&self) -> bool {
        matches!(
            self.session.workspace_type,
            WorkspaceType::Story | WorkspaceType::Design
        )
    }

    pub(crate) async fn finish_aborted_run(&mut self) {
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(
                &node_id,
                TimelineNodeStatus::Failed,
                Some("运行已中止".to_string()),
            )
            .await;
        }
        self.finish_failed_run().await;
    }

    pub(crate) async fn handle_permission_timeout(
        &mut self,
        permission_id: String,
        node_id: Option<String>,
    ) {
        tracing::warn!(permission_id = %permission_id, "permission timed out; aborting active run");
        if let Some(node_id) = node_id.as_deref() {
            let _ = self
                .persist_permission_timeout(node_id, permission_id.clone())
                .await;
            let _ = self.flush_stream_buffer(node_id).await;
            self.update_timeline_node(
                node_id,
                TimelineNodeStatus::Failed,
                Some("权限请求超时，运行已中止".to_string()),
            )
            .await;
        }
        self.active_run_id = None;
        self.cancel.cancel();
        let _ = self
            .event_tx
            .send(EngineEvent::PermissionTimeout {
                permission_id,
                node_id,
            })
            .await;
        self.finish_failed_run().await;
    }

    pub(crate) async fn create_timeline_node(&mut self, draft: TimelineNodeDraft) -> String {
        let node_id = format!("timeline_node_{:03}", self.timeline_nodes.len() + 1);
        let node = TimelineNode {
            node_id: node_id.clone(),
            node_type: draft.node_type,
            agent: draft.agent,
            stage: ws_stage(&draft.stage),
            round: draft.round,
            status: draft.status,
            title: draft.title,
            summary: draft.summary,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: self
                .session
                .artifact
                .as_ref()
                .map(|_| "artifact_current".to_string()),
            provider_config_snapshot: self.provider_config_snapshot(),
            retry: None,
        };
        self.timeline_nodes.push(node.clone());
        self.active_node_id = Some(node_id.clone());
        self.persist_timeline_nodes();
        let _ = self
            .event_tx
            .send(EngineEvent::TimelineNodeCreated { node })
            .await;
        node_id
    }

    pub(crate) async fn append_completed_timeline_event(
        &mut self,
        node_type: TimelineNodeType,
        stage: WorkspaceStage,
        title: String,
        summary: Option<String>,
        status: TimelineNodeStatus,
        make_active: bool,
    ) -> TimelineNode {
        let now = chrono::Utc::now().to_rfc3339();
        let node_id = format!("timeline_node_{:03}", self.timeline_nodes.len() + 1);
        let node = TimelineNode {
            node_id: node_id.clone(),
            node_type,
            agent: None,
            stage: ws_stage(&stage),
            round: None,
            status,
            title,
            summary,
            started_at: now.clone(),
            completed_at: Some(now),
            duration_ms: Some(0),
            artifact_ref: self
                .session
                .artifact
                .as_ref()
                .map(|_| "artifact_current".to_string()),
            provider_config_snapshot: self.provider_config_snapshot(),
            retry: None,
        };
        self.timeline_nodes.push(node.clone());
        if make_active {
            self.active_node_id = Some(node_id);
        }
        self.persist_timeline_nodes();
        let _ = self
            .event_tx
            .send(EngineEvent::TimelineNodeCreated { node: node.clone() })
            .await;
        node
    }

    pub(crate) async fn create_timeline_node_with_retry(
        &mut self,
        draft: TimelineNodeDraft,
        retry: Option<TimelineNodeRetry>,
    ) -> String {
        let node_id = format!("timeline_node_{:03}", self.timeline_nodes.len() + 1);
        let node = TimelineNode {
            node_id: node_id.clone(),
            node_type: draft.node_type,
            agent: draft.agent,
            stage: ws_stage(&draft.stage),
            round: draft.round,
            status: draft.status,
            title: draft.title,
            summary: draft.summary,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: self
                .session
                .artifact
                .as_ref()
                .map(|_| "artifact_current".to_string()),
            provider_config_snapshot: self.provider_config_snapshot(),
            retry,
        };
        self.timeline_nodes.push(node.clone());
        self.active_node_id = Some(node_id.clone());
        self.persist_timeline_nodes();
        let _ = self
            .event_tx
            .send(EngineEvent::TimelineNodeCreated { node })
            .await;
        node_id
    }

    pub(crate) fn empty_node_detail_for(&self, node: &TimelineNode) -> NodeDetail {
        NodeDetail {
            node_id: node.node_id.clone(),
            session_id: self.session.session_id.clone(),
            node_type: node.node_type.clone(),
            status: node.status.clone(),
            agent_role: match node.node_type {
                TimelineNodeType::AuthorRun
                | TimelineNodeType::WorkItemPlanOutlineRun
                | TimelineNodeType::WorkItemDraftRun
                | TimelineNodeType::WorkItemBatchRun => Some(AgentRole::Author),
                TimelineNodeType::ReviewerRun
                | TimelineNodeType::WorkItemPlanOutlineReview
                | TimelineNodeType::WorkItemDraftReview
                | TimelineNodeType::WorkItemBatchReview => Some(AgentRole::Reviewer),
                _ => None,
            },
            provider: node.agent.as_ref().map(|provider| ProviderSnapshot {
                name: provider_name_text(provider).to_string(),
                model: provider_name_text(provider).to_string(),
            }),
            prompt: None,
            messages: Vec::new(),
            streaming_content: String::new(),
            execution_events: Vec::new(),
            permission_events: Vec::new(),
            verdict: None,
            artifact_ref: None,
            is_revision: node.node_type == TimelineNodeType::AuthorRun
                && node.stage == WsWorkspaceStage::Revision,
            base_artifact_ref: None,
            started_at: node.started_at.clone(),
            ended_at: node.completed_at.clone(),
        }
    }

    pub(crate) async fn update_node_detail<F>(
        &mut self,
        node_id: &str,
        update: F,
    ) -> Result<(), String>
    where
        F: FnOnce(&mut NodeDetail),
    {
        let Some(store) = &self.lifecycle_store else {
            return Ok(());
        };

        let Some(node) = self
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .cloned()
            .or_else(|| {
                store
                    .load_timeline_nodes(&self.session.session_id)
                    .ok()?
                    .into_iter()
                    .find(|node| node.node_id == node_id)
            })
        else {
            return Err(format!("timeline node not found: {node_id}"));
        };

        let mut detail = match store.load_node_detail(&self.session.session_id, node_id) {
            Ok(detail) => detail,
            Err(ProductStoreError::NotFound { .. }) => self.empty_node_detail_for(&node),
            Err(error) => return Err(format!("load node detail failed: {error}")),
        };
        update(&mut detail);
        store
            .save_node_detail(&self.session.session_id, node_id, &detail)
            .map_err(|error| format!("save node detail failed: {error}"))?;
        Ok(())
    }

    pub(crate) async fn persist_prompt_snapshot(
        &mut self,
        node_id: &str,
        prompt: String,
    ) -> Result<(), String> {
        self.update_node_detail(node_id, |detail| {
            detail.prompt = Some(prompt);
        })
        .await
    }

    pub(crate) async fn complete_active_node(&mut self, summary: Option<String>) {
        let Some(node_id) = self.active_node_id.clone() else {
            return;
        };
        self.update_timeline_node(&node_id, TimelineNodeStatus::Completed, summary)
            .await;
    }

    pub(crate) async fn update_timeline_node(
        &mut self,
        node_id: &str,
        status: TimelineNodeStatus,
        summary: Option<String>,
    ) {
        let completed_at = if matches!(
            status,
            TimelineNodeStatus::Completed
                | TimelineNodeStatus::Failed
                | TimelineNodeStatus::Skipped
        ) {
            Some(chrono::Utc::now().to_rfc3339())
        } else {
            None
        };

        if let Some(node) = self
            .timeline_nodes
            .iter_mut()
            .find(|node| node.node_id == node_id)
        {
            node.status = status.clone();
            if summary.is_some() {
                node.summary = summary.clone();
            }
            if completed_at.is_some() {
                node.completed_at = completed_at.clone();
            }
        }
        self.persist_timeline_nodes();
        let detail_status = status.clone();
        let detail_completed_at = completed_at.clone();
        let _ = self
            .update_node_detail(node_id, |detail| {
                detail.status = detail_status;
                if detail_completed_at.is_some() {
                    detail.ended_at = detail_completed_at;
                }
            })
            .await;

        let _ = self
            .event_tx
            .send(EngineEvent::TimelineNodeUpdated {
                node_id: node_id.to_string(),
                status,
                summary,
                completed_at,
            })
            .await;
    }

    pub(crate) fn provider_config_snapshot(&self) -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: self.session.author_provider.clone(),
            reviewer: self.session.reviewer_provider.clone(),
            review_rounds: self.session.review_rounds,
        }
    }

    pub(crate) fn next_review_round(&self) -> u32 {
        self.timeline_nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.node_type,
                    TimelineNodeType::ReviewerRun | TimelineNodeType::WorkItemPlanOutlineReview
                )
            })
            .count() as u32
            + 1
    }

    pub(crate) fn active_review_round(&self) -> Option<u32> {
        let active_node_id = self.active_node_id.as_ref()?;
        self.timeline_nodes
            .iter()
            .find(|node| &node.node_id == active_node_id)
            .and_then(|node| node.round)
    }

    pub(crate) fn active_node_agent(&self) -> Option<ProviderName> {
        let active_node_id = self.active_node_id.as_ref()?;
        self.timeline_nodes
            .iter()
            .find(|node| &node.node_id == active_node_id)
            .and_then(|node| node.agent.clone())
    }

    pub(crate) fn record_review_message(&mut self, content: String) {
        let msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();
        self.session.messages.push(SessionMessage {
            id: msg_id,
            role: "reviewer".to_string(),
            content: content.clone(),
            checkpoint_id: None,
            created_at: now,
        });
        if let Some(store) = &self.lifecycle_store {
            let _ = store.append_workspace_message(
                &self.session.session_id,
                "reviewer".to_string(),
                content,
            );
        }
    }

    pub(crate) fn mark_latest_artifact_reviewed(
        &mut self,
        reviewed_by: Option<ProviderName>,
        review_verdict: Option<ReviewVerdictType>,
    ) {
        if let Some(version) = self.artifact_versions.last_mut() {
            version.reviewed_by = reviewed_by;
            version.review_verdict = review_verdict;
            self.persist_artifact_versions();
        }
    }

    pub(crate) fn mark_latest_artifact_confirmed(&mut self, confirmed_by: Option<String>) {
        if let Some(version) = self.artifact_versions.last_mut() {
            version.confirmed_by = confirmed_by;
            self.persist_artifact_versions();
        }
    }

    pub(crate) fn mark_latest_artifact_rejected(&mut self) {
        if let Some(version) = self.artifact_versions.last_mut() {
            version.is_current = false;
            self.persist_artifact_versions();
        }
    }

    pub(crate) fn persist_timeline_nodes(&self) {
        if let Some(store) = &self.lifecycle_store {
            let _ = store.save_timeline_nodes(&self.session.session_id, &self.timeline_nodes);
        }
    }

    pub(crate) fn persist_artifact_versions(&self) {
        if let Some(store) = &self.lifecycle_store {
            let _ = store.save_artifact_versions(&self.session.session_id, &self.artifact_versions);
        }
    }
}
