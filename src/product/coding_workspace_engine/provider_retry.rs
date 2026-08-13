use super::*;
use crate::protocol::provider_errors::ProviderErrorCode;

const MAX_PROVIDER_INVOCATIONS_PER_CYCLE: u32 = 3;

pub(crate) struct ProviderRetryCycleSuccess {
    pub(crate) outcome: ProviderStreamOutcome,
    pub(crate) role_run: CodingRoleRun,
    pub(crate) node: CodingTimelineNode,
}

pub(crate) struct CoderRetryCycleInput<'a> {
    pub(crate) attempt: &'a CodingExecutionAttempt,
    pub(crate) initial_node: CodingTimelineNode,
    pub(crate) initial_role_run: CodingRoleRun,
    pub(crate) provider: &'a dyn StreamingProviderAdapter,
    pub(crate) provider_name: &'a ProviderName,
    pub(crate) worktree_path: &'a Path,
    pub(crate) initial_prompt: String,
    pub(crate) fresh_prompt: String,
    pub(crate) initial_prompt_mode: CodingPromptMode,
    pub(crate) initial_resume_provider_session_id: Option<String>,
    pub(crate) command_rx: &'a mut mpsc::Receiver<CodingRunnerCommand>,
}

pub(crate) struct CodeReviewerRetryCycleInput<'a> {
    pub(crate) attempt: &'a CodingExecutionAttempt,
    pub(crate) initial_node: CodingTimelineNode,
    pub(crate) initial_role_run: CodingRoleRun,
    pub(crate) provider: &'a dyn StreamingProviderAdapter,
    pub(crate) reviewer: &'a ProviderName,
    pub(crate) worktree_path: &'a Path,
    pub(crate) initial_resume_provider_session_id: Option<String>,
    pub(crate) command_rx: &'a mut mpsc::Receiver<CodingRunnerCommand>,
}

/// Provider 调用中允许交给外层协调器自动恢复的技术失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RetryableProviderFailure {
    StartIo,
    StreamEnded,
    ConnectionInterrupted,
    ExecutionTimeout,
    Upstream5xx { status: u16 },
}

impl RetryableProviderFailure {
    pub(crate) fn reason_code(&self) -> &'static str {
        match self {
            Self::StartIo => "provider_start_io",
            Self::StreamEnded => "provider_stream_ended",
            Self::ConnectionInterrupted => "provider_connection_interrupted",
            Self::ExecutionTimeout => "provider_execution_timeout",
            Self::Upstream5xx { .. } => "provider_upstream_5xx",
        }
    }

    pub(crate) fn is_reason_code(reason_code: &str) -> bool {
        [
            Self::StartIo,
            Self::StreamEnded,
            Self::ConnectionInterrupted,
            Self::ExecutionTimeout,
            Self::Upstream5xx { status: 500 },
        ]
        .iter()
        .any(|failure| failure.reason_code() == reason_code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderFailureClassification {
    Retryable {
        failure: RetryableProviderFailure,
        reason_code: String,
        message: String,
    },
    NonRetryable {
        reason_code: String,
        interaction_wait: bool,
    },
}

impl ProviderFailureClassification {
    #[allow(dead_code)]
    pub(crate) fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable { .. })
    }

    #[allow(dead_code)]
    pub(crate) fn is_interaction_wait(&self) -> bool {
        matches!(
            self,
            Self::NonRetryable {
                interaction_wait: true,
                ..
            }
        )
    }
}

/// 一次 Provider 调用的边界结果。协调器只应依据此结果决定是否创建下一次 role run。
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum ProviderInvocationOutcome {
    Completed(ProviderStreamOutcome),
    Cancelled,
    NonRetryable {
        reason_code: String,
        error: CodingWorkspaceEngineError,
        interaction_wait: bool,
    },
    RetryableTransport {
        failure: RetryableProviderFailure,
        reason_code: String,
        message: String,
        partial_output: String,
    },
}

impl ProviderInvocationOutcome {
    #[allow(dead_code)]
    pub(crate) fn from_result(
        result: Result<ProviderStreamOutcome, CodingWorkspaceEngineError>,
        partial_output: String,
    ) -> Self {
        match result {
            Ok(outcome) => Self::Completed(outcome),
            Err(CodingWorkspaceEngineError::Aborted) => Self::Cancelled,
            Err(error) => match classify_provider_failure(&error) {
                ProviderFailureClassification::Retryable {
                    failure,
                    reason_code,
                    message,
                } => Self::RetryableTransport {
                    failure,
                    reason_code,
                    message,
                    partial_output,
                },
                ProviderFailureClassification::NonRetryable {
                    reason_code,
                    interaction_wait,
                } => Self::NonRetryable {
                    reason_code,
                    error,
                    interaction_wait,
                },
            },
        }
    }
}

impl CodingWorkspaceEngine {
    pub(crate) async fn run_coder_with_retry_cycle(
        &self,
        input: CoderRetryCycleInput<'_>,
    ) -> Result<ProviderRetryCycleSuccess, CodingWorkspaceEngineError> {
        let CoderRetryCycleInput {
            attempt,
            initial_node,
            initial_role_run,
            provider,
            provider_name,
            worktree_path,
            initial_prompt,
            fresh_prompt,
            initial_prompt_mode,
            initial_resume_provider_session_id,
            command_rx,
        } = input;
        let permission_mode =
            role_permission_mode_for_attempt(&self.store, attempt, CodingProviderRole::Coder)?;
        let mut node = initial_node;
        let mut role_run = self.ensure_retry_cycle_metadata(attempt, initial_role_run)?;

        for attempt_no in 1..=MAX_PROVIDER_INVOCATIONS_PER_CYCLE {
            if attempt_no > 1 {
                (node, role_run) = self
                    .prepare_automatic_retry_invocation(
                        attempt,
                        CodingExecutionStage::Coding,
                        CodingProviderRole::Coder,
                        &role_run,
                    )
                    .await?;
            }

            let (prompt, prompt_mode, resume_provider_session_id) = if attempt_no == 1 {
                (
                    initial_prompt.clone(),
                    initial_prompt_mode,
                    initial_resume_provider_session_id.clone(),
                )
            } else {
                (
                    fresh_prompt.clone(),
                    CodingPromptMode::FullConversation,
                    None,
                )
            };
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingExecutionEvent {
                    event: provider_prompt_event(
                        &node.id,
                        provider_name,
                        prompt.clone(),
                        prompt_mode.event_detail(),
                    ),
                })
                .await;
            let legacy_input = AdapterInput {
                provider_type: provider_type_for_name(provider_name),
                role: AdapterRole::Executor,
                worktree_path: Some(worktree_path.to_string_lossy().to_string()),
                provider_stream_log_dir: Some(self.attempt_provider_stream_log_dir(attempt)),
                prompt: prompt.clone(),
                context_files: Vec::new(),
                output_schema: "coding_workspace_markdown".to_string(),
                timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
                max_retries: 0,
            };
            let mut provider_input = streaming_input_from_adapter(
                &legacy_input,
                worktree_path.to_path_buf(),
                permission_mode.clone(),
            );
            provider_input.workspace_session_id = Some(attempt.id.clone());
            provider_input.resume_provider_session_id = resume_provider_session_id;
            let invocation_attempt = match self.ensure_provider_retry_cycle_active(attempt) {
                Ok(current) => current,
                Err(error) => {
                    self.finalize_retry_cycle_state_change(attempt, &node, &role_run)
                        .await?;
                    return Err(error);
                }
            };
            // Task 6:逻辑 target + 已注入 gateway 时生产 validated input；否则为
            // None（Legacy 直连）。provider_input 在 move 进 run 结构前先 clone,
            // 保证 validated 与 input 同源。
            let validated_input = self
                .validated_streaming_input_for_role(
                    &invocation_attempt,
                    CodingProviderRole::Coder,
                    provider_input.clone(),
                )
                .map_err(|error| CodingWorkspaceEngineError::ProviderStream(error.to_string()))?;
            let outcome = self
                .run_provider_stream_invocation(CodingProviderStreamRun {
                    attempt: &invocation_attempt,
                    node_id: &node.id,
                    role_run: Some(&role_run),
                    provider,
                    legacy_input: &legacy_input,
                    input: provider_input,
                    provider_name,
                    provider_role: CodingProviderRole::Coder,
                    command_rx: &mut *command_rx,
                    // Task 12:逻辑 target 不得回落到 legacy `run_streaming` bridge;
                    // 传统/非逻辑路径保留 `true` 以兼容未实现 `start` 的历史 adapter。
                    allow_legacy_stream_fallback: invocation_attempt.target_snapshot.is_none(),
                    timeout: None,
                    timeout_reason_code: None,
                    suppress_failure_side_effects: true,
                    validated_input,
                })
                .await;
            if let Some(outcome) = self
                .resolve_provider_retry_cycle_outcome(
                    &invocation_attempt,
                    &node,
                    &role_run,
                    attempt_no,
                    outcome,
                )
                .await?
            {
                return Ok(ProviderRetryCycleSuccess {
                    outcome,
                    role_run,
                    node,
                });
            }
        }

        unreachable!("provider retry cycle is bounded to three invocations")
    }

    pub(crate) async fn run_code_reviewer_with_retry_cycle(
        &self,
        input: CodeReviewerRetryCycleInput<'_>,
    ) -> Result<ProviderRetryCycleSuccess, CodingWorkspaceEngineError> {
        let CodeReviewerRetryCycleInput {
            attempt,
            initial_node,
            initial_role_run,
            provider,
            reviewer,
            worktree_path,
            initial_resume_provider_session_id,
            command_rx,
        } = input;
        let permission_mode = role_permission_mode_for_attempt(
            &self.store,
            attempt,
            CodingProviderRole::CodeReviewer,
        )?;
        let mut node = initial_node;
        let mut role_run = self.ensure_retry_cycle_metadata(attempt, initial_role_run)?;

        for attempt_no in 1..=MAX_PROVIDER_INVOCATIONS_PER_CYCLE {
            if attempt_no > 1 {
                (node, role_run) = self
                    .prepare_automatic_retry_invocation(
                        attempt,
                        CodingExecutionStage::CodeReview,
                        CodingProviderRole::CodeReviewer,
                        &role_run,
                    )
                    .await?;
            }

            let retry_diagnostic = self.retry_diagnostic_for_previous_run(attempt, &role_run)?;
            let nonce = crate::product::workspace_engine::structured_output_nonce();
            let structured_output_contract = code_review_structured_output_contract(nonce.clone());
            let terminal_contract = code_review_output_contract(&nonce);
            let mut prompt = match self.render_reviewer_unit_run_context(attempt, reviewer)? {
                Some(rendered) => rendered.text,
                None => {
                    self.build_code_review_prompt(
                        attempt,
                        worktree_path,
                        retry_diagnostic.as_deref(),
                    )
                    .await?
                }
            };
            if !prompt.ends_with(&terminal_contract) {
                prompt.push_str(&terminal_contract);
            }
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingExecutionEvent {
                    event: provider_prompt_event(
                        &node.id,
                        reviewer,
                        prompt.clone(),
                        CodingPromptMode::FullConversation.event_detail(),
                    ),
                })
                .await;
            let legacy_input = AdapterInput {
                provider_type: provider_type_for_name(reviewer),
                role: AdapterRole::Reviewer,
                worktree_path: Some(worktree_path.to_string_lossy().to_string()),
                provider_stream_log_dir: Some(self.attempt_provider_stream_log_dir(attempt)),
                prompt,
                context_files: Vec::new(),
                output_schema: "coding_workspace_code_review_json".to_string(),
                timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
                max_retries: 0,
            };
            let mut provider_input = streaming_input_from_adapter(
                &legacy_input,
                worktree_path.to_path_buf(),
                permission_mode.clone(),
            );
            provider_input.workspace_session_id = Some(attempt.id.clone());
            provider_input.resume_provider_session_id = if attempt_no == 1 {
                initial_resume_provider_session_id.clone()
            } else {
                None
            };
            provider_input.structured_output_contract = Some(structured_output_contract);
            let invocation_attempt = match self.ensure_provider_retry_cycle_active(attempt) {
                Ok(current) => current,
                Err(error) => {
                    self.finalize_retry_cycle_state_change(attempt, &node, &role_run)
                        .await?;
                    return Err(error);
                }
            };
            let outcome = self
                .run_provider_stream_invocation(CodingProviderStreamRun {
                    attempt: &invocation_attempt,
                    node_id: &node.id,
                    role_run: Some(&role_run),
                    provider,
                    legacy_input: &legacy_input,
                    input: provider_input,
                    provider_name: reviewer,
                    provider_role: CodingProviderRole::CodeReviewer,
                    command_rx: &mut *command_rx,
                    // Task 12:逻辑 target 不得回落到 legacy `run_streaming` bridge;
                    // 传统/非逻辑路径保留 `true`。
                    allow_legacy_stream_fallback: invocation_attempt.target_snapshot.is_none(),
                    timeout: None,
                    timeout_reason_code: None,
                    suppress_failure_side_effects: true,
                    validated_input: None,
                })
                .await;
            if let Some(outcome) = self
                .resolve_provider_retry_cycle_outcome(
                    &invocation_attempt,
                    &node,
                    &role_run,
                    attempt_no,
                    outcome,
                )
                .await?
            {
                return Ok(ProviderRetryCycleSuccess {
                    outcome,
                    role_run,
                    node,
                });
            }
        }

        unreachable!("provider retry cycle is bounded to three invocations")
    }

    fn ensure_retry_cycle_metadata(
        &self,
        attempt: &CodingExecutionAttempt,
        mut role_run: CodingRoleRun,
    ) -> Result<CodingRoleRun, CodingWorkspaceEngineError> {
        if role_run.retry_metadata.is_none() {
            role_run.retry_metadata =
                Some(crate::product::coding_models::CodingRoleRunRetryMetadata {
                    cycle_id: role_run.id.clone(),
                    attempt_no: 1,
                    prior_run_id: None,
                });
            self.store
                .save_role_run(&attempt.project_id, &attempt.issue_id, &role_run)?;
        }
        Ok(role_run)
    }

    fn create_automatic_retry_role_run(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        prior_run: &CodingRoleRun,
    ) -> Result<CodingRoleRun, CodingWorkspaceEngineError> {
        let prior_retry = prior_run.retry_metadata.as_ref().ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream(
                "provider_retry_cycle_metadata_missing".to_string(),
            )
        })?;
        self.store
            .create_retry_role_run(
                attempt,
                stage,
                role,
                CodingRoleRunTrigger::AutomaticRetry,
                None,
                crate::product::coding_models::CodingRoleRunRetryMetadata {
                    cycle_id: prior_retry.cycle_id.clone(),
                    attempt_no: prior_retry.attempt_no + 1,
                    prior_run_id: Some(prior_run.id.clone()),
                },
            )
            .map_err(CodingWorkspaceEngineError::from)
    }

    async fn prepare_automatic_retry_invocation(
        &self,
        expected: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        prior_run: &CodingRoleRun,
    ) -> Result<(CodingTimelineNode, CodingRoleRun), CodingWorkspaceEngineError> {
        let current = self.ensure_provider_retry_cycle_active(expected)?;
        #[cfg(test)]
        crate::product::coding_workspace_engine::mutation_test_pause::pause_coding_mutation_for_test(
            self.store.paths().root(),
            crate::product::coding_workspace_engine::mutation_test_pause::CodingMutationTestPoint::ProviderFailure,
        )
        .await;
        let current = self.ensure_provider_retry_cycle_active(&current)?;
        let role_run =
            match self.create_automatic_retry_role_run(&current, stage.clone(), role, prior_run) {
                Ok(role_run) => role_run,
                Err(error) => return Err(error),
            };
        let node = match stage {
            CodingExecutionStage::Coding => self.create_coding_timeline_node(&current),
            CodingExecutionStage::CodeReview => self.create_code_review_timeline_node(&current),
            _ => unreachable!("automatic retry only supports coder and code reviewer"),
        };
        let node = match node {
            Ok(node) => node,
            Err(error) => {
                self.store.update_role_run_status(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    &role_run.id,
                    CodingRoleRunStatus::Failed,
                    Some("automatic_retry_timeline_create_failed".to_string()),
                )?;
                return Err(error.into());
            }
        };
        let role_run = match self.store.attach_role_run_node(
            &current.project_id,
            &current.issue_id,
            &current.id,
            &role_run.id,
            node.id.clone(),
        ) {
            Ok(role_run) => role_run,
            Err(error) => {
                self.store.update_role_run_status(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    &role_run.id,
                    CodingRoleRunStatus::Failed,
                    Some("automatic_retry_timeline_bind_failed".to_string()),
                )?;
                self.complete_timeline_node(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    &node.id,
                    CodingTimelineNodeStatus::Failed,
                    Some("绑定自动重试 role run 失败".to_string()),
                )
                .await?;
                return Err(error.into());
            }
        };
        if let Err(error) = self.ensure_provider_retry_cycle_active(&current) {
            self.finalize_retry_cycle_state_change(&current, &node, &role_run)
                .await?;
            return Err(error);
        }
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;
        Ok((node, role_run))
    }

    async fn finalize_retry_cycle_state_change(
        &self,
        attempt: &CodingExecutionAttempt,
        node: &CodingTimelineNode,
        role_run: &CodingRoleRun,
    ) -> Result<(), CodingWorkspaceEngineError> {
        self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            CodingRoleRunStatus::Failed,
            Some("provider_retry_attempt_state_changed".to_string()),
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Failed,
            Some("自动重试前 attempt 状态已变更".to_string()),
        )
        .await?;
        Ok(())
    }

    fn ensure_provider_retry_cycle_active(
        &self,
        expected: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.validate_attempt_issue_shared_worktree_lock_if_present(expected)?;
        let current =
            self.store
                .get_attempt(&expected.project_id, &expected.issue_id, &expected.id)?;
        if current.status != CodingAttemptStatus::Running || current.stage != expected.stage {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "provider_retry_attempt_state_changed: {}",
                expected.id
            )));
        }
        let current = self.store.ensure_provider_run_allowed(&current)?;
        if current.status != CodingAttemptStatus::Running || current.stage != expected.stage {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "provider_retry_attempt_state_changed: {}",
                expected.id
            )));
        }
        self.validate_attempt_issue_shared_worktree_lock_if_present(&current)?;
        Ok(current)
    }

    async fn resolve_provider_retry_cycle_outcome(
        &self,
        attempt: &CodingExecutionAttempt,
        node: &CodingTimelineNode,
        role_run: &CodingRoleRun,
        attempt_no: u32,
        outcome: ProviderInvocationOutcome,
    ) -> Result<Option<ProviderStreamOutcome>, CodingWorkspaceEngineError> {
        match outcome {
            ProviderInvocationOutcome::Completed(outcome) => Ok(Some(outcome)),
            ProviderInvocationOutcome::Cancelled => {
                self.finalize_cancelled_provider_retry_cycle(attempt, node, role_run)
                    .await?;
                Err(CodingWorkspaceEngineError::Aborted)
            }
            ProviderInvocationOutcome::RetryableTransport {
                failure: _,
                reason_code,
                message,
                partial_output: _,
            } => {
                if let Err(error) = self.ensure_provider_retry_cycle_active(attempt) {
                    self.finalize_retry_cycle_state_change(attempt, node, role_run)
                        .await?;
                    return Err(error);
                }
                self.store.update_role_run_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &role_run.id,
                    CodingRoleRunStatus::Failed,
                    Some(reason_code),
                )?;
                self.complete_timeline_node(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &node.id,
                    CodingTimelineNodeStatus::Failed,
                    Some(message.clone()),
                )
                .await?;
                if attempt_no < MAX_PROVIDER_INVOCATIONS_PER_CYCLE {
                    Ok(None)
                } else {
                    self.fail_provider_stream::<Option<ProviderStreamOutcome>>(
                        attempt, &node.id, message,
                    )
                    .await
                }
            }
            ProviderInvocationOutcome::NonRetryable {
                reason_code,
                error,
                interaction_wait,
            } => {
                if interaction_wait && reason_code != "permission_timeout" {
                    self.store.update_role_run_status(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        &role_run.id,
                        CodingRoleRunStatus::Blocked,
                        Some(reason_code),
                    )?;
                    self.complete_timeline_node(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        &node.id,
                        CodingTimelineNodeStatus::Blocked,
                        Some(error.to_string()),
                    )
                    .await?;
                    return Err(error);
                }
                match error {
                    CodingWorkspaceEngineError::ProviderAdapter(error) => {
                        self.fail_provider_stream::<Option<ProviderStreamOutcome>>(
                            attempt,
                            &node.id,
                            error.details,
                        )
                        .await
                    }
                    CodingWorkspaceEngineError::ProviderStream(message)
                    | CodingWorkspaceEngineError::ProviderProtocol(message) => {
                        self.fail_provider_stream::<Option<ProviderStreamOutcome>>(
                            attempt, &node.id, message,
                        )
                        .await
                    }
                    error => {
                        self.store.update_role_run_status(
                            &attempt.project_id,
                            &attempt.issue_id,
                            &attempt.id,
                            &role_run.id,
                            CodingRoleRunStatus::Failed,
                            Some(reason_code),
                        )?;
                        self.complete_timeline_node(
                            &attempt.project_id,
                            &attempt.issue_id,
                            &attempt.id,
                            &node.id,
                            CodingTimelineNodeStatus::Failed,
                            Some(error.to_string()),
                        )
                        .await?;
                        Err(error)
                    }
                }
            }
        }
    }

    async fn finalize_cancelled_provider_retry_cycle(
        &self,
        attempt: &CodingExecutionAttempt,
        node: &CodingTimelineNode,
        role_run: &CodingRoleRun,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        if current.status != CodingAttemptStatus::Aborted {
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Aborted,
            )?;
        }
        self.release_issue_shared_worktree_lock_for_attempt(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            CodingRoleRunStatus::Aborted,
            Some("abort_attempt".to_string()),
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Failed,
            Some("用户已中止".to_string()),
        )
        .await?;
        Ok(())
    }
}

pub(crate) fn classify_provider_failure(
    error: &CodingWorkspaceEngineError,
) -> ProviderFailureClassification {
    match error {
        CodingWorkspaceEngineError::ProviderAdapter(error) => classify_adapter_error(error),
        CodingWorkspaceEngineError::ProviderStream(message) => classify_stream_message(message),
        CodingWorkspaceEngineError::ProviderProtocol(_) => {
            non_retryable("provider_protocol", false)
        }
        CodingWorkspaceEngineError::Aborted => non_retryable("abort_attempt", false),
        _ => non_retryable("provider_business_failure", false),
    }
}

fn classify_adapter_error(error: &ProviderAdapterError) -> ProviderFailureClassification {
    let message = adapter_error_message(error);
    if is_permission_wait(&message) {
        return non_retryable("permission_timeout", true);
    }
    if is_choice_wait(&message) {
        return non_retryable("choice_timeout", true);
    }
    if let Some(classification) = classify_transport_message(&message) {
        return classification;
    }
    match error.code {
        ProviderErrorCode::ProviderTimeout => {
            retryable(RetryableProviderFailure::ExecutionTimeout, message)
        }
        ProviderErrorCode::ProviderExecutionFailed => {
            non_retryable("provider_execution_failed", false)
        }
        ProviderErrorCode::ProviderParseError => non_retryable("provider_parse_error", false),
        ProviderErrorCode::ProviderCommandMissing => {
            non_retryable("provider_command_missing", false)
        }
        ProviderErrorCode::ProviderUnavailable => non_retryable("provider_unavailable", false),
        ProviderErrorCode::ProviderUnauthorized => non_retryable("provider_unauthorized", false),
        ProviderErrorCode::ProviderPermissionDenied => {
            non_retryable("provider_permission_denied", false)
        }
        ProviderErrorCode::ProviderIncompatibleOutput => {
            non_retryable("provider_incompatible_output", false)
        }
    }
}

fn adapter_error_message(error: &ProviderAdapterError) -> String {
    let mut parts = vec![format!("{:?}: {}", error.code, error.details)];
    if !error.stdout.is_empty() {
        parts.push(format!("stdout: {}", error.stdout));
    }
    if !error.stderr.is_empty() {
        parts.push(format!("stderr: {}", error.stderr));
    }
    parts.join("; ")
}

fn classify_stream_message(message: &str) -> ProviderFailureClassification {
    if message.eq_ignore_ascii_case("provider_choice_unresolved") {
        return non_retryable("provider_choice_unresolved", true);
    }
    if is_permission_wait(message) {
        return non_retryable("permission_timeout", true);
    }
    if is_choice_wait(message) {
        return non_retryable("choice_timeout", true);
    }
    if message.to_ascii_lowercase().contains("protocol") {
        return non_retryable("provider_protocol", false);
    }
    if message.to_ascii_lowercase().contains("structured output")
        || message.to_ascii_lowercase().contains("parser")
    {
        return non_retryable("provider_structured_output", false);
    }
    classify_transport_message(message)
        .unwrap_or_else(|| non_retryable("provider_stream_failed", false))
}

fn classify_transport_message(message: &str) -> Option<ProviderFailureClassification> {
    let lower = message.to_ascii_lowercase();
    if let Some(status) = upstream_status(&lower) {
        return Some(retryable(
            RetryableProviderFailure::Upstream5xx { status },
            message.to_string(),
        ));
    }
    if lower.contains("stream ended")
        || lower.contains("stream closed")
        || lower.contains("eof")
        || lower.contains("closed before completion")
    {
        return Some(retryable(
            RetryableProviderFailure::StreamEnded,
            message.to_string(),
        ));
    }
    if lower.contains("connection")
        || lower.contains("broken pipe")
        || lower.contains("process interrupted")
        || lower.contains("process exited")
        || lower.contains("process terminated")
        || crate::cross_cutting::codex_provider::is_resume_stall_failure(message)
        || lower.contains("resume stalled")
    {
        return Some(retryable(
            RetryableProviderFailure::ConnectionInterrupted,
            message.to_string(),
        ));
    }
    if lower.contains("timeout") || lower.contains("timed out") {
        return Some(retryable(
            RetryableProviderFailure::ExecutionTimeout,
            message.to_string(),
        ));
    }
    if is_start_io_message(&lower) {
        return Some(retryable(
            RetryableProviderFailure::StartIo,
            message.to_string(),
        ));
    }
    None
}

fn upstream_status(message: &str) -> Option<u16> {
    let tokens = message
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    tokens.iter().enumerate().find_map(|(index, token)| {
        let status = token.parse::<u16>().ok()?;
        if !matches!(status, 503 | 504) {
            return None;
        }
        let context = &tokens[index.saturating_sub(5)..index];
        let nearest_identifier = context
            .iter()
            .enumerate()
            .rev()
            .find_map(|(offset, token)| {
                matches!(*token, "id" | "port" | "count").then_some(offset)
            });
        let nearest_upstream_context = context
            .iter()
            .enumerate()
            .rev()
            .find_map(|(offset, token)| is_upstream_status_context(token).then_some(offset));
        let identifier_is_closer = nearest_identifier
            .zip(nearest_upstream_context)
            .is_some_and(|(identifier, upstream)| identifier > upstream);
        (!identifier_is_closer && nearest_upstream_context.is_some()).then_some(status)
    })
}

fn is_upstream_status_context(token: &str) -> bool {
    matches!(token, "http" | "status" | "upstream" | "gateway")
}

fn is_start_io_message(message: &str) -> bool {
    message.contains("text file busy")
        || message.contains("resource temporarily unavailable")
        || ((message.contains("start") || message.contains("spawn"))
            && (message.contains("i/o")
                || message.contains("io error")
                || message.contains("input/output")))
}

fn is_permission_wait(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("permission") && (lower.contains("timeout") || lower.contains("timed out"))
}

fn is_choice_wait(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("choice") && (lower.contains("timeout") || lower.contains("timed out"))
}

fn retryable(failure: RetryableProviderFailure, message: String) -> ProviderFailureClassification {
    ProviderFailureClassification::Retryable {
        reason_code: failure.reason_code().to_string(),
        failure,
        message,
    }
}

fn non_retryable(reason_code: &str, interaction_wait: bool) -> ProviderFailureClassification {
    ProviderFailureClassification::NonRetryable {
        reason_code: reason_code.to_string(),
        interaction_wait,
    }
}
