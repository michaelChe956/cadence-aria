use super::*;

pub(crate) fn internal_review_blocked_gate_reason(
    decision: CodeReviewFlowDecision,
    is_group_final_review: bool,
) -> Option<&'static str> {
    match decision {
        CodeReviewFlowDecision::RunCoderFix if is_group_final_review => {
            Some("group_final_review_blocked")
        }
        CodeReviewFlowDecision::RunCoderFix => Some("internal_review_blocked"),
        CodeReviewFlowDecision::OpenOperationalGate => Some("internal_review_operational_blocker"),
        CodeReviewFlowDecision::StopForHumanTriage => Some("internal_review_human_triage"),
        CodeReviewFlowDecision::RetryVerification
        | CodeReviewFlowDecision::StartPlanRepair
        | CodeReviewFlowDecision::StartStoryAmendment
        | CodeReviewFlowDecision::StartDesignAmendment
        | CodeReviewFlowDecision::ContinueAfterApprove => None,
    }
}
use crate::product::coding_models::CodingAttemptScope;

impl CodingWorkspaceEngine {
    pub async fn build_group_internal_pr_review_prompt_for_test(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let review_request = self
            .store
            .list_review_requests(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .last()
            .unwrap_or(ReviewRequest {
                id: "review_request_for_test".to_string(),
                attempt_id: attempt.id.clone(),
                kind: ReviewRequestKind::GitBranchOnly,
                remote_kind: crate::product::coding_models::RemoteKind::GenericGit,
                remote: "origin".to_string(),
                base_branch: attempt.base_branch.clone(),
                branch_name: attempt.branch_name.clone(),
                commit_sha: attempt
                    .head_commit
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                push_status: PushStatus::Pushed,
                external_url: None,
                manual_instructions: Vec::new(),
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            });
        self.build_group_internal_pr_review_prompt(attempt, &review_request, None, None)
            .await
    }

    async fn build_group_internal_pr_review_prompt(
        &self,
        attempt: &CodingExecutionAttempt,
        review_request: &ReviewRequest,
        worktree_path: Option<&Path>,
        retry_diagnostic: Option<&str>,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let handoffs = self.collect_completed_group_unit_handoffs(attempt)?;
        let units_section = self.format_group_unit_handoff_section(&handoffs);
        let evaluation_context_json = self.group_final_review_evaluation_context_json(attempt)?;
        let diff = match worktree_path {
            Some(path) => {
                self._git_service
                    .git_diff(path, &attempt.base_branch)
                    .await?
            }
            None => String::new(),
        };
        let retry_diagnostic_section = retry_diagnostic
            .map(|summary| format!("\n上一轮 role run 诊断摘要:\n{}\n", summary))
            .unwrap_or_default();
        let worktree_display = worktree_path
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "未提供".to_string());

        Ok(format!(
            "Coding Workspace GroupFinalReview\n\
             {}\n\
             你是 WorkItemGroup GroupFinalReview reviewer，仅在 WorkItemGroup 全部 coding units 完成且 ReviewRequest push 之后做整组功能审查。单 WorkItem scope 不应生成本 prompt。\n\
             Project: {}\n\
             Issue: {}\n\
             Scope: WorkItemGroup\n\
             Attempt: {}\n\
             Branch: {}\n\
             Review Request: {}\n\
             Review Remote: {}\n\
             Commit: {}\n\
             Worktree: {}\n\
             \nCompleted Units:\n{}\n\
             \nEvaluationContextPack:\n````json\n{}\n````\n\
             \n完整变更 git diff:\n````diff\n{}\n````\n\
             {}\
             {}\
             {}\
             {}\
             \n输出要求:\n\
             - 基于所有 completed units 的 handoff 汇总评估整组风险、测试覆盖和剩余问题。\n\
             - 分析影响范围（影响范围/impact_scope）。\n\
             - 给出 PR description 预览。\n\
             - 给出 commit message 建议。\n\
             - findings 必须包含 source_stage=group_final_review。\n\
             \n只输出 JSON：{{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[...],\"impact_scope\":[\"...\"],\"pr_description\":\"...\",\"commit_message_suggestion\":\"...\"}}\n",
            provider_runtime_contract("GroupFinalReview"),
            attempt.project_id,
            attempt.issue_id,
            attempt.id,
            attempt.branch_name,
            review_request.id,
            review_request.remote,
            review_request.commit_sha,
            worktree_display,
            units_section,
            evaluation_context_json,
            truncate_prompt_section(&diff, 30_000),
            group_final_review_material_protocol(),
            reviewer_test_scope_contract(),
            no_default_stack_assumption_contract(),
            retry_diagnostic_section
        ))
    }

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
        let attempt = self.store.ensure_provider_run_allowed(attempt)?;
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
        let is_group_final_review = attempt.scope == CodingAttemptScope::WorkItemGroup;
        let prepared_group_context = if is_group_final_review {
            Some(self.prepare_group_reviewer_context(&attempt, &reviewer)?)
        } else {
            None
        };
        let legacy_prompt = if is_group_final_review {
            self.build_group_internal_pr_review_prompt(
                &attempt,
                &review_request,
                Some(worktree_path.as_path()),
                retry_diagnostic.as_deref(),
            )
            .await?
        } else {
            self.build_internal_pr_review_prompt(
                &attempt,
                &review_request,
                worktree_path,
                retry_diagnostic.as_deref(),
            )
            .await?
        };
        let prompt = prepared_group_context
            .as_ref()
            .map(|context| format!("{}\n\n{legacy_prompt}", context.prompt_section))
            .unwrap_or(legacy_prompt);
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
                fresh_retry: None,
                timeout: None,
                timeout_reason_code: None,
            })
            .await?;
        let raw_provider_output_ref = self.store.save_provider_raw_output(
            &attempt,
            CodingExecutionStage::InternalPrReview,
            if is_group_final_review {
                "group_final_review"
            } else {
                "internal_pr_review"
            },
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
        let review_flow_decision = match prepared_group_context.as_ref() {
            Some(context) => {
                internal_review_flow_decision_with_bindings(&review, &context.bindings)
            }
            None => self.internal_review_flow_decision_for_attempt(&attempt, &review)?,
        };
        self.store.save_internal_pr_review(&attempt, &review)?;
        self.emit_internal_pr_review_chat_entry(
            &attempt,
            &node.id,
            &review,
            review_flow_decision.label(),
        )
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
                Some(
                    if is_group_final_review {
                        "GroupFinalReview 通过"
                    } else {
                        "internal PR review 通过"
                    }
                    .to_string(),
                ),
                CodingRoleRunStatus::Completed,
                None,
            ),
            ReviewVerdict::RequestChanges => (
                CodingTimelineNodeStatus::Failed,
                Some(
                    if is_group_final_review {
                        "GroupFinalReview 要求修改"
                    } else {
                        "internal PR review 要求修改"
                    }
                    .to_string(),
                ),
                CodingRoleRunStatus::Completed,
                None,
            ),
            ReviewVerdict::Blocked => (
                CodingTimelineNodeStatus::Blocked,
                Some(
                    if is_group_final_review {
                        "GroupFinalReview 被阻塞"
                    } else {
                        "internal PR review 被阻塞"
                    }
                    .to_string(),
                ),
                CodingRoleRunStatus::Blocked,
                Some(
                    if is_group_final_review {
                        "group_final_review_blocked"
                    } else {
                        "internal_review_blocked"
                    }
                    .to_string(),
                ),
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
        let blocked_gate_reason = (review.verdict == ReviewVerdict::Blocked)
            .then(|| {
                internal_review_blocked_gate_reason(review_flow_decision, is_group_final_review)
            })
            .flatten();
        if let Some(blocked_gate_reason) = blocked_gate_reason {
            let operational = blocked_gate_reason == "internal_review_operational_blocker";
            let human_triage = blocked_gate_reason == "internal_review_human_triage";
            self.create_review_blocked_gate(ReviewBlockedGateInput {
                attempt: &attempt,
                node_id: &node.id,
                stage: CodingExecutionStage::InternalPrReview,
                role: CodingProviderRole::InternalReviewer,
                title: if operational {
                    "Internal review operational blocker"
                } else if human_triage {
                    "Internal review requires human triage"
                } else if is_group_final_review {
                    "GroupFinalReview blocked"
                } else {
                    "Internal PR review blocked"
                }
                .to_string(),
                description: review.summary.clone(),
                reason_code: blocked_gate_reason,
                evidence_refs: vec![review.id.clone()],
                raw_provider_output_ref: Some(raw_provider_output_ref),
            })
            .await?;
        }
        Ok(review)
    }

    pub(crate) async fn execute_group_final_review_with_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<InternalPrReview, CodingWorkspaceEngineError> {
        let attempt = self.store.ensure_provider_run_allowed(attempt)?;
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt.id.clone(),
            ));
        }
        self.execute_internal_pr_review_with_commands(&attempt, provider, command_rx)
            .await
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

        let existing_journal = self.store.get_coding_git_operation(&attempt)?;
        let before_head = match existing_journal.as_ref() {
            Some(journal) if journal.kind == CodingGitOperationKind::ReviewRequest => {
                journal.before_head.clone()
            }
            _ => match self._git_service.git_current_head(worktree_path).await {
                Ok(head) => head,
                Err(GitWorkspaceError::Cancelled { .. }) => {
                    return Err(CodingWorkspaceEngineError::Aborted);
                }
                Err(error) => return Err(error.into()),
            },
        };
        let mut journal = self.store.prepare_coding_git_operation(
            &attempt,
            PrepareCodingGitOperationInput {
                kind: CodingGitOperationKind::ReviewRequest,
                repo_path: worktree_path.to_path_buf(),
                worktree_path: worktree_path.to_path_buf(),
                branch_name: attempt.branch_name.clone(),
                base_branch: attempt.base_branch.clone(),
                before_head,
                remote: Some(remote.to_string()),
                commit_message: Some(commit_message.to_string()),
            },
        )?;
        if journal.phase == CodingGitOperationPhase::Compensated {
            return Err(CodingWorkspaceEngineError::Aborted);
        }
        if journal.phase == CodingGitOperationPhase::Before {
            let add_result = self
                ._git_service
                .git_add_work_item_changes(worktree_path)
                .await;
            if matches!(add_result, Err(GitWorkspaceError::Cancelled { .. })) {
                self.compensate_cancelled_review_commit(&attempt, &journal)
                    .await?;
                return Err(CodingWorkspaceEngineError::Aborted);
            }
            add_result?;
            let has_staged_changes = match self
                ._git_service
                .git_has_staged_changes(worktree_path)
                .await
            {
                Ok(has_staged_changes) => has_staged_changes,
                Err(GitWorkspaceError::Cancelled { .. }) => {
                    self.compensate_cancelled_review_commit(&attempt, &journal)
                        .await?;
                    return Err(CodingWorkspaceEngineError::Aborted);
                }
                Err(error) => return Err(error.into()),
            };
            if has_staged_changes || attempt.head_commit.is_some() {
                journal = self.store.advance_coding_git_operation(
                    &attempt,
                    &journal,
                    CodingGitOperationPhase::CommitStarted,
                    None,
                )?;
            } else {
                self.store.advance_coding_git_operation(
                    &attempt,
                    &journal,
                    CodingGitOperationPhase::Compensated,
                    None,
                )?;
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
                    self.active_work_item_id_for_attempt(&attempt),
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
        }
        if journal.phase == CodingGitOperationPhase::CommitStarted {
            let current_head = GitWorkspaceService::new()
                .git_current_head(worktree_path)
                .await?;
            let commit_sha = if current_head != journal.before_head {
                self.confirm_review_commit_identity(&journal, &current_head)
                    .await?;
                current_head
            } else if attempt.head_commit.is_some() {
                current_head
            } else {
                match self
                    ._git_service
                    .git_commit(worktree_path, commit_message)
                    .await
                {
                    Ok(commit) => commit.commit_sha,
                    Err(GitWorkspaceError::Cancelled { .. }) => {
                        self.compensate_cancelled_review_commit(&attempt, &journal)
                            .await?;
                        return Err(CodingWorkspaceEngineError::Aborted);
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            journal = self.store.advance_coding_git_operation(
                &attempt,
                &journal,
                CodingGitOperationPhase::CommitCreated,
                Some(commit_sha),
            )?;
        }
        let push_just_started = if journal.phase == CodingGitOperationPhase::CommitCreated {
            journal = self.store.advance_coding_git_operation(
                &attempt,
                &journal,
                CodingGitOperationPhase::PushStarted,
                None,
            )?;
            true
        } else {
            false
        };
        if journal.phase == CodingGitOperationPhase::PushStarted {
            let commit_sha = journal.commit_sha.as_deref().ok_or_else(|| {
                ProductStoreError::IdentityMismatch {
                    kind: "coding_git_operation",
                    id: attempt.id.clone(),
                }
            })?;
            let remote_head = if push_just_started {
                Ok(None)
            } else {
                self._git_service
                    .git_remote_branch_head(worktree_path, remote, &attempt.branch_name)
                    .await
            };
            if matches!(remote_head.as_ref(), Ok(Some(head)) if head == commit_sha) {
                journal = self
                    .finish_review_git_operation(&attempt, &journal, PushStatus::Pushed)
                    .await?;
            } else if matches!(remote_head, Err(GitWorkspaceError::Cancelled { .. })) {
                let Some(completed) = self
                    .reconcile_cancelled_review_push(&attempt, &journal)
                    .await?
                else {
                    return Err(CodingWorkspaceEngineError::Aborted);
                };
                journal = completed;
            } else {
                remote_head?;
                match self
                    ._git_service
                    .git_push(worktree_path, remote, &attempt.branch_name)
                    .await
                {
                    Ok(push) => {
                        journal = if push.status == PushStatus::Pushed {
                            self.finish_review_git_operation(&attempt, &journal, PushStatus::Pushed)
                                .await?
                        } else {
                            self.finish_nonzero_review_push(
                                &attempt,
                                &journal,
                                push.remote_rejected,
                            )
                            .await?
                        };
                    }
                    Err(GitWorkspaceError::Cancelled { .. }) => {
                        let Some(completed) = self
                            .reconcile_cancelled_review_push(&attempt, &journal)
                            .await?
                        else {
                            return Err(CodingWorkspaceEngineError::Aborted);
                        };
                        journal = completed;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        if journal.phase != CodingGitOperationPhase::Completed {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_git_operation",
                id: attempt.id.clone(),
            }
            .into());
        }
        let request = self.persist_review_request_from_git_journal(&attempt, &journal)?;
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
                self.active_work_item_id_for_attempt(&attempt),
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
