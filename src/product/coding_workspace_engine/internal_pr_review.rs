use super::*;

impl CodingWorkspaceEngine {
    pub async fn execute_internal_pr_review(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
    ) -> Result<InternalPrReview, CodingWorkspaceEngineError> {
        let (_command_tx, mut command_rx) = mpsc::channel(1);
        self.execute_internal_pr_review_with_commands(attempt, provider, &mut command_rx)
            .await
    }

    pub async fn execute_internal_pr_review_with_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<InternalPrReview, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let review_request = self
            .store
            .list_review_requests(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .last()
            .ok_or_else(|| CodingWorkspaceEngineError::MissingReviewRequest(attempt.id.clone()))?;
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::InternalPrReview,
        )?;
        let node = self.create_internal_pr_review_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let role_run = match self.store.latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
        )? {
            Some(run) if run.status == CodingRoleRunStatus::Running && run.node_id.is_none() => {
                self.store.attach_role_run_node(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &run.id,
                    node.id.clone(),
                )?
            }
            _ => self.store.create_role_run(
                &attempt,
                CodingExecutionStage::InternalPrReview,
                CodingProviderRole::InternalReviewer,
                CodingRoleRunTrigger::Initial,
                Some(node.id.clone()),
            )?,
        };

        let reviewer = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .internal_reviewer;
        let retry_diagnostic = self.retry_diagnostic_for_previous_run(&attempt, &role_run)?;
        let prompt = self
            .build_internal_pr_review_prompt(
                &attempt,
                &review_request,
                worktree_path,
                retry_diagnostic.as_deref(),
            )
            .await?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &reviewer,
                    prompt.clone(),
                    CodingPromptMode::FullConversation.event_detail(),
                ),
            })
            .await;
        let input = AdapterInput {
            provider_type: provider_type_for_name(&reviewer),
            role: AdapterRole::Reviewer,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt,
            context_files: Vec::new(),
            output_schema: "coding_workspace_internal_pr_review_json".to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::InternalReviewer,
            &reviewer,
        );
        let mut provider_input = streaming_input_from_adapter(&input, worktree_path.clone());
        provider_input.workspace_session_id = Some(attempt.id.clone());
        provider_input.resume_provider_session_id = resume_provider_session_id;
        provider_input.permission_mode = role_permission_mode_for_attempt(
            &self.store,
            &attempt,
            CodingProviderRole::InternalReviewer,
        )?;
        let full_output = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                role_run: Some(&role_run),
                provider,
                legacy_input: &input,
                input: provider_input,
                provider_name: &reviewer,
                provider_role: CodingProviderRole::InternalReviewer,
                command_rx,
                allow_legacy_stream_fallback: true,
                timeout: None,
                timeout_reason_code: None,
            })
            .await?;
        let raw_provider_output_ref = self.store.save_provider_raw_output(
            &attempt.id,
            CodingExecutionStage::InternalPrReview,
            "internal_pr_review",
            &full_output,
        )?;
        self.store.update_role_run_refs(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            vec![raw_provider_output_ref.clone()],
            Vec::new(),
        )?;
        let review = self.build_internal_pr_review(
            &attempt,
            &review_request,
            &full_output,
            Some(raw_provider_output_ref.clone()),
            &role_run,
        )?;
        self.store.save_internal_pr_review(&review)?;
        self.emit_internal_pr_review_chat_entry(&attempt, &node.id, &review)
            .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::InternalPrReviewComplete {
                review: Box::new(review.clone()),
            })
            .await;
        let (node_status, summary, role_run_status, reason_code) = match review.verdict {
            ReviewVerdict::Approve => (
                CodingTimelineNodeStatus::Completed,
                Some("internal PR review 通过".to_string()),
                CodingRoleRunStatus::Completed,
                None,
            ),
            ReviewVerdict::RequestChanges => (
                CodingTimelineNodeStatus::Failed,
                Some("internal PR review 要求修改".to_string()),
                CodingRoleRunStatus::Completed,
                None,
            ),
            ReviewVerdict::Blocked => (
                CodingTimelineNodeStatus::Blocked,
                Some("internal PR review 被阻塞".to_string()),
                CodingRoleRunStatus::Blocked,
                Some("internal_review_blocked".to_string()),
            ),
        };
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            node_status,
            summary,
        )
        .await?;
        self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            role_run_status,
            reason_code,
        )?;
        Ok(review)
    }

    pub async fn execute_review_request(
        &self,
        attempt: &CodingExecutionAttempt,
        remote: &str,
        commit_message: &str,
    ) -> Result<ReviewRequest, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )?;
        let node = self.create_review_request_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        self._git_service
            .git_add_work_item_changes(worktree_path)
            .await?;
        if !self
            ._git_service
            .git_has_staged_changes(worktree_path)
            .await?
        {
            let summary =
                "过滤运行产物后没有可提交的业务变更，请检查上一轮 Coder 是否只修改了运行产物。"
                    .to_string();
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )?;
            self.release_active_lock_if_shared_worktree_clean(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &attempt.work_item_id,
            )
            .await?;
            self.complete_timeline_node(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &node.id,
                CodingTimelineNodeStatus::Blocked,
                Some(summary.clone()),
            )
            .await?;
            return Err(CodingWorkspaceEngineError::NoReviewableChanges(summary));
        }
        let commit = self
            ._git_service
            .git_commit(worktree_path, commit_message)
            .await?;
        let push = self
            ._git_service
            .git_push(worktree_path, remote, &attempt.branch_name)
            .await?;
        let remote_kind = self._git_service.detect_remote_kind(worktree_path).await?;
        let existing_requests =
            self.store
                .list_review_requests(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let now = Utc::now().to_rfc3339();
        let request = ReviewRequest {
            id: next_sequential_id("review_request", existing_requests.len()),
            attempt_id: attempt.id.clone(),
            kind: ReviewRequestKind::GitBranchOnly,
            remote_kind,
            remote: remote.to_string(),
            base_branch: attempt.base_branch.clone(),
            branch_name: attempt.branch_name.clone(),
            commit_sha: commit.commit_sha,
            push_status: push.status,
            external_url: None,
            manual_instructions: vec![format!(
                "基于远端 {remote}/{} 发起代码审查",
                attempt.branch_name
            )],
            created_at: now.clone(),
            updated_at: now,
        };
        self.store.save_review_request(&request)?;
        self.store.update_attempt_review_request_state(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            request.commit_sha.clone(),
            remote.to_string(),
            request.id.clone(),
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::ReviewRequestUpdate {
                review_request: Box::new(request.clone()),
            })
            .await;

        let (node_status, summary) = if request.push_status == PushStatus::Pushed {
            (
                CodingTimelineNodeStatus::Completed,
                Some("review request 已创建".to_string()),
            )
        } else {
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )?;
            self.release_active_lock_if_shared_worktree_clean(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &attempt.work_item_id,
            )
            .await?;
            (
                CodingTimelineNodeStatus::Failed,
                Some("review request 推送失败".to_string()),
            )
        };
        let completed_at = Utc::now().to_rfc3339();
        self.store.update_timeline_node_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            node_status.clone(),
            summary.clone(),
            Some(completed_at.clone()),
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                node_id: node.id,
                status: node_status,
                summary,
                completed_at: Some(completed_at),
            })
            .await;
        Ok(request)
    }
}
