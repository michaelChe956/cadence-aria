use super::*;

impl CodingWorkspaceEngine {
    pub fn new(
        store: CodingAttemptStore,
        git_service: GitWorkspaceService,
        event_tx: mpsc::Sender<CodingWsOutMessage>,
    ) -> Self {
        Self {
            store,
            _git_service: git_service,
            provider: None,
            event_tx,
        }
    }

    pub fn with_provider(
        store: CodingAttemptStore,
        git_service: GitWorkspaceService,
        provider: Arc<dyn ProviderAdapter + Send + Sync>,
        event_tx: mpsc::Sender<CodingWsOutMessage>,
    ) -> Self {
        Self {
            store,
            _git_service: git_service,
            provider: Some(provider),
            event_tx,
        }
    }

    pub(crate) fn provider_resume_session_id_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
        role: &CodingProviderRole,
        provider: &ProviderName,
    ) -> Option<String> {
        if !should_resume_provider_conversation(role) {
            return None;
        }

        let conversation_role = provider_conversation_role_for_coding_role(role);
        attempt
            .provider_conversations
            .iter()
            .find(|conversation| {
                conversation.role == conversation_role && &conversation.provider == provider
            })
            .map(|conversation| conversation.provider_session_id.clone())
            .filter(|id| !id.trim().is_empty())
    }

    pub(crate) fn record_attempt_provider_session(
        &self,
        attempt: &CodingExecutionAttempt,
        role: &CodingProviderRole,
        provider: ProviderName,
        provider_session_id: Option<String>,
        node_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let Some(provider_session_id) = provider_session_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
        else {
            return Ok(());
        };

        let conversation_role = provider_conversation_role_for_coding_role(role);
        let mut conversations = attempt.provider_conversations.clone();
        let now = chrono::Utc::now().to_rfc3339();
        if let Some(existing) = conversations.iter_mut().find(|conversation| {
            conversation.role == conversation_role && conversation.provider == provider
        }) {
            existing.provider_session_id = provider_session_id;
            existing.updated_at = now;
            existing.last_node_id = Some(node_id.to_string());
        } else {
            conversations.push(ProviderConversationRef {
                role: conversation_role,
                provider,
                provider_session_id,
                updated_at: now,
                last_node_id: Some(node_id.to_string()),
            });
        }

        self.store
            .replace_attempt_provider_conversations(&attempt.id, conversations)
            .map_err(CodingWorkspaceEngineError::from)?;
        Ok(())
    }

    pub async fn start_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Running,
        )?;
        let attempt = self.store.update_attempt_stage(
            project_id,
            issue_id,
            attempt_id,
            CodingExecutionStage::WorktreePrepare,
        )?;
        let node = self.create_running_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingStageChange {
                stage: CodingExecutionStage::WorktreePrepare,
            })
            .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node })
            .await;
        Ok(attempt)
    }

    pub async fn execute_worktree_prepare(
        &self,
        attempt: &CodingExecutionAttempt,
        repo_path: &Path,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let worktree_path = worktree_path_for_attempt(repo_path, attempt);
        self._git_service
            .create_branch(repo_path, &attempt.branch_name, &attempt.base_branch)
            .await?;
        self._git_service
            .create_worktree(repo_path, &attempt.branch_name, &worktree_path)
            .await?;
        let updated = self.store.update_attempt_worktree_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            worktree_path,
        )?;
        if let Some(node_id) = self.active_worktree_prepare_node_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )? {
            let completed_at = Utc::now().to_rfc3339();
            self.store.update_timeline_node_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &node_id,
                CodingTimelineNodeStatus::Completed,
                Some("worktree 已准备".to_string()),
                Some(completed_at.clone()),
            )?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                    node_id,
                    status: CodingTimelineNodeStatus::Completed,
                    summary: Some("worktree 已准备".to_string()),
                    completed_at: Some(completed_at),
                })
                .await;
        }
        Ok(updated)
    }
}
