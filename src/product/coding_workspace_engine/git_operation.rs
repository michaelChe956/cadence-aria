use super::*;

#[cfg(test)]
mod push_decision_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewPushDecision {
    Pushed,
    Failed,
    Indeterminate,
}

fn review_push_decision<E>(
    expected_commit: &str,
    remote_head: Result<Option<&str>, E>,
) -> ReviewPushDecision {
    match remote_head {
        Ok(Some(remote_commit)) if remote_commit == expected_commit => ReviewPushDecision::Pushed,
        Ok(_) => ReviewPushDecision::Failed,
        Err(_) => ReviewPushDecision::Indeterminate,
    }
}

impl CodingWorkspaceEngine {
    pub(crate) async fn reconcile_coding_git_operation_for_termination(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let Some(journal) = self.store.get_coding_git_operation(attempt)? else {
            return Ok(attempt.clone());
        };
        match journal.kind {
            CodingGitOperationKind::WorktreePrepare => {
                if !journal.is_terminal() {
                    self.compensate_cancelled_worktree_prepare(attempt, &journal)
                        .await?;
                }
            }
            CodingGitOperationKind::ReviewRequest => match journal.phase {
                CodingGitOperationPhase::Before
                | CodingGitOperationPhase::CommitStarted
                | CodingGitOperationPhase::CommitCreated => {
                    self.compensate_cancelled_review_commit(attempt, &journal)
                        .await?;
                }
                CodingGitOperationPhase::PushStarted => {
                    if let Some(completed) = self
                        .reconcile_cancelled_review_push(attempt, &journal)
                        .await?
                    {
                        self.persist_review_request_from_git_journal(attempt, &completed)?;
                    }
                }
                CodingGitOperationPhase::Completed => {
                    self.persist_review_request_from_git_journal(attempt, &journal)?;
                }
                CodingGitOperationPhase::Compensated => {}
                CodingGitOperationPhase::BranchCreated
                | CodingGitOperationPhase::WorktreeCreated => {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "coding_git_operation",
                        id: journal.attempt_id,
                    }
                    .into());
                }
            },
        }
        self.store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .map_err(Into::into)
    }

    pub(crate) async fn confirm_review_commit_identity(
        &self,
        journal: &CodingGitOperationJournal,
        commit_sha: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let (parent, message) = GitWorkspaceService::new()
            .git_commit_parent_and_message(&journal.worktree_path, commit_sha)
            .await?;
        if parent != journal.before_head
            || journal.commit_message.as_deref() != Some(message.as_str())
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_git_operation",
                id: journal.attempt_id.clone(),
            }
            .into());
        }
        Ok(())
    }

    pub(crate) async fn compensate_cancelled_review_commit(
        &self,
        attempt: &CodingExecutionAttempt,
        journal: &CodingGitOperationJournal,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let git = GitWorkspaceService::new();
        let current_head = git.git_current_head(&journal.worktree_path).await?;
        let mut journal = journal.clone();
        if current_head != journal.before_head {
            self.confirm_review_commit_identity(&journal, &current_head)
                .await?;
            journal = self.store.advance_coding_git_operation(
                attempt,
                &journal,
                CodingGitOperationPhase::CommitCreated,
                Some(current_head),
            )?;
        }
        git.git_reset_mixed(&journal.worktree_path, &journal.before_head)
            .await?;
        self.store.advance_coding_git_operation(
            attempt,
            &journal,
            CodingGitOperationPhase::Compensated,
            None,
        )?;
        Ok(())
    }

    pub(crate) async fn finish_review_git_operation(
        &self,
        attempt: &CodingExecutionAttempt,
        journal: &CodingGitOperationJournal,
        push_status: PushStatus,
    ) -> Result<CodingGitOperationJournal, CodingWorkspaceEngineError> {
        let remote_kind = GitWorkspaceService::new()
            .detect_remote_kind(&journal.worktree_path)
            .await?;
        let existing_requests =
            self.store
                .list_review_requests(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let review_request_id = attempt
            .review_request_id
            .clone()
            .unwrap_or_else(|| next_sequential_id("review_request", existing_requests.len()));
        self.store
            .complete_review_coding_git_operation(
                attempt,
                journal,
                CompleteReviewGitOperationInput {
                    push_status,
                    remote_kind,
                    review_request_id,
                },
            )
            .map_err(Into::into)
    }

    pub(crate) async fn finish_nonzero_review_push(
        &self,
        attempt: &CodingExecutionAttempt,
        journal: &CodingGitOperationJournal,
    ) -> Result<CodingGitOperationJournal, CodingWorkspaceEngineError> {
        let remote =
            journal
                .remote
                .as_deref()
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "coding_git_operation",
                    id: journal.attempt_id.clone(),
                })?;
        let commit_sha =
            journal
                .commit_sha
                .as_deref()
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "coding_git_operation",
                    id: journal.attempt_id.clone(),
                })?;
        let remote_head = self
            ._git_service
            .git_remote_branch_head(&journal.worktree_path, remote, &journal.branch_name)
            .await;
        match review_push_decision(commit_sha, remote_head.as_ref().map(|head| head.as_deref())) {
            ReviewPushDecision::Pushed => {
                self.finish_review_git_operation(attempt, journal, PushStatus::Pushed)
                    .await
            }
            ReviewPushDecision::Failed => {
                self.finish_review_git_operation(attempt, journal, PushStatus::Failed)
                    .await
            }
            ReviewPushDecision::Indeterminate => Err(remote_head
                .expect_err("indeterminate push requires remote query error")
                .into()),
        }
    }

    pub(crate) async fn reconcile_cancelled_review_push(
        &self,
        attempt: &CodingExecutionAttempt,
        journal: &CodingGitOperationJournal,
    ) -> Result<Option<CodingGitOperationJournal>, CodingWorkspaceEngineError> {
        let remote =
            journal
                .remote
                .as_deref()
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "coding_git_operation",
                    id: journal.attempt_id.clone(),
                })?;
        let commit_sha =
            journal
                .commit_sha
                .as_deref()
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "coding_git_operation",
                    id: journal.attempt_id.clone(),
                })?;
        let git = GitWorkspaceService::new();
        if git
            .git_remote_branch_head(&journal.worktree_path, remote, &journal.branch_name)
            .await?
            .as_deref()
            == Some(commit_sha)
        {
            return self
                .finish_review_git_operation(attempt, journal, PushStatus::Pushed)
                .await
                .map(Some);
        }
        git.git_reset_mixed(&journal.worktree_path, &journal.before_head)
            .await?;
        self.store.advance_coding_git_operation(
            attempt,
            journal,
            CodingGitOperationPhase::Compensated,
            None,
        )?;
        Ok(None)
    }

    pub(crate) fn persist_review_request_from_git_journal(
        &self,
        attempt: &CodingExecutionAttempt,
        journal: &CodingGitOperationJournal,
    ) -> Result<ReviewRequest, CodingWorkspaceEngineError> {
        let request = ReviewRequest {
            id: journal.review_request_id.clone().ok_or_else(|| {
                ProductStoreError::IdentityMismatch {
                    kind: "coding_git_operation",
                    id: journal.attempt_id.clone(),
                }
            })?,
            attempt_id: attempt.id.clone(),
            kind: ReviewRequestKind::GitBranchOnly,
            remote_kind: journal.remote_kind.clone().ok_or_else(|| {
                ProductStoreError::IdentityMismatch {
                    kind: "coding_git_operation",
                    id: journal.attempt_id.clone(),
                }
            })?,
            remote: journal
                .remote
                .clone()
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "coding_git_operation",
                    id: journal.attempt_id.clone(),
                })?,
            base_branch: journal.base_branch.clone(),
            branch_name: journal.branch_name.clone(),
            commit_sha: journal.commit_sha.clone().ok_or_else(|| {
                ProductStoreError::IdentityMismatch {
                    kind: "coding_git_operation",
                    id: journal.attempt_id.clone(),
                }
            })?,
            push_status: journal.push_status.clone().ok_or_else(|| {
                ProductStoreError::IdentityMismatch {
                    kind: "coding_git_operation",
                    id: journal.attempt_id.clone(),
                }
            })?,
            external_url: None,
            manual_instructions: vec![format!(
                "基于远端 {}/{} 发起代码审查",
                journal.remote.as_deref().unwrap_or_default(),
                journal.branch_name
            )],
            created_at: journal.updated_at.clone(),
            updated_at: journal.updated_at.clone(),
        };
        self.store.save_review_request(attempt, &request)?;
        self.store.update_attempt_review_request_state(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            request.commit_sha.clone(),
            request.remote.clone(),
            request.id.clone(),
        )?;
        Ok(request)
    }
}
