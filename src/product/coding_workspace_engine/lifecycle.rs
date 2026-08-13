use super::*;
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::product::coding_models::CodingAttemptScope;
use crate::product::logical_codebase::policy::{PolicyTarget, SessionPolicyAction};
use crate::product::logical_codebase::provider_gateway::{
    ProviderGatewayError, SessionLaunchRequest,
};
use crate::product::work_item_split_engine::engine::provider_ref_for_name;
use std::sync::Arc;

impl CodingWorkspaceEngine {
    pub fn new(
        store: CodingAttemptStore,
        git_service: GitWorkspaceService,
        event_tx: mpsc::Sender<CodingWsOutMessage>,
    ) -> Self {
        let cancellation = CancellationToken::new();
        Self {
            store,
            _git_service: git_service,
            event_tx: CancellableCodingEventSender::new(event_tx, cancellation.clone()),
            cancellation,
            logical_provider_gateway: None,
        }
    }

    /// 注入逻辑代码库 provider gateway。Task 11:Web 接入 task 为逻辑代码库 issue
    /// 构造 gateway 后调用此方法,使 provider stream 经 gateway 启动;未调用时
    /// 引擎保留直接 `provider.start` 路径。
    pub fn with_logical_provider_gateway(
        mut self,
        gateway: Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
    ) -> Self {
        self.logical_provider_gateway = Some(gateway);
        self
    }

    /// 逻辑 target + 已注入 gateway 时构造 validated input；否则 None（Legacy 直连）。
    ///
    /// `provider` 由 `input.provider_type` 反推（与 `provider_type_for_name` 1:1），
    /// 再经 `provider_ref_for_name` 映射为 gateway 的 `ProviderRef`。Coder 角色映射
    /// `CodingTargetWrite`（writable_roots 为 worktree），其余角色映射
    /// `ReviewReadOnly`（writable_roots 为空）。
    pub(crate) fn validated_streaming_input_for_role(
        &self,
        attempt: &CodingExecutionAttempt,
        role: CodingProviderRole,
        input: StreamingProviderInput,
    ) -> Result<Option<ValidatedStreamingProviderInput>, ProviderGatewayError> {
        let Some(gateway) = self.logical_provider_gateway.as_ref() else {
            return Ok(None);
        };
        let Some(snapshot) = attempt.target_snapshot.as_ref() else {
            return Ok(None);
        };

        let action = match role {
            CodingProviderRole::Coder => SessionPolicyAction::CodingTargetWrite,
            CodingProviderRole::CodeReviewer | CodingProviderRole::InternalReviewer => {
                SessionPolicyAction::ReviewReadOnly
            }
        };
        let writable_roots = match role {
            CodingProviderRole::Coder => vec![input.working_dir.clone()],
            CodingProviderRole::CodeReviewer | CodingProviderRole::InternalReviewer => Vec::new(),
        };
        let provider_name = match input.provider_type {
            ProviderType::ClaudeCode => ProviderName::ClaudeCode,
            ProviderType::Codex => ProviderName::Codex,
            ProviderType::Pi => ProviderName::Pi,
            ProviderType::Fake => ProviderName::Fake,
        };
        let request = SessionLaunchRequest {
            project_id: attempt.project_id.clone(),
            provider: provider_ref_for_name(&provider_name),
            action,
            // CodingTargetWrite 的 single writable root 与 target worktree 都是当前 coding
            // 嵌套 worktree（C-1 行为：checkout/.worktrees/aria-issues/{issue}），主 checkout
            // 只提供身份（logical_repository_id/checkout_id 仍取 snapshot 身份）。
            target: PolicyTarget::checkout(
                snapshot.logical_repository_id.0.to_string(),
                snapshot.checkout_id.0.to_string(),
                input.working_dir.clone(),
            ),
            readable_roots: vec![input.working_dir.clone()],
            writable_roots,
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        };
        let validated = gateway.validate(request)?;
        Ok(Some(ValidatedStreamingProviderInput::new(input, validated)))
    }

    pub(crate) fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.event_tx = CancellableCodingEventSender::new(
            self.event_tx.raw_sender().clone(),
            cancellation.clone(),
        );
        self._git_service = self._git_service.with_cancellation(cancellation.clone());
        self.cancellation = cancellation;
        self
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
            .replace_attempt_provider_conversations(attempt, conversations)
            .map_err(CodingWorkspaceEngineError::from)?;
        Ok(())
    }

    pub(crate) fn clear_attempt_provider_conversation(
        &self,
        attempt: &CodingExecutionAttempt,
        role: &CodingProviderRole,
        provider: &ProviderName,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let conversation_role = provider_conversation_role_for_coding_role(role);
        let conversations = attempt
            .provider_conversations
            .iter()
            .filter(|conversation| {
                conversation.role != conversation_role || &conversation.provider != provider
            })
            .cloned()
            .collect();
        self.store
            .replace_attempt_provider_conversations(attempt, conversations)
            .map_err(CodingWorkspaceEngineError::from)
    }

    pub async fn start_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        if current.scope == CodingAttemptScope::WorkItemGroup {
            self.store.validate_group_attempt_integrity(&current)?;
        }
        let running = if current.status == CodingAttemptStatus::Running {
            current
        } else {
            self.store
                .admit_and_transition_attempt_to_executable(project_id, issue_id, attempt_id)?
        };
        if running.scope == CodingAttemptScope::WorkItemGroup && running.worktree_path.is_some() {
            let mut attempt = self.store.update_attempt_stage(
                project_id,
                issue_id,
                attempt_id,
                CodingExecutionStage::Coding,
            )?;
            if attempt.head_commit.is_none() {
                let worktree_path = attempt.worktree_path.as_ref().ok_or_else(|| {
                    CodingWorkspaceEngineError::MissingWorktree(attempt.id.clone())
                })?;
                let base_head = self._git_service.git_current_head(worktree_path).await?;
                attempt = self.store.update_attempt_head_commit(
                    project_id,
                    issue_id,
                    attempt_id,
                    Some(base_head),
                )?;
            }
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingStageChange {
                    stage: CodingExecutionStage::Coding,
                })
                .await;
            return Ok(attempt);
        }
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
        let before_head = self
            ._git_service
            .git_ref_head(repo_path, &attempt.base_branch)
            .await?;
        let mut journal = self.store.prepare_coding_git_operation(
            attempt,
            crate::product::coding_attempt_store::PrepareCodingGitOperationInput {
                kind: crate::product::coding_attempt_store::CodingGitOperationKind::WorktreePrepare,
                repo_path: repo_path.to_path_buf(),
                worktree_path,
                branch_name: attempt.branch_name.clone(),
                base_branch: attempt.base_branch.clone(),
                before_head,
                remote: None,
                commit_message: None,
            },
        )?;
        if journal.phase == crate::product::coding_attempt_store::CodingGitOperationPhase::Before {
            let branch_result = self
                ._git_service
                .create_branch(repo_path, &journal.branch_name, &journal.base_branch)
                .await;
            if matches!(branch_result, Err(GitWorkspaceError::Cancelled { .. })) {
                self.compensate_cancelled_worktree_prepare(attempt, &journal)
                    .await?;
                return Err(CodingWorkspaceEngineError::Aborted);
            }
            branch_result?;
            journal = self.store.advance_coding_git_operation(
                attempt,
                &journal,
                crate::product::coding_attempt_store::CodingGitOperationPhase::BranchCreated,
                None,
            )?;
        }
        if journal.phase
            == crate::product::coding_attempt_store::CodingGitOperationPhase::BranchCreated
        {
            let worktree_result = self
                ._git_service
                .create_worktree(repo_path, &journal.branch_name, &journal.worktree_path)
                .await;
            if matches!(worktree_result, Err(GitWorkspaceError::Cancelled { .. })) {
                self.compensate_cancelled_worktree_prepare(attempt, &journal)
                    .await?;
                return Err(CodingWorkspaceEngineError::Aborted);
            }
            worktree_result?;
            journal = self.store.advance_coding_git_operation(
                attempt,
                &journal,
                crate::product::coding_attempt_store::CodingGitOperationPhase::WorktreeCreated,
                None,
            )?;
        }
        if journal.phase
            == crate::product::coding_attempt_store::CodingGitOperationPhase::Compensated
        {
            return Err(CodingWorkspaceEngineError::Aborted);
        }
        let updated = if journal.phase
            == crate::product::coding_attempt_store::CodingGitOperationPhase::WorktreeCreated
        {
            let updated = self.store.update_attempt_worktree_path(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                journal.worktree_path.clone(),
            )?;
            self.store.advance_coding_git_operation(
                &updated,
                &journal,
                crate::product::coding_attempt_store::CodingGitOperationPhase::Completed,
                None,
            )?;
            updated
        } else {
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        };
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

    pub(crate) async fn compensate_cancelled_worktree_prepare(
        &self,
        attempt: &CodingExecutionAttempt,
        journal: &crate::product::coding_attempt_store::CodingGitOperationJournal,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let git = GitWorkspaceService::new();
        match git
            .git_worktree_branch(&journal.repo_path, &journal.worktree_path)
            .await?
        {
            Some(branch) if branch == journal.branch_name => {
                git.remove_worktree(&journal.repo_path, &journal.worktree_path)
                    .await?;
            }
            Some(branch) => {
                return Err(GitWorkspaceError::UnsafePath(format!(
                    "worktree {} belongs to unexpected branch {branch}",
                    journal.worktree_path.display()
                ))
                .into());
            }
            None => {}
        }
        git.prune_worktrees(&journal.repo_path).await?;
        if let Some(head) = git
            .git_local_branch_head(&journal.repo_path, &journal.branch_name)
            .await?
        {
            if head != journal.before_head {
                return Err(GitWorkspaceError::UnsafePath(format!(
                    "branch {} moved from expected head {} to {head}",
                    journal.branch_name, journal.before_head
                ))
                .into());
            }
            git.delete_local_branch(&journal.repo_path, &journal.branch_name)
                .await?;
        }
        self.store.advance_coding_git_operation(
            attempt,
            journal,
            crate::product::coding_attempt_store::CodingGitOperationPhase::Compensated,
            None,
        )?;
        Ok(())
    }
}
