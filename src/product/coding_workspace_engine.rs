use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Deserializer};
use serde_json::json;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::{
    ProviderAdapter, DEFAULT_PROVIDER_TIMEOUT_SECS, ProviderAdapterError,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole};
use crate::cross_cutting::worktree::{scope_allows_path, validate_write_path};
use crate::cross_cutting::streaming_provider::{
    ChoiceRequestData, PermissionRequestData, ProviderCommand, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, ProviderStatus, ProviderToolCall, ProviderToolResult, RiskLevel,
    StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CreateBlockedGateInput, CreateChoiceGateInput,
    CreateQualityBypassAuditInput,
};
use crate::product::coding_evaluation_context::{
    EvaluationContextRole, build_evaluation_context_pack,
};
use crate::product::coding_models::{
    AnalystDecisionNextStage, AnalystDecisionRecord, AnalystDecisionVerdict,
    AnalystHumanGateRecommendation, AnalystReworkInstructions, AnalystVerdict, CodeReviewReport,
    CodingAgentRole, CodingAttemptStatus, CodingChatEntry, CodingChoiceOption, CodingContextNote,
    CodingEntryType, CodingExecutionAttempt, CodingExecutionStage, CodingGateAction,
    CodingGateActionType, CodingGateRequired, CodingProviderPermissionMode, CodingProviderRole,
    CodingReworkInstruction, CodingRoleRun, CodingRoleRunEventType, CodingRoleRunStatus,
    CodingRoleRunTrigger, CodingTimelineNode, CodingTimelineNodeStatus, InternalPrReview,
    PushStatus, ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict, TestCommand,
    TestCommandStatus, TestPlan, TestPlanRiskLevel, TestingOverallStatus, TestingReport,
    TestingStepResult, TestingUnplannedEvidence, WorkItemHandoff,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::{GitWorkspaceError, GitWorkspaceService};
use crate::product::id::next_sequential_id;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    LifecycleWorkItemRecord, ProviderConversationRef, ProviderConversationRole, ProviderName,
    WorkItemStatus, WorkspaceType,
};
use crate::product::test_executor::{TestCommandSpec, TestExecutorError, run_all_tests};
use crate::product::tester_agent_loop::{
    TesterAgentOptions, build_plan_based_testing_report, build_tester_execute_repair_prompt,
    build_tester_plan_prompt, build_tester_plan_repair_prompt, build_testing_report,
    execute_tester_tool_call, format_test_plan_chat_summary, format_testing_report_chat_summary,
    parse_test_plan_payload,
};
use crate::protocol::contracts::ProviderType;
use crate::web::coding_ws_handler::CodingWsOutMessage;
use crate::web::workspace_ws_types::{
    ChoiceOption, WsExecutionEvent, WsExecutionEventKind, WsExecutionEventStatus,
    WsPermissionRiskLevel,
};

pub const TESTING_RESULT_REVIEW_REASON_CODE: &str = "testing_result_review_required";

const REWORK_CONTEXT_NOTE_CHAR_LIMIT: usize = 10_000;

#[derive(Debug, Error)]
pub enum CodingWorkspaceEngineError {
    #[error(transparent)]
    Store(#[from] ProductStoreError),
    #[error(transparent)]
    Git(#[from] GitWorkspaceError),
    #[error(transparent)]
    TestExecutor(#[from] TestExecutorError),
    #[error(transparent)]
    TesterAgent(#[from] crate::product::tester_agent_loop::TesterAgentError),
    #[error(transparent)]
    ProviderAdapter(#[from] ProviderAdapterError),
    #[error("coding_provider_stream_failed: {0}")]
    ProviderStream(String),
    #[error("coding_aborted")]
    Aborted,
    #[error("coding_rework_limit_exceeded: {0}")]
    ReworkLimitExceeded(String),
    #[error("coding_review_request_missing: {0}")]
    MissingReviewRequest(String),
    #[error("coding_attempt_missing_worktree: {0}")]
    MissingWorktree(String),
    #[error("coding_attempt_not_ready_for_final_confirm: {0}")]
    FinalConfirmNotReady(String),
    #[error("{0}")]
    NoReviewableChanges(String),
    #[error("shared_worktree_dirty_manual_gate: {0}")]
    SharedWorktreeDirtyManualGate(String),
    #[error("work_item_execution_plan_not_confirmed: {0}")]
    ExecutionPlanNotConfirmed(String),
    #[error("completion_commit_missing: {0}")]
    CompletionCommitMissing(String),
    #[error("work_item_handoff_missing: {0}")]
    WorkItemHandoffMissing(String),
    #[error("verification_gate_result_missing: {0}")]
    VerificationGateResultMissing(String),
    #[error("verification_gate_failed: {0}")]
    VerificationGateFailed(String),
    #[error("work_item_diff_scope_violation: {0}")]
    WorkItemDiffScopeViolation(String),
}

#[derive(Debug, Clone)]
pub struct CompletionGateReport;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodingExecutionContext {
    pub work_item_markdown: Option<String>,
    pub verification_commands: Vec<String>,
}

struct CodingProviderStreamRun<'a> {
    attempt: &'a CodingExecutionAttempt,
    node_id: &'a str,
    role_run: Option<&'a CodingRoleRun>,
    provider: &'a dyn StreamingProviderAdapter,
    legacy_input: &'a AdapterInput,
    input: StreamingProviderInput,
    provider_name: &'a ProviderName,
    provider_role: CodingProviderRole,
    command_rx: &'a mut mpsc::Receiver<CodingRunnerCommand>,
    allow_legacy_stream_fallback: bool,
    timeout: Option<Duration>,
    timeout_reason_code: Option<&'static str>,
}

struct BlockedTestingGateContext<'a> {
    reason_code: String,
    description: String,
    raw_provider_output_ref: Option<String>,
    role_run: Option<&'a CodingRoleRun>,
}

fn run_timeout_sleep(timeout: Option<Duration>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    match timeout {
        Some(duration) => Box::pin(tokio::time::sleep(duration)),
        None => Box::pin(std::future::pending()),
    }
}

fn provider_conversation_role_for_coding_role(
    role: &CodingProviderRole,
) -> ProviderConversationRole {
    match role {
        CodingProviderRole::Coder => ProviderConversationRole::Coder,
        CodingProviderRole::Tester => ProviderConversationRole::Tester,
        CodingProviderRole::Analyst => ProviderConversationRole::Analyst,
        CodingProviderRole::CodeReviewer => ProviderConversationRole::CodeReviewer,
        CodingProviderRole::InternalReviewer => ProviderConversationRole::InternalReviewer,
    }
}

fn should_resume_provider_conversation(role: &CodingProviderRole) -> bool {
    matches!(role, CodingProviderRole::Coder)
}

fn coding_provider_permission_mode(mode: CodingProviderPermissionMode) -> ProviderPermissionMode {
    match mode {
        CodingProviderPermissionMode::Auto => ProviderPermissionMode::Auto,
        CodingProviderPermissionMode::Supervised => ProviderPermissionMode::Supervised,
    }
}

fn role_permission_mode_for_attempt(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role: CodingProviderRole,
) -> Result<ProviderPermissionMode, CodingWorkspaceEngineError> {
    let snapshot = store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    Ok(coding_provider_permission_mode(
        snapshot.permission_mode_for_role(&role),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodingPromptMode {
    FullConversation,
    DeltaOnly,
}

impl CodingPromptMode {
    fn event_detail(self) -> &'static str {
        match self {
            Self::FullConversation => "发送给 Coding provider 的完整提示词",
            Self::DeltaOnly => "发送给 Coding provider 的增量提示词",
        }
    }
}

#[derive(Clone)]
pub struct CodingWorkspaceEngine {
    store: CodingAttemptStore,
    _git_service: GitWorkspaceService,
    provider: Option<Arc<dyn ProviderAdapter + Send + Sync>>,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
}

impl std::fmt::Debug for CodingWorkspaceEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodingWorkspaceEngine")
            .field("store", &self.store)
            .field("event_tx", &self.event_tx)
            .finish_non_exhaustive()
    }
}

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

    fn provider_resume_session_id_for_attempt(
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

    fn record_attempt_provider_session(
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

    pub async fn execute_coding(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        context: &CodingExecutionContext,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let (_command_tx, mut command_rx) = mpsc::channel(1);
        self.execute_coding_with_commands(attempt, provider, context, &mut command_rx)
            .await
    }

    pub async fn execute_coding_with_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        context: &CodingExecutionContext,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
        )?;
        let node = self.create_coding_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let coder_provider = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .coder;
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Coder,
            &coder_provider,
        );
        let rework_instruction = self.store.latest_unconsumed_rework_instruction(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let context_notes = self.store.list_unconsumed_context_notes(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let context_note_ids = context_notes
            .iter()
            .map(|note| note.id.clone())
            .collect::<Vec<_>>();
        let context_note_input =
            format_rework_context_notes(&context_notes, REWORK_CONTEXT_NOTE_CHAR_LIMIT);
        let coding_context_notes = (!context_note_ids.is_empty()).then_some(&context_note_input);
        let prompt_mode = if resume_provider_session_id.is_some() {
            CodingPromptMode::DeltaOnly
        } else {
            CodingPromptMode::FullConversation
        };
        let prompt = match prompt_mode {
            CodingPromptMode::FullConversation => build_coding_prompt(
                &attempt,
                context,
                rework_instruction.as_ref(),
                coding_context_notes,
            ),
            CodingPromptMode::DeltaOnly => build_coding_delta_prompt(
                &attempt,
                context,
                rework_instruction.as_ref(),
                coding_context_notes,
            ),
        };
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &coder_provider,
                    prompt.clone(),
                    prompt_mode.event_detail(),
                ),
            })
            .await;
        if let Some(instruction) = rework_instruction.as_ref() {
            self.store.mark_rework_instruction_consumed(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &instruction.id,
                &node.id,
            )?;
        }
        if !context_note_ids.is_empty() {
            self.store.mark_context_notes_consumed(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &context_note_ids,
                attempt.rework_count,
            )?;
        }

        let legacy_input = AdapterInput {
            provider_type: provider_type_for_name(&coder_provider),
            role: AdapterRole::Executor,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt,
            context_files: Vec::new(),
            output_schema: "coding_workspace_markdown".to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };
        let input = StreamingProviderInput {
            provider_type: legacy_input.provider_type.clone(),
            role: legacy_input.role.clone(),
            prompt: legacy_input.prompt.clone(),
            working_dir: worktree_path.clone(),
            workspace_session_id: Some(attempt.id.clone()),
            resume_provider_session_id,
            permission_mode: role_permission_mode_for_attempt(
                &self.store,
                &attempt,
                CodingProviderRole::Coder,
            )?,
            env_vars: BTreeMap::new(),
            timeout_secs: legacy_input.timeout,
        };
        let _full_output = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                role_run: None,
                provider,
                legacy_input: &legacy_input,
                input,
                provider_name: &coder_provider,
                provider_role: CodingProviderRole::Coder,
                command_rx,
                allow_legacy_stream_fallback: true,
                timeout: None,
                timeout_reason_code: None,
            })
            .await?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Completed,
            Some("代码编写完成".to_string()),
        )
        .await?;
        Ok(attempt)
    }

    fn record_role_run_event(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: Option<&CodingRoleRun>,
        event_type: CodingRoleRunEventType,
        payload: serde_json::Value,
    ) {
        let Some(role_run) = role_run else {
            return;
        };
        if let Err(error) = self
            .store
            .append_role_run_event(attempt, role_run, event_type, payload)
        {
            tracing::warn!(
                role_run_id = role_run.id.as_str(),
                event_type = ?event_type,
                error = %error,
                "failed to persist coding role run event"
            );
        }
    }

    fn unresolved_provider_choice_error(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: Option<&CodingRoleRun>,
        phase: &str,
        open_choice_ids: &[String],
    ) -> CodingWorkspaceEngineError {
        self.record_role_run_event(
            attempt,
            role_run,
            CodingRoleRunEventType::ProviderFailed,
            json!({
                "phase": phase,
                "code": "provider_choice_unresolved",
                "message": "provider continued before required user choice was resolved",
                "choice_ids": open_choice_ids
            }),
        );
        CodingWorkspaceEngineError::ProviderStream("provider_choice_unresolved".to_string())
    }

    async fn run_provider_stream_to_completion(
        &self,
        run: CodingProviderStreamRun<'_>,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let CodingProviderStreamRun {
            attempt,
            node_id,
            role_run,
            provider,
            legacy_input,
            input,
            provider_name,
            provider_role,
            command_rx,
            allow_legacy_stream_fallback,
            timeout,
            timeout_reason_code,
        } = run;
        let cancel = CancellationToken::new();
        self.record_role_run_event(
            attempt,
            role_run,
            CodingRoleRunEventType::ProviderPrompt,
            json!({
                "provider": provider_name,
                "role": format!("{provider_role:?}"),
                "output_schema": legacy_input.output_schema.clone(),
                "prompt": legacy_input.prompt.clone()
            }),
        );
        let start_result = if let Some(duration) = timeout {
            tokio::select! {
                result = provider.start(input, cancel.clone()) => result,
                _ = tokio::time::sleep(duration) => {
                    cancel.cancel();
                    self.record_role_run_event(
                        attempt,
                        role_run,
                        CodingRoleRunEventType::Timeout,
                        json!({
                            "phase": "provider_start",
                            "reason_code": timeout_reason_code
                                .unwrap_or("provider_stream_timeout")
                        }),
                    );
                    return Err(CodingWorkspaceEngineError::ProviderStream(
                        timeout_reason_code
                            .unwrap_or("provider_stream_timeout")
                            .to_string(),
                    ));
                }
            }
        } else {
            provider.start(input, cancel.clone()).await
        };
        let mut session = match start_result {
            Ok(session) => {
                self.record_role_run_event(
                    attempt,
                    role_run,
                    CodingRoleRunEventType::ProviderStart,
                    json!({
                        "provider": provider_name,
                        "role": format!("{provider_role:?}")
                    }),
                );
                session
            }
            Err(error)
                if provider_start_is_not_implemented(&error) && allow_legacy_stream_fallback =>
            {
                return self
                    .run_legacy_stream_to_completion(attempt, node_id, provider, legacy_input)
                    .await;
            }
            Err(error) if !allow_legacy_stream_fallback => {
                let message = error.details;
                self.record_role_run_event(
                    attempt,
                    role_run,
                    CodingRoleRunEventType::ProviderFailed,
                    json!({
                        "phase": "provider_start",
                        "message": message.clone()
                    }),
                );
                return Err(CodingWorkspaceEngineError::ProviderStream(message));
            }
            Err(error) => {
                let message = error.details;
                self.record_role_run_event(
                    attempt,
                    role_run,
                    CodingRoleRunEventType::ProviderFailed,
                    json!({
                        "phase": "provider_start",
                        "message": message.clone()
                    }),
                );
                return self.fail_provider_stream(attempt, node_id, message).await;
            }
        };
        let mut commands_open = true;
        let mut full_output = String::new();
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();
        let mut open_choice_ids = Vec::<String>::new();
        let timeout = run_timeout_sleep(timeout);
        tokio::pin!(timeout);
        loop {
            tokio::select! {
                _ = &mut timeout => {
                    cancel.cancel();
                    self.record_role_run_event(
                        attempt,
                        role_run,
                        CodingRoleRunEventType::Timeout,
                        json!({
                            "phase": "provider_stream",
                            "reason_code": timeout_reason_code
                                .unwrap_or("provider_stream_timeout")
                        }),
                    );
                    return Err(CodingWorkspaceEngineError::ProviderStream(
                        timeout_reason_code
                            .unwrap_or("provider_stream_timeout")
                            .to_string(),
                    ));
                }
                command = command_rx.recv(), if commands_open => {
                    let Some(command) = command else {
                        commands_open = false;
                        continue;
                    };
                    match command {
                        CodingRunnerCommand::AbortAttempt => {
                            let _ = session.commands.send(ProviderCommand::Abort).await;
                            cancel.cancel();
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_provider_status(
                                        node_id,
                                        provider_name,
                                        ProviderStatus::Aborted,
                                    ),
                                })
                                .await;
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::Aborted,
                                json!({
                                    "reason": "abort_attempt"
                                }),
                            );
                            return Err(CodingWorkspaceEngineError::Aborted);
                        }
                        CodingRunnerCommand::ChoiceResponse {
                            id,
                            selected_option_ids,
                            free_text,
                        } => {
                            if !open_choice_ids.iter().any(|choice_id| choice_id == &id) {
                                let _ = self
                                    .event_tx
                                    .send(CodingWsOutMessage::CodingProtocolError {
                                        code: "coding_choice_gate_not_found".to_string(),
                                        message: format!(
                                            "ChoiceResponse id={id} not found in open coding choice gates"
                                        ),
                                    })
                                    .await;
                                continue;
                            }
                            if session
                                .commands
                                .send(ProviderCommand::ChoiceResponse {
                                    id: id.clone(),
                                    selected_option_ids: selected_option_ids.clone(),
                                    free_text: free_text.clone(),
                                })
                                .await
                                .is_ok()
                            {
                                let ack_selected_option_ids = selected_option_ids.clone();
                                let ack_free_text = free_text.clone();
                                let _ = self.store.resolve_choice_gate(
                                    &attempt.project_id,
                                    &attempt.issue_id,
                                    &attempt.id,
                                    &id,
                                    selected_option_ids,
                                    free_text,
                                )?;
                                open_choice_ids.retain(|choice_id| choice_id != &id);
                                let current = self.store.get_attempt(
                                    &attempt.project_id,
                                    &attempt.issue_id,
                                    &attempt.id,
                                )?;
                                if current.status == CodingAttemptStatus::WaitingForHuman {
                                    self.store.update_attempt_status(
                                        &attempt.project_id,
                                        &attempt.issue_id,
                                        &attempt.id,
                                        CodingAttemptStatus::Running,
                                    )?;
                                }
                                let _ = self
                                    .event_tx
                                    .send(CodingWsOutMessage::CodingChoiceResponseAck {
                                        id,
                                        selected_option_ids: ack_selected_option_ids,
                                        free_text: ack_free_text,
                                    })
                                    .await;
                            } else {
                                commands_open = false;
                            }
                        }
                        command => {
                            if !forward_runner_command_to_provider(command, &session.commands).await {
                                commands_open = false;
                            }
                        }
                    }
                }
                event = session.events.recv() => {
                    let Some(event) = event else {
                        if !open_choice_ids.is_empty() {
                            return Err(self.unresolved_provider_choice_error(
                                attempt,
                                role_run,
                                "provider_stream_closed",
                                &open_choice_ids,
                            ));
                        }
                        return self.fail_provider_stream_ended(attempt, node_id).await;
                    };
                    match event {
                        ProviderEvent::TextDelta { content } => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    attempt,
                                    role_run,
                                    "provider_text_delta",
                                    &open_choice_ids,
                                ));
                            }
                            let content_for_event = content.clone();
                            full_output.push_str(&content);
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingStreamChunk {
                                    content,
                                    node_id: Some(node_id.to_string()),
                                })
                                .await;
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::TextDelta,
                                json!({
                                    "content": content_for_event
                                }),
                            );
                        }
                        ProviderEvent::Execution(event) => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    attempt,
                                    role_run,
                                    "provider_execution",
                                    &open_choice_ids,
                                ));
                            }
                            let event_for_record = event.clone();
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_provider_execution(
                                        event,
                                        node_id,
                                        provider_name,
                                    ),
                                })
                                .await;
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ExecutionEvent,
                                json!({
                                    "event_id": event_for_record.event_id,
                                    "kind": format!("{:?}", event_for_record.kind),
                                    "status": format!("{:?}", event_for_record.status),
                                    "title": event_for_record.title,
                                    "detail": event_for_record.detail,
                                    "command": event_for_record.command,
                                    "cwd": event_for_record.cwd,
                                    "output": event_for_record.output,
                                    "exit_code": event_for_record.exit_code
                                }),
                            );
                        }
                        ProviderEvent::ToolCall(call) => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    attempt,
                                    role_run,
                                    "provider_tool_call",
                                    &open_choice_ids,
                                ));
                            }
                            let call_for_record = call.clone();
                            tool_call_titles.insert(call.id.clone(), call.tool_name.clone());
                            if let Some(command) = extract_tool_command(&call.input) {
                                tool_call_commands.insert(call.id.clone(), command);
                            }
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_tool_call(node_id, provider_name, call),
                                })
                                .await;
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ToolCall,
                                json!({
                                    "id": call_for_record.id,
                                    "tool_name": call_for_record.tool_name,
                                    "input": call_for_record.input
                                }),
                            );
                        }
                        ProviderEvent::ToolResult(result) => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    attempt,
                                    role_run,
                                    "provider_tool_result",
                                    &open_choice_ids,
                                ));
                            }
                            let result_for_record = result.clone();
                            let title = tool_call_titles
                                .get(&result.tool_use_id)
                                .cloned()
                                .unwrap_or_else(|| "Tool result".to_string());
                            let command = tool_call_commands.get(&result.tool_use_id).cloned();
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_tool_result(
                                        node_id,
                                        provider_name,
                                        &title,
                                        command,
                                        result,
                                    ),
                                })
                                .await;
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ToolResult,
                                json!({
                                    "tool_use_id": result_for_record.tool_use_id,
                                    "output": result_for_record.output,
                                    "is_error": result_for_record.is_error
                                }),
                            );
                        }
                        ProviderEvent::PermissionRequest(request) => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    attempt,
                                    role_run,
                                    "provider_permission_request",
                                    &open_choice_ids,
                                ));
                            }
                            let request_for_record = request.clone();
                            self.emit_permission_request(node_id, provider_name, request).await;
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::PermissionRequest,
                                json!({
                                    "id": request_for_record.id,
                                    "tool_name": request_for_record.tool_name,
                                    "description": request_for_record.description,
                                    "risk_level": format!("{:?}", request_for_record.risk_level)
                                }),
                            );
                        }
                        ProviderEvent::ChoiceRequest(request) => {
                            let request_for_record = request.clone();
                            self.emit_choice_request(
                                attempt,
                                node_id,
                                attempt.stage.clone(),
                                provider_role.clone(),
                                provider_name,
                                request,
                            )
                            .await?;
                            open_choice_ids.push(request_for_record.id.clone());
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ChoiceRequest,
                                json!({
                                    "id": request_for_record.id,
                                    "prompt": request_for_record.prompt,
                                    "allow_multiple": request_for_record.allow_multiple,
                                    "allow_free_text": request_for_record.allow_free_text,
                                    "source": request_for_record.source.as_str()
                                }),
                            );
                        }
                        ProviderEvent::StatusChanged(status) => {
                            let status_for_record = status.clone();
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_provider_status(
                                        node_id,
                                        provider_name,
                                        status,
                                    ),
                                })
                                .await;
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::StatusChanged,
                                json!({
                                    "status": format!("{status_for_record:?}")
                                }),
                            );
                        }
                        ProviderEvent::Completed {
                            full_output: completed_output,
                            provider_session_id,
                        } => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    attempt,
                                    role_run,
                                    "provider_completed",
                                    &open_choice_ids,
                                ));
                            }
                            let provider_session_id_for_record = provider_session_id.clone();
                            let output_bytes = completed_output.len();
                            self.record_attempt_provider_session(
                                attempt,
                                &provider_role,
                                provider_name.clone(),
                                provider_session_id,
                                node_id,
                            )?;
                            if !completed_output.trim().is_empty() {
                                full_output = completed_output;
                            }
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingMessageComplete {
                                    node_id: Some(node_id.to_string()),
                                })
                                .await;
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::MessageComplete,
                                json!({
                                    "provider_session_id": provider_session_id_for_record,
                                    "output_bytes": output_bytes
                                }),
                            );
                            return Ok(full_output);
                        }
                        ProviderEvent::Failed { message } => {
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ProviderFailed,
                                json!({
                                    "message": message.clone()
                                }),
                            );
                            return self.fail_provider_stream(attempt, node_id, message).await;
                        }
                        ProviderEvent::ProtocolError {
                            code,
                            message,
                            context,
                        } => {
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ProviderFailed,
                                json!({
                                    "code": code,
                                    "message": message.clone(),
                                    "context": context
                                }),
                            );
                            return self.fail_provider_stream(attempt, node_id, message).await;
                        }
                        ProviderEvent::PermissionTimeout { permission_id } => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    attempt,
                                    role_run,
                                    "provider_permission_timeout",
                                    &open_choice_ids,
                                ));
                            }
                            let message = format!("Permission request {permission_id} timed out");
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::Timeout,
                                json!({
                                    "permission_id": permission_id,
                                    "reason": "permission_timeout",
                                    "message": message.clone()
                                }),
                            );
                            return self
                                .fail_provider_stream(attempt, node_id, message)
                                .await;
                        }
                    }
                }
            }
        }
    }

    async fn run_legacy_stream_to_completion(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        provider: &dyn StreamingProviderAdapter,
        input: &AdapterInput,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let mut stream = provider
            .run_streaming(input, CancellationToken::new())
            .await?;
        let mut full_output = String::new();
        while let Some(chunk) = stream.recv().await {
            match chunk {
                StreamChunk::Text(content) => {
                    full_output.push_str(&content);
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingStreamChunk {
                            content,
                            node_id: Some(node_id.to_string()),
                        })
                        .await;
                }
                StreamChunk::Done {
                    full_output: completed_output,
                } => {
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingMessageComplete {
                            node_id: Some(node_id.to_string()),
                        })
                        .await;
                    if !completed_output.trim().is_empty() {
                        return Ok(completed_output);
                    }
                    return Ok(full_output);
                }
                StreamChunk::Error(message) => {
                    return self.fail_provider_stream(attempt, node_id, message).await;
                }
            }
        }

        self.fail_provider_stream_ended(attempt, node_id).await
    }

    async fn fail_provider_stream<T>(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        message: String,
    ) -> Result<T, CodingWorkspaceEngineError> {
        self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Failed,
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            node_id,
            CodingTimelineNodeStatus::Failed,
            Some(message.clone()),
        )
        .await?;
        self.handle_attempt_failed(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .await?;
        Err(CodingWorkspaceEngineError::ProviderStream(message))
    }

    async fn fail_provider_stream_ended<T>(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
    ) -> Result<T, CodingWorkspaceEngineError> {
        self.fail_provider_stream(
            attempt,
            node_id,
            "provider stream ended before completion".to_string(),
        )
        .await
    }

    async fn emit_permission_request(
        &self,
        node_id: &str,
        provider: &ProviderName,
        request: PermissionRequestData,
    ) {
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: ws_event_from_permission_request(node_id, provider, &request),
            })
            .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingPermissionRequest {
                id: request.id,
                tool_name: request.tool_name,
                description: request.description,
                risk_level: ws_permission_risk_level(request.risk_level),
            })
            .await;
    }

    async fn emit_choice_request(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        provider: &ProviderName,
        request: ChoiceRequestData,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let source = request.source.as_str().to_string();
        self.store.create_choice_gate(CreateChoiceGateInput {
            attempt_id: attempt.id.clone(),
            choice_id: request.id.clone(),
            stage,
            node_id: Some(node_id.to_string()),
            role,
            provider: provider.clone(),
            source: source.clone(),
            prompt: request.prompt.clone(),
            options: request
                .options
                .iter()
                .map(|option| CodingChoiceOption {
                    id: option.id.clone(),
                    label: option.label.clone(),
                    description: option.description.clone(),
                })
                .collect(),
            allow_multiple: request.allow_multiple,
            allow_free_text: request.allow_free_text,
        })?;
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        if current.status == CodingAttemptStatus::Running {
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::WaitingForHuman,
            )?;
        }
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: ws_event_from_choice_request(node_id, provider, &request),
            })
            .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingChoiceRequest {
                id: request.id,
                prompt: request.prompt,
                source,
                options: request
                    .options
                    .into_iter()
                    .map(|option| ChoiceOption {
                        id: option.id,
                        label: option.label,
                        description: option.description,
                    })
                    .collect(),
                allow_multiple: request.allow_multiple,
                allow_free_text: request.allow_free_text,
            })
            .await;
        Ok(())
    }

    pub async fn execute_testing(
        &self,
        attempt: &CodingExecutionAttempt,
        specs: &[TestCommandSpec],
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Testing,
        )?;
        let node = self.create_testing_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;
        let artifact_output_root = self.store.attempt_test_output_root(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        );
        let report = run_all_tests(&attempt.id, worktree_path, artifact_output_root, specs).await?;
        self.store.save_testing_report(&report)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::TestingReportUpdate {
                report: Box::new(report.clone()),
            })
            .await;

        let (node_status, summary) = match report.overall_status {
            TestingOverallStatus::Passed => (
                CodingTimelineNodeStatus::Completed,
                Some("测试通过".to_string()),
            ),
            TestingOverallStatus::PassedWithWarnings => (
                CodingTimelineNodeStatus::Completed,
                Some("测试通过但有警告".to_string()),
            ),
            TestingOverallStatus::Failed => (
                CodingTimelineNodeStatus::Failed,
                Some("测试失败".to_string()),
            ),
            TestingOverallStatus::SkippedByUserDecision => (
                CodingTimelineNodeStatus::Completed,
                Some("测试由用户决策跳过".to_string()),
            ),
            TestingOverallStatus::Blocked => (
                CodingTimelineNodeStatus::Blocked,
                Some("测试被阻塞".to_string()),
            ),
        };
        if matches!(
            report.overall_status,
            TestingOverallStatus::Failed | TestingOverallStatus::Blocked
        ) && !testing_report_should_enter_analyst(&report)
        {
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
        }
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
        Ok(report)
    }

    pub async fn create_testing_result_review_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        report: &TestingReport,
    ) -> Result<Option<CodingGateRequired>, CodingWorkspaceEngineError> {
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let open_testing_gate_exists = self
            .store
            .list_open_blocked_gates(&current.project_id, &current.issue_id, &current.id)?
            .into_iter()
            .any(|gate| {
                gate.stage == Some(CodingExecutionStage::Testing)
                    && gate.reason_code.as_deref() != Some(TESTING_RESULT_REVIEW_REASON_CODE)
            });
        if open_testing_gate_exists {
            return Ok(None);
        }

        if current.status != CodingAttemptStatus::Blocked {
            self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Blocked,
            )?;
            self.release_active_lock_if_shared_worktree_clean(
                &current.project_id,
                &current.issue_id,
                &current.id,
                &current.work_item_id,
            )
            .await?;
        }

        let node_id = self
            .store
            .latest_role_run(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingExecutionStage::Testing,
                CodingProviderRole::Tester,
            )?
            .and_then(|run| run.node_id);
        let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
            attempt_id: current.id.clone(),
            stage: CodingExecutionStage::Testing,
            node_id,
            role: Some(CodingProviderRole::Tester),
            title: "确认 Tester 测试结果".to_string(),
            description: testing_result_review_description(report),
            reason_code: Some(TESTING_RESULT_REVIEW_REASON_CODE.to_string()),
            evidence_refs: vec![format!("{}.json", report.id)],
            raw_provider_output_ref: report.raw_provider_output_ref.clone(),
            available_actions: testing_result_review_gate_actions(),
        })?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingGateRequired { gate: gate.clone() })
            .await;
        Ok(Some(gate))
    }

    async fn save_blocked_testing_report_and_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        node: &CodingTimelineNode,
        mut report: TestingReport,
        gate_context: BlockedTestingGateContext<'_>,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let BlockedTestingGateContext {
            reason_code,
            description,
            raw_provider_output_ref,
            role_run,
        } = gate_context;
        if let Some(role_run) = role_run {
            bind_testing_report_role_run(&mut report, role_run);
        }
        self.store.save_testing_report(&report)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::TestingReportUpdate {
                report: Box::new(report.clone()),
            })
            .await;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Blocked,
            Some("测试被阻塞".to_string()),
        )
        .await?;
        if testing_blocked_report_needs_gate(&report, &reason_code) {
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
            let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Testing,
                node_id: Some(node.id.clone()),
                role: Some(CodingProviderRole::Tester),
                title: "Testing blocked".to_string(),
                description,
                reason_code: Some(reason_code.clone()),
                evidence_refs: vec![format!("{}.json", report.id)],
                raw_provider_output_ref,
                available_actions: testing_blocked_gate_actions(),
            })?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingGateRequired { gate })
                .await;
        }
        if let Some(role_run) = role_run {
            self.store.update_role_run_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &role_run.id,
                testing_role_run_status(&report),
                Some(reason_code.clone()),
            )?;
        }
        Ok(report)
    }

    async fn block_provider_driven_testing(
        &self,
        attempt: &CodingExecutionAttempt,
        node: &CodingTimelineNode,
        gate_context: BlockedTestingGateContext<'_>,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let raw_provider_output_ref = gate_context.raw_provider_output_ref.clone();
        let reason_code = gate_context.reason_code.clone();
        let description = gate_context.description.clone();
        let report_id = next_sequential_id(
            "testing_report",
            self.store
                .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                .len(),
        );
        let mut report = build_testing_report(&attempt.id, Vec::new(), "", Some(description));
        report.id = report_id;
        report.overall_status = TestingOverallStatus::Blocked;
        report.raw_provider_output_ref = raw_provider_output_ref.clone();
        report.context_warnings.push(reason_code.to_string());
        self.save_blocked_testing_report_and_gate(attempt, node, report, gate_context)
            .await
    }

    async fn block_invalid_test_plan(
        &self,
        attempt: &CodingExecutionAttempt,
        node: &CodingTimelineNode,
        provider_output: &str,
        error: String,
        gate_context: BlockedTestingGateContext<'_>,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let raw_provider_output_ref = gate_context.raw_provider_output_ref.clone();
        let reason_code = gate_context.reason_code.clone();
        let report_id = next_sequential_id(
            "testing_report",
            self.store
                .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                .len(),
        );
        let mut report = build_testing_report(
            &attempt.id,
            Vec::new(),
            provider_output,
            Some(format!("TestPlan parse failed: {error}")),
        );
        report.id = report_id;
        report.raw_provider_output_ref = raw_provider_output_ref;
        report
            .context_warnings
            .push(format!("{reason_code}:{error}"));
        self.save_blocked_testing_report_and_gate(attempt, node, report, gate_context)
            .await
    }

    pub async fn execute_testing_with_provider(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        context: &CodingExecutionContext,
        specs: &[TestCommandSpec],
        options: TesterAgentOptions,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let (_command_tx, mut command_rx) = mpsc::channel(1);
        self.execute_testing_with_provider_commands(
            attempt,
            provider,
            context,
            specs,
            options,
            &mut command_rx,
        )
        .await
    }

    pub async fn execute_testing_with_provider_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        _context: &CodingExecutionContext,
        _specs: &[TestCommandSpec],
        options: TesterAgentOptions,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Testing,
        )?;
        let node = self.create_testing_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;
        let role_run = match self.store.latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
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
                CodingExecutionStage::Testing,
                CodingProviderRole::Tester,
                CodingRoleRunTrigger::Initial,
                Some(node.id.clone()),
            )?,
        };

        if !provider.supports_provider_driven_testing() {
            return self
                .block_provider_driven_testing(
                    &attempt,
                    &node,
                    BlockedTestingGateContext {
                        reason_code: "provider_driven_testing_not_supported".to_string(),
                        description: "Tester provider does not support provider-driven testing"
                            .to_string(),
                        raw_provider_output_ref: None,
                        role_run: Some(&role_run),
                    },
                )
                .await;
        }

        let tester_provider = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .tester;
        let evaluation_context = build_evaluation_context_pack(
            self.store.paths(),
            &attempt,
            EvaluationContextRole::Tester,
        )?;
        let evaluation_context_json =
            serde_json::to_string_pretty(&evaluation_context).map_err(|error| {
                CodingWorkspaceEngineError::ProviderStream(format!(
                    "serialize_evaluation_context_failed: {error}"
                ))
            })?;
        let retry_diagnostic = self.retry_diagnostic_for_previous_run(&attempt, &role_run)?;
        let plan_prompt = build_tester_plan_prompt(
            &attempt,
            &evaluation_context_json,
            retry_diagnostic.as_deref(),
        );
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &tester_provider,
                    plan_prompt.clone(),
                    "plan_tests",
                ),
            })
            .await;
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Tester,
            &tester_provider,
        );
        let mut chat_entry_sequence = 1usize;
        let plan_adapter_input = AdapterInput {
            provider_type: provider_type_for_name(&tester_provider),
            role: AdapterRole::Reviewer,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt: plan_prompt,
            context_files: Vec::new(),
            output_schema: "coding_workspace_test_plan_json".to_string(),
            timeout: options.timeout.as_secs().max(1),
            max_retries: 0,
        };
        let plan_input = StreamingProviderInput {
            provider_type: plan_adapter_input.provider_type.clone(),
            role: plan_adapter_input.role.clone(),
            prompt: plan_adapter_input.prompt.clone(),
            working_dir: worktree_path.clone(),
            workspace_session_id: Some(attempt.id.clone()),
            resume_provider_session_id,
            permission_mode: role_permission_mode_for_attempt(
                &self.store,
                &attempt,
                CodingProviderRole::Tester,
            )?,
            env_vars: BTreeMap::new(),
            timeout_secs: plan_adapter_input.timeout,
        };
        let plan_output = match self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                role_run: Some(&role_run),
                provider,
                legacy_input: &plan_adapter_input,
                input: plan_input,
                provider_name: &tester_provider,
                provider_role: CodingProviderRole::Tester,
                command_rx,
                allow_legacy_stream_fallback: false,
                timeout: Some(options.timeout),
                timeout_reason_code: Some("plan_tests_timeout"),
            })
            .await
        {
            Ok(output) => output,
            Err(error) => {
                let reason_code = if error.to_string().contains("plan_tests_timeout") {
                    "plan_tests_timeout"
                } else {
                    "provider_start_failed"
                };
                return self
                    .block_provider_driven_testing(
                        &attempt,
                        &node,
                        BlockedTestingGateContext {
                            reason_code: reason_code.to_string(),
                            description: format!(
                                "Tester provider failed during plan_tests: {error}"
                            ),
                            raw_provider_output_ref: None,
                            role_run: Some(&role_run),
                        },
                    )
                    .await;
            }
        };
        let plan_raw_ref = self.store.save_provider_raw_output(
            &attempt.id,
            CodingExecutionStage::Testing,
            "plan_tests",
            &plan_output,
        )?;
        let plan_id = next_sequential_id(
            "test_plan",
            self.store
                .list_test_plans(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                .len(),
        );
        let plan = match parse_test_plan_payload(
            &attempt.id,
            &plan_id,
            &plan_output,
            Some(plan_raw_ref.clone()),
        ) {
            Ok(mut plan) => {
                bind_test_plan_role_run(&mut plan, &role_run);
                self.store.save_test_plan(&plan)?;
                plan
            }
            Err(first_error) => {
                let repair_prompt =
                    build_tester_plan_repair_prompt(&plan_output, &first_error.to_string());
                let repair_adapter_input = AdapterInput {
                    provider_type: provider_type_for_name(&tester_provider),
                    role: AdapterRole::Reviewer,
                    worktree_path: Some(worktree_path.to_string_lossy().to_string()),
                    prompt: repair_prompt,
                    context_files: Vec::new(),
                    output_schema: "coding_workspace_test_plan_json".to_string(),
                    timeout: options.timeout.as_secs().max(1),
                    max_retries: 0,
                };
                let repair_input = StreamingProviderInput {
                    provider_type: repair_adapter_input.provider_type.clone(),
                    role: repair_adapter_input.role.clone(),
                    prompt: repair_adapter_input.prompt.clone(),
                    working_dir: worktree_path.clone(),
                    workspace_session_id: Some(attempt.id.clone()),
                    resume_provider_session_id: None,
                    permission_mode: role_permission_mode_for_attempt(
                        &self.store,
                        &attempt,
                        CodingProviderRole::Tester,
                    )?,
                    env_vars: BTreeMap::new(),
                    timeout_secs: repair_adapter_input.timeout,
                };
                let repair_output = match self
                    .run_provider_stream_to_completion(CodingProviderStreamRun {
                        attempt: &attempt,
                        node_id: &node.id,
                        role_run: Some(&role_run),
                        provider,
                        legacy_input: &repair_adapter_input,
                        input: repair_input,
                        provider_name: &tester_provider,
                        provider_role: CodingProviderRole::Tester,
                        command_rx,
                        allow_legacy_stream_fallback: false,
                        timeout: Some(options.timeout),
                        timeout_reason_code: Some("plan_tests_timeout"),
                    })
                    .await
                {
                    Ok(output) => output,
                    Err(error) => {
                        let reason_code = if error.to_string().contains("plan_tests_timeout") {
                            "plan_tests_timeout"
                        } else {
                            "provider_start_failed"
                        };
                        return self
                            .block_provider_driven_testing(
                                &attempt,
                                &node,
                                BlockedTestingGateContext {
                                    reason_code: reason_code.to_string(),
                                    description: format!(
                                        "Tester provider failed during plan_tests_repair: {error}"
                                    ),
                                    raw_provider_output_ref: None,
                                    role_run: Some(&role_run),
                                },
                            )
                            .await;
                    }
                };
                let repair_raw_ref = self.store.save_provider_raw_output(
                    &attempt.id,
                    CodingExecutionStage::Testing,
                    "plan_tests_repair",
                    &repair_output,
                )?;
                match parse_test_plan_payload(
                    &attempt.id,
                    &plan_id,
                    &repair_output,
                    Some(repair_raw_ref.clone()),
                ) {
                    Ok(mut plan) => {
                        bind_test_plan_role_run(&mut plan, &role_run);
                        self.store.save_test_plan(&plan)?;
                        plan
                    }
                    Err(repair_error) => {
                        return self
                            .block_invalid_test_plan(
                                &attempt,
                                &node,
                                &repair_output,
                                repair_error.to_string(),
                                BlockedTestingGateContext {
                                    reason_code: "test_plan_repair_failed".to_string(),
                                    description: "TestPlan parse failed".to_string(),
                                    raw_provider_output_ref: Some(repair_raw_ref),
                                    role_run: Some(&role_run),
                                },
                            )
                            .await;
                    }
                }
            }
        };
        let entry = tester_chat_entry(
            &attempt,
            &node.id,
            &mut chat_entry_sequence,
            CodingEntryType::AssistantMessage,
            Some(format_test_plan_chat_summary(&plan)),
            Some(serde_json::json!({
                "phase": "test_plan",
                "test_plan_id": plan.id.clone(),
                "role_run_id": role_run.id.clone(),
                "run_no": role_run.run_no,
                "raw_provider_output_ref": plan.raw_provider_output_ref.clone()
            })),
        );
        self.save_and_emit_chat_entry(entry).await;
        let prompt = build_tester_execute_plan_prompt(&attempt, &plan, &evaluation_context_json);
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &tester_provider,
                    prompt.clone(),
                    "execute_test_plan",
                ),
            })
            .await;
        self.record_role_run_event(
            &attempt,
            Some(&role_run),
            CodingRoleRunEventType::ProviderPrompt,
            json!({
                "provider": tester_provider.clone(),
                "role": format!("{:?}", CodingProviderRole::Tester),
                "output_schema": "coding_workspace_execute_test_plan_json",
                "prompt": prompt.clone()
            }),
        );
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Tester,
            &tester_provider,
        );
        let input = StreamingProviderInput {
            provider_type: provider_type_for_name(&tester_provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir: worktree_path.clone(),
            workspace_session_id: Some(attempt.id.clone()),
            resume_provider_session_id,
            permission_mode: role_permission_mode_for_attempt(
                &self.store,
                &attempt,
                CodingProviderRole::Tester,
            )?,
            env_vars: BTreeMap::new(),
            timeout_secs: options.timeout.as_secs().max(1),
        };
        let cancel = CancellationToken::new();
        let start_result = tokio::select! {
            result = provider.start(input, cancel.clone()) => result,
            _ = tokio::time::sleep(options.timeout) => {
                cancel.cancel();
                self.record_role_run_event(
                    &attempt,
                    Some(&role_run),
                    CodingRoleRunEventType::Timeout,
                    json!({
                        "phase": "execute_test_plan_start",
                        "reason_code": "execute_test_plan_timeout"
                    }),
                );
                let report_id = next_sequential_id(
                    "testing_report",
                    self.store
                        .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                        .len(),
                );
                let mut report = build_plan_based_testing_report(
                    &report_id,
                    &attempt.id,
                    &plan,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                );
                report.overall_status = TestingOverallStatus::Blocked;
                report
                    .context_warnings
                    .push("execute_test_plan_timeout".to_string());
                return self
                    .save_blocked_testing_report_and_gate(
                        &attempt,
                        &node,
                        report,
                        BlockedTestingGateContext {
                            reason_code: "execute_test_plan_timeout".to_string(),
                            description: "Tester provider timed out starting execute_test_plan"
                                .to_string(),
                            raw_provider_output_ref: None,
                            role_run: Some(&role_run),
                        },
                    )
                    .await;
            }
        };
        let mut session = match start_result {
            Ok(session) => {
                self.record_role_run_event(
                    &attempt,
                    Some(&role_run),
                    CodingRoleRunEventType::ProviderStart,
                    json!({
                        "provider": tester_provider.clone(),
                        "role": format!("{:?}", CodingProviderRole::Tester),
                        "phase": "execute_test_plan"
                    }),
                );
                session
            }
            Err(error) => {
                self.record_role_run_event(
                    &attempt,
                    Some(&role_run),
                    CodingRoleRunEventType::ProviderFailed,
                    json!({
                        "phase": "execute_test_plan",
                        "message": error.details.clone()
                    }),
                );
                let report_id = next_sequential_id(
                    "testing_report",
                    self.store
                        .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                        .len(),
                );
                let mut report = build_plan_based_testing_report(
                    &report_id,
                    &attempt.id,
                    &plan,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                );
                report.overall_status = TestingOverallStatus::Blocked;
                report
                    .context_warnings
                    .push(format!("provider_start_failed:{error}"));
                return self
                    .save_blocked_testing_report_and_gate(
                        &attempt,
                        &node,
                        report,
                        BlockedTestingGateContext {
                            reason_code: "provider_start_failed".to_string(),
                            description: "Tester provider failed during execute_test_plan"
                                .to_string(),
                            raw_provider_output_ref: None,
                            role_run: Some(&role_run),
                        },
                    )
                    .await;
            }
        };
        let timeout = tokio::time::sleep(options.timeout);
        tokio::pin!(timeout);
        let mut full_output = String::new();
        let mut step_results = Vec::new();
        let mut unplanned_commands = Vec::new();
        let mut unplanned_evidence = Vec::new();
        let mut context_warnings = Vec::new();
        let mut consecutive_failures = 0usize;
        let mut blocked_summary = None;
        let mut blocked_reason_code = None;
        let mut commands_open = true;
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();
        let mut open_choice_ids = Vec::<String>::new();

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    cancel.cancel();
                    self.record_role_run_event(
                        &attempt,
                        Some(&role_run),
                        CodingRoleRunEventType::Timeout,
                        json!({
                            "phase": "execute_test_plan",
                            "reason_code": "provider_stream_timeout"
                        }),
                    );
                    blocked_summary = Some("Tester Agent Loop 超时".to_string());
                    break;
                }
                command = command_rx.recv(), if commands_open => {
                    let Some(command) = command else {
                        commands_open = false;
                        continue;
                    };
                    match command {
                        CodingRunnerCommand::AbortAttempt => {
                            let _ = session.commands.send(ProviderCommand::Abort).await;
                            cancel.cancel();
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_provider_status(
                                        &node.id,
                                        &tester_provider,
                                        ProviderStatus::Aborted,
                                    ),
                                })
                                .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::Aborted,
                                json!({
                                    "reason": "abort_attempt",
                                    "phase": "execute_test_plan"
                                }),
                            );
                            return Err(CodingWorkspaceEngineError::Aborted);
                        }
                        CodingRunnerCommand::ChoiceResponse {
                            id,
                            selected_option_ids,
                            free_text,
                        } => {
                            if !open_choice_ids.iter().any(|choice_id| choice_id == &id) {
                                let _ = self
                                    .event_tx
                                    .send(CodingWsOutMessage::CodingProtocolError {
                                        code: "coding_choice_gate_not_found".to_string(),
                                        message: format!(
                                            "ChoiceResponse id={id} not found in open coding choice gates"
                                        ),
                                    })
                                    .await;
                                continue;
                            }
                            if session
                                .commands
                                .send(ProviderCommand::ChoiceResponse {
                                    id: id.clone(),
                                    selected_option_ids: selected_option_ids.clone(),
                                    free_text: free_text.clone(),
                                })
                                .await
                                .is_ok()
                            {
                                let ack_selected_option_ids = selected_option_ids.clone();
                                let ack_free_text = free_text.clone();
                                let _ = self.store.resolve_choice_gate(
                                    &attempt.project_id,
                                    &attempt.issue_id,
                                    &attempt.id,
                                    &id,
                                    selected_option_ids,
                                    free_text,
                                )?;
                                open_choice_ids.retain(|choice_id| choice_id != &id);
                                let current = self.store.get_attempt(
                                    &attempt.project_id,
                                    &attempt.issue_id,
                                    &attempt.id,
                                )?;
                                if current.status == CodingAttemptStatus::WaitingForHuman {
                                    self.store.update_attempt_status(
                                        &attempt.project_id,
                                        &attempt.issue_id,
                                        &attempt.id,
                                        CodingAttemptStatus::Running,
                                    )?;
                                }
                                let _ = self
                                    .event_tx
                                    .send(CodingWsOutMessage::CodingChoiceResponseAck {
                                        id,
                                        selected_option_ids: ack_selected_option_ids,
                                        free_text: ack_free_text,
                                    })
                                    .await;
                            } else {
                                commands_open = false;
                            }
                        }
                        command => {
                            if !forward_runner_command_to_provider(command, &session.commands).await {
                                commands_open = false;
                            }
                        }
                    }
                }
                event = session.events.recv() => {
                    let Some(event) = event else {
                        if !open_choice_ids.is_empty() {
                            return Err(self.unresolved_provider_choice_error(
                                &attempt,
                                Some(&role_run),
                                "execute_test_plan_stream_closed",
                                &open_choice_ids,
                            ));
                        }
                        blocked_summary = Some("Tester Provider stream ended before completion".to_string());
                        break;
                    };
                    match event {
                        ProviderEvent::TextDelta { content } => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_text_delta",
                                    &open_choice_ids,
                                ));
                            }
                            let content_for_event = content.clone();
                            full_output.push_str(&content);
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingStreamChunk {
                                    content,
                                    node_id: Some(node.id.clone()),
                                })
                                .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::TextDelta,
                                json!({
                                    "content": content_for_event,
                                    "phase": "execute_test_plan"
                                }),
                            );
                        }
                        ProviderEvent::ToolCall(call) => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_tool_call",
                                    &open_choice_ids,
                                ));
                            }
                            let call_for_event = call.clone();
                            tool_call_titles.insert(call.id.clone(), call.tool_name.clone());
                            if let Some(command) = extract_tool_command(&call.input) {
                                tool_call_commands.insert(call.id.clone(), command);
                            }
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_tool_call(&node.id, &tester_provider, call.clone()),
                                })
                                .await;
                            let entry = tester_chat_entry(
                                &attempt,
                                &node.id,
                                &mut chat_entry_sequence,
                                CodingEntryType::ToolCall {
                                    tool_name: call.tool_name.clone(),
                                    input: call.input.clone(),
                                },
                                None,
                                Some(serde_json::json!({
                                    "tool_use_id": call.id.clone(),
                                    "role_run_id": role_run.id.clone(),
                                    "run_no": role_run.run_no
                                })),
                            );
                            self.save_and_emit_chat_entry(entry).await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ToolCall,
                                json!({
                                    "id": call_for_event.id,
                                    "tool_name": call_for_event.tool_name,
                                    "input": call_for_event.input,
                                    "phase": "execute_test_plan"
                                }),
                            );

                            if let Some(reason_code) =
                                high_risk_test_step_block_reason(&plan, &call)
                            {
                                cancel.cancel();
                                blocked_reason_code = Some(reason_code.to_string());
                                blocked_summary = Some(
                                    "High risk TestPlan step requires permission".to_string(),
                                );
                                break;
                            }

                            let artifact_output_root = self.store.attempt_test_output_root(
                                &attempt.project_id,
                                &attempt.issue_id,
                                &attempt.id,
                            );
                            let outcome =
                                execute_tester_tool_call(&call, worktree_path, artifact_output_root)
                                    .await?;
                            let command_result = outcome.command.clone();
                            let result = outcome.result;
                            record_tester_step_result(
                                &plan,
                                &call,
                                command_result,
                                &result,
                                TesterStepResultOutputs {
                                    step_results: &mut step_results,
                                    unplanned_commands: &mut unplanned_commands,
                                    unplanned_evidence: &mut unplanned_evidence,
                                    context_warnings: &mut context_warnings,
                                },
                            );
                            let is_error = result.is_error;
                            let result_for_event = result.clone();
                            let _ = session
                                .commands
                                .send(ProviderCommand::ToolResult(result.clone()))
                                .await;
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_tool_result(
                                        &node.id,
                                        &tester_provider,
                                        &call.tool_name,
                                        extract_tool_command(&call.input),
                                        result.clone(),
                                    ),
                                })
                                .await;
                            self.emit_tester_tool_result_entry(
                                &attempt,
                                &node.id,
                                &mut chat_entry_sequence,
                                Some(&role_run),
                                result,
                            )
                            .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ToolResult,
                                json!({
                                    "tool_use_id": result_for_event.tool_use_id,
                                    "output": result_for_event.output,
                                    "is_error": result_for_event.is_error,
                                    "phase": "execute_test_plan"
                                }),
                            );

                            if is_error {
                                consecutive_failures += 1;
                            } else {
                                consecutive_failures = 0;
                            }
                            if consecutive_failures >= options.failure_limit {
                                cancel.cancel();
                                blocked_summary = Some(format!(
                                    "Tester Agent Loop 连续 {} 次 tool_use 失败",
                                    options.failure_limit
                                ));
                                break;
                            }
                        }
                        ProviderEvent::ToolResult(result) => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_tool_result",
                                    &open_choice_ids,
                                ));
                            }
                            let result_for_event = result.clone();
                            let title = tool_call_titles
                                .get(&result.tool_use_id)
                                .cloned()
                                .unwrap_or_else(|| "Tool result".to_string());
                            let command = tool_call_commands.get(&result.tool_use_id).cloned();
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_tool_result(
                                        &node.id,
                                        &tester_provider,
                                        &title,
                                        command,
                                        result.clone(),
                                    ),
                                })
                                .await;
                            self.emit_tester_tool_result_entry(
                                &attempt,
                                &node.id,
                                &mut chat_entry_sequence,
                                Some(&role_run),
                                result,
                            )
                            .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ToolResult,
                                json!({
                                    "tool_use_id": result_for_event.tool_use_id,
                                    "output": result_for_event.output,
                                    "is_error": result_for_event.is_error,
                                    "phase": "execute_test_plan"
                                }),
                            );
                        }
                        ProviderEvent::Execution(event) => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_execution",
                                    &open_choice_ids,
                                ));
                            }
                            let event_for_record = event.clone();
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_provider_execution(
                                        event,
                                        &node.id,
                                        &tester_provider,
                                    ),
                                })
                                .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ExecutionEvent,
                                json!({
                                    "event_id": event_for_record.event_id,
                                    "kind": format!("{:?}", event_for_record.kind),
                                    "status": format!("{:?}", event_for_record.status),
                                    "title": event_for_record.title,
                                    "detail": event_for_record.detail,
                                    "command": event_for_record.command,
                                    "cwd": event_for_record.cwd,
                                    "output": event_for_record.output,
                                    "exit_code": event_for_record.exit_code,
                                    "phase": "execute_test_plan"
                                }),
                            );
                        }
                        ProviderEvent::Completed {
                            full_output: completed_output,
                            provider_session_id,
                        } => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_completed",
                                    &open_choice_ids,
                                ));
                            }
                            let provider_session_id_for_event = provider_session_id.clone();
                            let output_bytes = completed_output.len();
                            self.record_attempt_provider_session(
                                &attempt,
                                &CodingProviderRole::Tester,
                                tester_provider.clone(),
                                provider_session_id,
                                &node.id,
                            )?;
                            if !completed_output.trim().is_empty() {
                                full_output = completed_output;
                            }
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingMessageComplete {
                                    node_id: Some(node.id.clone()),
                                })
                                .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::MessageComplete,
                                json!({
                                    "provider_session_id": provider_session_id_for_event,
                                    "output_bytes": output_bytes,
                                    "phase": "execute_test_plan"
                                }),
                            );
                            break;
                        }
                        ProviderEvent::Failed { message } => {
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ProviderFailed,
                                json!({
                                    "phase": "execute_test_plan",
                                    "message": message.clone()
                                }),
                            );
                            blocked_summary = Some(message);
                            break;
                        }
                        ProviderEvent::ProtocolError {
                            code,
                            message,
                            context,
                        } => {
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ProviderFailed,
                                json!({
                                    "phase": "execute_test_plan",
                                    "code": code,
                                    "message": message.clone(),
                                    "context": context
                                }),
                            );
                            blocked_summary = Some(message);
                            break;
                        }
                        ProviderEvent::PermissionTimeout { permission_id } => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_permission_timeout",
                                    &open_choice_ids,
                                ));
                            }
                            let message = format!("Permission request {permission_id} timed out");
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::Timeout,
                                json!({
                                    "phase": "execute_test_plan",
                                    "reason": "permission_timeout",
                                    "permission_id": permission_id,
                                    "message": message.clone()
                                }),
                            );
                            blocked_summary = Some(message);
                            break;
                        }
                        ProviderEvent::PermissionRequest(request) => {
                            if !open_choice_ids.is_empty() {
                                return Err(self.unresolved_provider_choice_error(
                                    &attempt,
                                    Some(&role_run),
                                    "execute_test_plan_permission_request",
                                    &open_choice_ids,
                                ));
                            }
                            let request_for_event = request.clone();
                            self.emit_permission_request(&node.id, &tester_provider, request).await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::PermissionRequest,
                                json!({
                                    "id": request_for_event.id,
                                    "tool_name": request_for_event.tool_name,
                                    "description": request_for_event.description,
                                    "risk_level": format!("{:?}", request_for_event.risk_level),
                                    "phase": "execute_test_plan"
                                }),
                            );
                        }
                        ProviderEvent::ChoiceRequest(request) => {
                            let request_for_event = request.clone();
                            self.emit_choice_request(
                                &attempt,
                                &node.id,
                                CodingExecutionStage::Testing,
                                CodingProviderRole::Tester,
                                &tester_provider,
                                request,
                            )
                            .await?;
                            open_choice_ids.push(request_for_event.id.clone());
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::ChoiceRequest,
                                json!({
                                    "id": request_for_event.id,
                                    "prompt": request_for_event.prompt,
                                    "allow_multiple": request_for_event.allow_multiple,
                                    "allow_free_text": request_for_event.allow_free_text,
                                    "source": request_for_event.source.as_str(),
                                    "phase": "execute_test_plan"
                                }),
                            );
                        }
                        ProviderEvent::StatusChanged(status) => {
                            let status_for_event = status.clone();
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingExecutionEvent {
                                    event: ws_event_from_provider_status(
                                        &node.id,
                                        &tester_provider,
                                        status,
                                    ),
                                })
                                .await;
                            self.record_role_run_event(
                                &attempt,
                                Some(&role_run),
                                CodingRoleRunEventType::StatusChanged,
                                json!({
                                    "status": format!("{status_for_event:?}"),
                                    "phase": "execute_test_plan"
                                }),
                            );
                        }
                    }
                }
            }
        }

        let execute_raw_ref = self.store.save_provider_raw_output(
            &attempt.id,
            CodingExecutionStage::Testing,
            "execute_test_plan",
            &full_output,
        )?;
        for provider_step_result in parse_testing_step_results_from_provider_output(&full_output) {
            if !step_results
                .iter()
                .any(|existing| existing.step_id == provider_step_result.step_id)
            {
                step_results.push(provider_step_result);
            }
        }
        let mut report_plan = plan.clone();
        for warning in context_warnings {
            if !report_plan
                .context_warnings
                .iter()
                .any(|existing| existing == &warning)
            {
                report_plan.context_warnings.push(warning);
            }
        }
        let report_id = next_sequential_id(
            "testing_report",
            self.store
                .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                .len(),
        );
        let mut report_raw_ref = execute_raw_ref.clone();
        let provider_claim = serde_json::from_str(&full_output).ok();
        let mut report = build_plan_based_testing_report(
            &report_id,
            &attempt.id,
            &report_plan,
            step_results.clone(),
            unplanned_commands.clone(),
            provider_claim,
            Some(report_raw_ref.clone()),
        );
        report.unplanned_evidence = unplanned_evidence.clone();
        if !report.missing_required_steps.is_empty() && blocked_summary.is_none() {
            let repair_prompt =
                build_tester_execute_repair_prompt(&full_output, &report.missing_required_steps);
            let repair_adapter_input = AdapterInput {
                provider_type: provider_type_for_name(&tester_provider),
                role: AdapterRole::Reviewer,
                worktree_path: Some(worktree_path.to_string_lossy().to_string()),
                prompt: repair_prompt,
                context_files: Vec::new(),
                output_schema: "coding_workspace_test_execution_json".to_string(),
                timeout: options.timeout.as_secs().max(1),
                max_retries: 0,
            };
            let repair_input = StreamingProviderInput {
                provider_type: repair_adapter_input.provider_type.clone(),
                role: repair_adapter_input.role.clone(),
                prompt: repair_adapter_input.prompt.clone(),
                working_dir: worktree_path.clone(),
                workspace_session_id: Some(attempt.id.clone()),
                resume_provider_session_id: None,
                permission_mode: role_permission_mode_for_attempt(
                    &self.store,
                    &attempt,
                    CodingProviderRole::Tester,
                )?,
                env_vars: BTreeMap::new(),
                timeout_secs: repair_adapter_input.timeout,
            };
            let repair_output = self
                .run_provider_stream_to_completion(CodingProviderStreamRun {
                    attempt: &attempt,
                    node_id: &node.id,
                    role_run: Some(&role_run),
                    provider,
                    legacy_input: &repair_adapter_input,
                    input: repair_input,
                    provider_name: &tester_provider,
                    provider_role: CodingProviderRole::Tester,
                    command_rx,
                    allow_legacy_stream_fallback: false,
                    timeout: None,
                    timeout_reason_code: None,
                })
                .await?;
            let repair_raw_ref = self.store.save_provider_raw_output(
                &attempt.id,
                CodingExecutionStage::Testing,
                "execute_test_plan_repair",
                &repair_output,
            )?;
            report_raw_ref = repair_raw_ref;
            for provider_step_result in
                parse_testing_step_results_from_provider_output(&repair_output)
            {
                if !step_results
                    .iter()
                    .any(|existing| existing.step_id == provider_step_result.step_id)
                {
                    step_results.push(provider_step_result);
                }
            }
            let repair_provider_claim = serde_json::from_str(&repair_output).ok();
            report = build_plan_based_testing_report(
                &report_id,
                &attempt.id,
                &report_plan,
                step_results.clone(),
                unplanned_commands.clone(),
                repair_provider_claim,
                Some(report_raw_ref.clone()),
            );
            report.unplanned_evidence = unplanned_evidence.clone();
        }
        if let Some(summary) = blocked_summary {
            report.overall_status = TestingOverallStatus::Blocked;
            report.context_warnings.push(summary);
        }
        bind_testing_report_role_run(&mut report, &role_run);
        self.store.save_testing_report(&report)?;
        let entry = tester_chat_entry(
            &attempt,
            &node.id,
            &mut chat_entry_sequence,
            CodingEntryType::AssistantMessage,
            Some(format_testing_report_chat_summary(&report)),
            Some(serde_json::json!({
                "phase": "testing_result",
                "testing_report_id": report.id.clone(),
                "role_run_id": role_run.id.clone(),
                "run_no": role_run.run_no,
                "raw_provider_output_ref": report.raw_provider_output_ref.clone()
            })),
        );
        self.save_and_emit_chat_entry(entry).await;
        self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            testing_role_run_status(&report),
            derive_testing_role_run_reason(&report),
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::TestingReportUpdate {
                report: Box::new(report.clone()),
            })
            .await;

        let (node_status, summary) = match report.overall_status {
            TestingOverallStatus::Passed => (
                CodingTimelineNodeStatus::Completed,
                Some("测试通过".to_string()),
            ),
            TestingOverallStatus::PassedWithWarnings => (
                CodingTimelineNodeStatus::Completed,
                Some("测试通过但有警告".to_string()),
            ),
            TestingOverallStatus::Failed => (
                CodingTimelineNodeStatus::Failed,
                Some("测试失败".to_string()),
            ),
            TestingOverallStatus::SkippedByUserDecision => (
                CodingTimelineNodeStatus::Completed,
                Some("测试由用户决策跳过".to_string()),
            ),
            TestingOverallStatus::Blocked => (
                CodingTimelineNodeStatus::Blocked,
                Some("测试被阻塞".to_string()),
            ),
        };
        if matches!(
            report.overall_status,
            TestingOverallStatus::Failed | TestingOverallStatus::Blocked
        ) && !testing_report_should_enter_analyst(&report)
        {
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
        }
        if report.overall_status == TestingOverallStatus::Blocked
            && !testing_report_should_enter_analyst(&report)
        {
            let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Testing,
                node_id: Some(node.id.clone()),
                role: Some(CodingProviderRole::Tester),
                title: "Testing blocked".to_string(),
                description: "Required testing steps are missing or blocked".to_string(),
                reason_code: Some(derive_testing_blocked_reason_code(
                    blocked_reason_code,
                    &report,
                )),
                evidence_refs: vec![format!("{}.json", report.id)],
                raw_provider_output_ref: Some(report_raw_ref),
                available_actions: testing_blocked_gate_actions(),
            })?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingGateRequired { gate })
                .await;
        }
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            node_status,
            summary,
        )
        .await?;
        Ok(report)
    }

    pub async fn execute_code_review(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
    ) -> Result<CodeReviewReport, CodingWorkspaceEngineError> {
        let (_command_tx, mut command_rx) = mpsc::channel(1);
        self.execute_code_review_with_commands(attempt, provider, &mut command_rx)
            .await
    }

    pub async fn execute_code_review_with_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CodeReviewReport, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )?;
        let node = self.create_code_review_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let role_run = match self.store.latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
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
                CodingExecutionStage::CodeReview,
                CodingProviderRole::CodeReviewer,
                CodingRoleRunTrigger::Initial,
                Some(node.id.clone()),
            )?,
        };

        let reviewer = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .code_reviewer;
        let retry_diagnostic = self.retry_diagnostic_for_previous_run(&attempt, &role_run)?;
        let prompt = self
            .build_code_review_prompt(&attempt, worktree_path, retry_diagnostic.as_deref())
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
            output_schema: "coding_workspace_code_review_json".to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::CodeReviewer,
            &reviewer,
        );
        let mut provider_input = streaming_input_from_adapter(&input, worktree_path.clone());
        provider_input.workspace_session_id = Some(attempt.id.clone());
        provider_input.resume_provider_session_id = resume_provider_session_id;
        provider_input.permission_mode = role_permission_mode_for_attempt(
            &self.store,
            &attempt,
            CodingProviderRole::CodeReviewer,
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
                provider_role: CodingProviderRole::CodeReviewer,
                command_rx,
                allow_legacy_stream_fallback: true,
                timeout: None,
                timeout_reason_code: None,
            })
            .await?;
        let raw_provider_output_ref = self.store.save_provider_raw_output(
            &attempt.id,
            CodingExecutionStage::CodeReview,
            "code_review",
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
        let report = self.build_code_review_report(
            &attempt,
            &full_output,
            Some(raw_provider_output_ref.clone()),
            &role_run,
        )?;
        self.store.save_code_review_report(&report)?;
        self.emit_code_review_chat_entry(&attempt, &node.id, &report)
            .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodeReviewComplete {
                report: Box::new(report.clone()),
            })
            .await;
        let (node_status, summary, role_run_status) = match report.verdict {
            ReviewVerdict::Approve => (
                CodingTimelineNodeStatus::Completed,
                Some("code review 通过".to_string()),
                CodingRoleRunStatus::Completed,
            ),
            ReviewVerdict::RequestChanges => (
                CodingTimelineNodeStatus::Failed,
                Some("code review 要求修改".to_string()),
                CodingRoleRunStatus::Completed,
            ),
            ReviewVerdict::Blocked => (
                CodingTimelineNodeStatus::Blocked,
                Some("code review 被阻塞".to_string()),
                CodingRoleRunStatus::Blocked,
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
            None,
        )?;
        Ok(report)
    }

    pub async fn execute_rework(
        &self,
        attempt: &CodingExecutionAttempt,
        evidence: &str,
        provider: &dyn StreamingProviderAdapter,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let (_command_tx, mut command_rx) = mpsc::channel(1);
        self.execute_rework_with_commands(attempt, evidence, provider, &mut command_rx)
            .await
    }

    pub async fn execute_rework_with_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        evidence: &str,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let source_stage = current.stage.clone();
        let rework_round = current.rework_count + 1;
        if current.status != CodingAttemptStatus::Running {
            self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Running,
            )?;
        }
        let attempt = self.store.update_attempt_stage(
            &current.project_id,
            &current.issue_id,
            &current.id,
            CodingExecutionStage::Rework,
        )?;
        let node = self.create_rework_timeline_node(&attempt, rework_round)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let role_run = match self.store.latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Rework,
            CodingProviderRole::Analyst,
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
                CodingExecutionStage::Rework,
                CodingProviderRole::Analyst,
                CodingRoleRunTrigger::Initial,
                Some(node.id.clone()),
            )?,
        };
        let evidence_ref = self.store.save_analyst_evidence(&attempt.id, evidence)?;
        self.store.update_role_run_refs(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            Vec::new(),
            vec![evidence_ref.clone()],
        )?;

        let notes = self.store.list_unconsumed_context_notes(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let note_ids = notes.iter().map(|note| note.id.clone()).collect::<Vec<_>>();
        let context_note_input =
            format_rework_context_notes(&notes, REWORK_CONTEXT_NOTE_CHAR_LIMIT);
        let evaluation_context_json =
            self.evaluation_context_json_for_role(&attempt, EvaluationContextRole::Analyst)?;
        let analyst_provider = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .analyst;
        let retry_diagnostic = self.retry_diagnostic_for_previous_run(&attempt, &role_run)?;
        let prompt = build_rework_prompt(
            &attempt,
            evidence,
            &source_stage,
            rework_round,
            &context_note_input,
            &evaluation_context_json,
            retry_diagnostic.as_deref(),
        );
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &analyst_provider,
                    prompt.clone(),
                    CodingPromptMode::FullConversation.event_detail(),
                ),
            })
            .await;

        let input = AdapterInput {
            provider_type: provider_type_for_name(&analyst_provider),
            role: AdapterRole::Reviewer,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt,
            context_files: Vec::new(),
            output_schema: "coding_workspace_analyst_verdict_json".to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Analyst,
            &analyst_provider,
        );
        let mut provider_input = streaming_input_from_adapter(&input, worktree_path.clone());
        provider_input.workspace_session_id = Some(attempt.id.clone());
        provider_input.resume_provider_session_id = resume_provider_session_id;
        provider_input.permission_mode =
            role_permission_mode_for_attempt(&self.store, &attempt, CodingProviderRole::Analyst)?;
        let full_output = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                role_run: Some(&role_run),
                provider,
                legacy_input: &input,
                input: provider_input,
                provider_name: &analyst_provider,
                provider_role: CodingProviderRole::Analyst,
                command_rx,
                allow_legacy_stream_fallback: true,
                timeout: None,
                timeout_reason_code: None,
            })
            .await?;
        let analyst_raw_ref = self.store.save_provider_raw_output(
            &attempt.id,
            CodingExecutionStage::Rework,
            "analyst_decision",
            &full_output,
        )?;
        self.store.update_role_run_refs(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            vec![analyst_raw_ref.clone()],
            Vec::new(),
        )?;
        if !note_ids.is_empty() {
            self.store.mark_context_notes_consumed(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &note_ids,
                rework_round,
            )?;
        }
        let mut decision = parse_analyst_verdict(&full_output, &source_stage);
        if decision.parse_error.is_some()
            && !decision
                .raw_provider_output_refs
                .iter()
                .any(|reference| reference == &analyst_raw_ref)
        {
            decision.raw_provider_output_refs.push(analyst_raw_ref);
        }
        let existing_decisions = self.store.list_analyst_decisions(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let decision_record = AnalystDecisionRecord {
            id: next_sequential_id("analyst_decision", existing_decisions.len()),
            attempt_id: attempt.id.clone(),
            source_stage: source_stage.clone(),
            rework_round,
            verdict: decision.structured_verdict.clone(),
            next_stage: decision.next_stage.clone().unwrap_or_else(|| {
                default_next_stage_for_legacy_verdict(&decision.structured_verdict, &source_stage)
            }),
            reason: decision.reason.clone(),
            evidence_refs: decision.evidence_refs.clone(),
            raw_provider_output_refs: decision.raw_provider_output_refs.clone(),
            rework_instructions: decision.rework_instructions.clone(),
            human_gate: decision.human_gate.clone(),
            created_at: Utc::now().to_rfc3339(),
            parse_error: decision.parse_error.clone(),
            role_run_id: Some(role_run.id.clone()),
            run_no: Some(role_run.run_no),
        };
        self.store.save_analyst_decision(&decision_record)?;
        self.emit_analyst_verdict_entry(
            &attempt,
            &node.id,
            rework_round,
            &source_stage,
            &decision,
            &role_run,
        )
        .await;
        let (updated, node_status, summary) = self
            .apply_analyst_decision(&attempt, &node.id, &source_stage, rework_round, &decision)
            .await?;
        let role_run_status = if node_status == CodingTimelineNodeStatus::Blocked {
            CodingRoleRunStatus::Blocked
        } else {
            CodingRoleRunStatus::Completed
        };
        self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            role_run_status,
            decision.parse_error.clone().or_else(|| {
                if node_status == CodingTimelineNodeStatus::Blocked {
                    Some("analyst_human_gate".to_string())
                } else {
                    None
                }
            }),
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            node_status,
            Some(summary),
        )
        .await?;
        Ok(updated)
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

    pub async fn handle_final_confirm(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        if current.status != CodingAttemptStatus::WaitingForHuman
            || current.stage != CodingExecutionStage::FinalConfirm
        {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt_id.to_string(),
            ));
        }

        self.generate_and_save_work_item_handoff_if_missing(&current)
            .await?;
        self.run_completion_gates(&current).await?;

        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Completed,
        )?;
        LifecycleStore::new(self.store.paths()).update_work_item_execution_status(
            &updated.project_id,
            &updated.issue_id,
            &updated.work_item_id,
            WorkItemStatus::Completed,
        )?;
        self.mark_issue_shared_worktree_completed_if_present(
            project_id,
            issue_id,
            &updated.work_item_id,
        )?;
        if let Some(node_id) =
            self.active_final_confirm_node_id(project_id, issue_id, attempt_id)?
        {
            let completed_at = Utc::now().to_rfc3339();
            self.store.update_timeline_node_status(
                project_id,
                issue_id,
                attempt_id,
                &node_id,
                CodingTimelineNodeStatus::Completed,
                Some("用户已确认完成".to_string()),
                Some(completed_at.clone()),
            )?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                    node_id,
                    status: CodingTimelineNodeStatus::Completed,
                    summary: Some("用户已确认完成".to_string()),
                    completed_at: Some(completed_at),
                })
                .await;
        }
        Ok(updated)
    }

    pub async fn handle_abort(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        self.ensure_issue_shared_worktree_clean(
            project_id,
            issue_id,
            attempt_id,
            &current.work_item_id,
        )
        .await?;

        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Aborted,
        )?;
        self.release_issue_shared_worktree_lock_if_holder(
            project_id,
            issue_id,
            &updated.work_item_id,
        )?;
        if let Some(node_id) = self.active_timeline_node_id(project_id, issue_id, attempt_id)? {
            let completed_at = Utc::now().to_rfc3339();
            self.store.update_timeline_node_status(
                project_id,
                issue_id,
                attempt_id,
                &node_id,
                CodingTimelineNodeStatus::Failed,
                Some("用户已中止".to_string()),
                Some(completed_at.clone()),
            )?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                    node_id,
                    status: CodingTimelineNodeStatus::Failed,
                    summary: Some("用户已中止".to_string()),
                    completed_at: Some(completed_at),
                })
                .await;
        }
        Ok(updated)
    }

    pub async fn handle_attempt_failed(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let updated = if current.status != CodingAttemptStatus::Failed {
            self.store.update_attempt_status(
                project_id,
                issue_id,
                attempt_id,
                CodingAttemptStatus::Failed,
            )?
        } else {
            current
        };

        self.ensure_issue_shared_worktree_clean(
            project_id,
            issue_id,
            attempt_id,
            &updated.work_item_id,
        )
        .await?;
        self.release_issue_shared_worktree_lock_if_holder(
            project_id,
            issue_id,
            &updated.work_item_id,
        )?;
        Ok(updated)
    }

    pub async fn handle_delete_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        self.ensure_issue_shared_worktree_clean(
            project_id,
            issue_id,
            attempt_id,
            &current.work_item_id,
        )
        .await?;
        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Aborted,
        )?;
        self.release_issue_shared_worktree_lock_if_holder(
            project_id,
            issue_id,
            &updated.work_item_id,
        )?;
        Ok(())
    }

    async fn ensure_issue_shared_worktree_clean(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        work_item_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let shared = match lifecycle.get_issue_shared_worktree(project_id, issue_id)? {
            Some(shared) => shared,
            None => return Ok(()),
        };
        if shared.current_active_work_item_id.as_deref() != Some(work_item_id) {
            return Ok(());
        }
        let worktree_path = shared.worktree_path;
        if !worktree_path.exists() {
            return Ok(());
        }
        let status = self._git_service.git_status(&worktree_path).await?;
        if !status.is_empty() {
            self.store.create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt_id.to_string(),
                stage: CodingExecutionStage::FinalConfirm,
                node_id: None,
                role: None,
                title: "Shared worktree has uncommitted changes".to_string(),
                description: "Issue shared worktree has uncommitted changes and must be cleaned up manually before the active lock can be released".to_string(),
                reason_code: Some("shared_worktree_dirty_manual_gate".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: vec![
                    coding_gate_action_for_id("manual_continue").expect("manual continue action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            })?;
            return Err(CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(
                worktree_path.to_string_lossy().to_string(),
            ));
        }
        Ok(())
    }

    async fn generate_and_save_work_item_handoff_if_missing(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        if self
            .store
            .get_work_item_handoff(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .is_some()
        {
            return Ok(());
        }

        let handoff = if let Some(provider) = self.provider.as_ref() {
            self.generate_work_item_handoff_from_provider(provider, attempt)
                .await?
        } else {
            self.generate_placeholder_work_item_handoff(attempt).await?
        };

        self.store.save_work_item_handoff(&handoff)?;

        let lifecycle = LifecycleStore::new(self.store.paths());
        if lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .iter()
            .any(|item| item.id == attempt.work_item_id)
        {
            lifecycle.update_work_item_handoff_summary(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.work_item_id,
                Some(format!(
                    "projects/{}/issues/{}/coding-attempts/{}/work-item-handoff.json",
                    attempt.project_id, attempt.issue_id, attempt.id
                )),
                attempt.head_commit.clone(),
            )?;
        }

        Ok(())
    }

    async fn generate_placeholder_work_item_handoff(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<WorkItemHandoff, CodingWorkspaceEngineError> {
        let testing_reports = self
            .store
            .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let tests_run: Vec<String> = testing_reports
            .iter()
            .flat_map(|report| report.commands.iter().map(|cmd| cmd.command.join(" ")))
            .collect();
        let test_result_summary = testing_reports
            .last()
            .map(|report| format!("{:?}", report.overall_status))
            .unwrap_or_else(|| "no testing report".to_string());

        let review_requests = self
            .store
            .list_review_requests(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let review_summary = review_requests.last().map(|r| format!("{:?}", r.push_status));

        Ok(WorkItemHandoff {
            id: format!(
                "work_item_handoff_{}_{}_{}",
                attempt.project_id, attempt.issue_id, attempt.id
            ),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            work_item_id: attempt.work_item_id.clone(),
            attempt_id: attempt.id.clone(),
            provider_run_ref: None,
            summary: "Handoff generated from attempt artifacts".to_string(),
            files_changed: Vec::new(),
            commit_sha: attempt.head_commit.clone(),
            diff_summary: String::new(),
            tests_run,
            test_result_summary,
            review_summary,
            api_or_contract_changes: Vec::new(),
            open_risks: Vec::new(),
            next_work_item_notes: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
        })
    }

    async fn generate_work_item_handoff_from_provider(
        &self,
        provider: &Arc<dyn ProviderAdapter + Send + Sync>,
        attempt: &CodingExecutionAttempt,
    ) -> Result<WorkItemHandoff, CodingWorkspaceEngineError> {
        let worktree_path = self.attempt_worktree_path(attempt).await?;
        let provider_type = provider_type_for_name(&attempt.provider_config_snapshot.author);
        let output_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "summary": {"type": "string"},
                "files_changed": {"type": "array", "items": {"type": "string"}},
                "diff_summary": {"type": "string"},
                "tests_run": {"type": "array", "items": {"type": "string"}},
                "test_result_summary": {"type": "string"},
                "api_or_contract_changes": {"type": "array", "items": {"type": "string"}},
                "next_work_item_notes": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["summary"]
        });
        let input = AdapterInput {
            provider_type,
            role: AdapterRole::Handoff,
            prompt: "Generate a concise handoff summary for the completed work item.".to_string(),
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            context_files: Vec::new(),
            output_schema: output_schema.to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };

        let output = tokio::task::spawn_blocking({
            let provider = Arc::clone(provider);
            move || provider.run(&input)
        })
        .await
        .map_err(|error| CodingWorkspaceEngineError::ProviderStream(error.to_string()))??;

        let structured = output.structured_output.unwrap_or_default();
        Ok(WorkItemHandoff {
            id: format!(
                "work_item_handoff_{}_{}_{}",
                attempt.project_id, attempt.issue_id, attempt.id
            ),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            work_item_id: attempt.work_item_id.clone(),
            attempt_id: attempt.id.clone(),
            provider_run_ref: None,
            summary: structured
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("Completed work item")
                .to_string(),
            files_changed: structured
                .get("files_changed")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            commit_sha: attempt.head_commit.clone(),
            diff_summary: structured
                .get("diff_summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            tests_run: structured
                .get("tests_run")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            test_result_summary: structured
                .get("test_result_summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            review_summary: None,
            api_or_contract_changes: structured
                .get("api_or_contract_changes")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            open_risks: Vec::new(),
            next_work_item_notes: structured
                .get("next_work_item_notes")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            created_at: Utc::now().to_rfc3339(),
        })
    }

    async fn run_completion_gates(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CompletionGateReport, CodingWorkspaceEngineError> {
        if attempt.head_commit.is_none() {
            return Err(CodingWorkspaceEngineError::CompletionCommitMissing(
                attempt.id.clone(),
            ));
        }

        let lifecycle = LifecycleStore::new(self.store.paths());
        let work_item = lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .into_iter()
            .find(|item| item.id == attempt.work_item_id)
            .ok_or_else(|| CodingWorkspaceEngineError::FinalConfirmNotReady(attempt.id.clone()))?;

        let changed_files = self
            .changed_files_for_attempt(attempt, &work_item)
            .await?;
        let worktree_path = self.attempt_worktree_path(attempt).await.ok();
        for relative_path in &changed_files {
            let candidate = std::path::Path::new(relative_path);
            if work_item
                .forbidden_write_scopes
                .iter()
                .any(|scope| scope_allows_path(scope, relative_path, true))
            {
                return Err(CodingWorkspaceEngineError::WorkItemDiffScopeViolation(
                    relative_path.clone(),
                ));
            }
            if !work_item.exclusive_write_scopes.is_empty()
                && let Some(ref base) = worktree_path
            {
                let _ = validate_write_path(
                    base,
                    &work_item.exclusive_write_scopes,
                    candidate,
                    true,
                )
                .map_err(|_| {
                    CodingWorkspaceEngineError::WorkItemDiffScopeViolation(relative_path.clone())
                })?;
            }
        }

        if let Some(plan_ref) = &work_item.verification_plan_ref {
            let verification_plan = lifecycle.get_verification_plan(
                &attempt.project_id,
                &attempt.issue_id,
                plan_ref,
            )?;
            if !verification_plan.required_gates.is_empty() {
                let reports = self
                    .store
                    .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
                let passed = reports.iter().any(|report| {
                    report.overall_status == TestingOverallStatus::Passed
                        || report.overall_status == TestingOverallStatus::PassedWithWarnings
                });
                if !passed {
                    return Err(CodingWorkspaceEngineError::VerificationGateResultMissing(
                        attempt.id.clone(),
                    ));
                }
            }
        }

        if self
            .store
            .get_work_item_handoff(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .is_none()
        {
            return Err(CodingWorkspaceEngineError::WorkItemHandoffMissing(
                attempt.id.clone(),
            ));
        }

        self.ensure_issue_shared_worktree_clean(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &attempt.work_item_id,
        )
        .await?;

        Ok(CompletionGateReport)
    }

    async fn changed_files_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
        _work_item: &LifecycleWorkItemRecord,
    ) -> Result<Vec<String>, CodingWorkspaceEngineError> {
        let worktree_path = match self.attempt_worktree_path(attempt).await {
            Ok(path) => path,
            Err(CodingWorkspaceEngineError::MissingWorktree(_)) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        if !worktree_path.exists() {
            return Ok(Vec::new());
        }
        match self._git_service.git_status(&worktree_path).await {
            Ok(status) => Ok(status.into_iter().map(|file| file.path).collect()),
            Err(_) => Ok(Vec::new()),
        }
    }

    async fn attempt_worktree_path(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<PathBuf, CodingWorkspaceEngineError> {
        if let Some(path) = attempt.worktree_path.as_ref() {
            return Ok(path.clone());
        }
        let lifecycle = LifecycleStore::new(self.store.paths());
        let shared = lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)?;
        match shared {
            Some(shared) if shared.worktree_path.exists() => Ok(shared.worktree_path),
            _ => Err(CodingWorkspaceEngineError::MissingWorktree(attempt.id.clone())),
        }
    }

    fn release_issue_shared_worktree_lock_if_holder(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let shared = match lifecycle.get_issue_shared_worktree(project_id, issue_id)? {
            Some(shared) => shared,
            None => return Ok(()),
        };
        if shared.current_active_work_item_id.as_deref() == Some(work_item_id) {
            lifecycle.release_issue_worktree_lock(project_id, issue_id, work_item_id)?;
        }
        Ok(())
    }

    fn mark_issue_shared_worktree_completed_if_present(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        if lifecycle
            .get_issue_shared_worktree(project_id, issue_id)?
            .is_some()
        {
            lifecycle.mark_issue_worktree_completed_item(project_id, issue_id, work_item_id)?;
        }
        Ok(())
    }

    async fn release_active_lock_if_shared_worktree_clean(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        work_item_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        match self
            .ensure_issue_shared_worktree_clean(project_id, issue_id, attempt_id, work_item_id)
            .await
        {
            Ok(()) => self.release_issue_shared_worktree_lock_if_holder(
                project_id,
                issue_id,
                work_item_id,
            ),
            Err(CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(_)) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn handle_blocked_gate_response(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
        action_id: &str,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let Some(gate) = self
            .store
            .list_open_blocked_gates(project_id, issue_id, attempt_id)?
            .into_iter()
            .find(|gate| gate.gate_id == gate_id)
        else {
            return Ok(self.store.get_attempt(project_id, issue_id, attempt_id)?);
        };
        let action = gate
            .available_actions
            .iter()
            .find(|action| action.action_id == action_id)
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "coding_gate_action_not_allowed".to_string(),
                )
            })?;
        let should_resolve_gate =
            !matches!(action.action_type, CodingGateActionType::ProvideContext);

        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let updated = match action.action_type {
            CodingGateActionType::Abort => {
                self.handle_abort(project_id, issue_id, attempt_id).await?
            }
            CodingGateActionType::RetryTestPlan
            | CodingGateActionType::RerunMissingSteps
            | CodingGateActionType::RerunTesting => {
                let trigger = match action.action_type {
                    CodingGateActionType::RetryTestPlan => CodingRoleRunTrigger::RetryTestPlan,
                    CodingGateActionType::RerunMissingSteps => {
                        CodingRoleRunTrigger::RerunMissingSteps
                    }
                    CodingGateActionType::RerunTesting => CodingRoleRunTrigger::ManualRerun,
                    _ => CodingRoleRunTrigger::ManualRerun,
                };
                let resumed =
                    self.resume_blocked_attempt_at_stage(&current, CodingExecutionStage::Testing)?;
                self.store.supersede_latest_role_run_and_create(
                    &resumed,
                    CodingExecutionStage::Testing,
                    CodingProviderRole::Tester,
                    trigger,
                    None,
                    gate.reason_code.clone(),
                )?;
                resumed
            }
            CodingGateActionType::RetryReview => {
                if gate.stage == Some(CodingExecutionStage::InternalPrReview)
                    || gate.role == Some(CodingProviderRole::InternalReviewer)
                {
                    let resumed = self.resume_blocked_attempt_at_stage(
                        &current,
                        CodingExecutionStage::InternalPrReview,
                    )?;
                    self.store.supersede_latest_role_run_and_create(
                        &resumed,
                        CodingExecutionStage::InternalPrReview,
                        CodingProviderRole::InternalReviewer,
                        CodingRoleRunTrigger::RetryInternalReview,
                        None,
                        gate.reason_code.clone(),
                    )?;
                    resumed
                } else {
                    let resumed = self.resume_blocked_attempt_at_stage(
                        &current,
                        CodingExecutionStage::CodeReview,
                    )?;
                    self.store.supersede_latest_role_run_and_create(
                        &resumed,
                        CodingExecutionStage::CodeReview,
                        CodingProviderRole::CodeReviewer,
                        CodingRoleRunTrigger::RetryReview,
                        None,
                        gate.reason_code.clone(),
                    )?;
                    resumed
                }
            }
            CodingGateActionType::RetryInternalReview => {
                let resumed = self.resume_blocked_attempt_at_stage(
                    &current,
                    CodingExecutionStage::InternalPrReview,
                )?;
                self.store.supersede_latest_role_run_and_create(
                    &resumed,
                    CodingExecutionStage::InternalPrReview,
                    CodingProviderRole::InternalReviewer,
                    CodingRoleRunTrigger::RetryInternalReview,
                    None,
                    gate.reason_code.clone(),
                )?;
                resumed
            }
            CodingGateActionType::RetryAnalyst => {
                let previous = self.store.latest_role_run(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    CodingExecutionStage::Rework,
                    CodingProviderRole::Analyst,
                )?;
                let resumed =
                    self.resume_blocked_attempt_at_stage(&current, CodingExecutionStage::Rework)?;
                let new_run = self.store.supersede_latest_role_run_and_create(
                    &resumed,
                    CodingExecutionStage::Rework,
                    CodingProviderRole::Analyst,
                    CodingRoleRunTrigger::RetryAnalyst,
                    None,
                    gate.reason_code.clone(),
                )?;
                if let Some(previous) = previous {
                    self.store.update_role_run_refs(
                        &resumed.project_id,
                        &resumed.issue_id,
                        &resumed.id,
                        &new_run.id,
                        Vec::new(),
                        previous.artifact_refs,
                    )?;
                }
                resumed
            }
            CodingGateActionType::AcceptTestingResult => {
                self.accept_testing_result_for_analyst(&current, &gate)?
            }
            CodingGateActionType::ContinueRework => {
                self.continue_rework_after_limit_for_attempt(&current, extra_context)?
            }
            CodingGateActionType::SendRawOutputToAnalyst => {
                self.resume_blocked_attempt_at_stage(&current, CodingExecutionStage::Rework)?
            }
            CodingGateActionType::ProvideContext => {
                if let Some(content) = extra_context
                    && !content.trim().is_empty()
                {
                    self.store.create_context_note(&current.id, content)?;
                }
                let running = if current.status == CodingAttemptStatus::Blocked {
                    self.store.update_attempt_status(
                        project_id,
                        issue_id,
                        attempt_id,
                        CodingAttemptStatus::Running,
                    )?
                } else {
                    current
                };
                self.store.update_attempt_status(
                    &running.project_id,
                    &running.issue_id,
                    &running.id,
                    CodingAttemptStatus::WaitingForHuman,
                )?
            }
            CodingGateActionType::ManualContinue | CodingGateActionType::AcceptRisk => {
                let operator_context = extra_context
                    .map(|content| content.trim().to_string())
                    .filter(|content| !content.is_empty())
                    .ok_or_else(|| {
                        CodingWorkspaceEngineError::ProviderStream(
                            "coding_gate_extra_context_required".to_string(),
                        )
                    })?;
                self.store
                    .create_context_note(&current.id, operator_context.clone())?;
                self.store
                    .create_quality_bypass_audit(CreateQualityBypassAuditInput {
                        attempt_id: current.id.clone(),
                        gate_id: gate.gate_id.clone(),
                        stage: gate.stage.clone().unwrap_or_else(|| current.stage.clone()),
                        reason_code: gate.reason_code.clone(),
                        skipped_required_steps: self.latest_missing_required_steps(&current)?,
                        operator_context,
                    })?;
                if current.status == CodingAttemptStatus::Blocked {
                    self.store.update_attempt_status(
                        project_id,
                        issue_id,
                        attempt_id,
                        CodingAttemptStatus::Running,
                    )?
                } else {
                    current
                }
            }
            _ => {
                return Err(CodingWorkspaceEngineError::ProviderStream(
                    "coding_gate_action_not_allowed".to_string(),
                ));
            }
        };
        if should_resolve_gate {
            self.store
                .resolve_blocked_gate(project_id, issue_id, attempt_id, gate_id)?;
        }
        Ok(updated)
    }

    pub fn continue_rework_after_limit(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        self.continue_rework_after_limit_for_attempt(&current, extra_context)
    }

    fn continue_rework_after_limit_for_attempt(
        &self,
        current: &CodingExecutionAttempt,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if current.stage != CodingExecutionStage::Rework
            || !matches!(
                current.status,
                CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
            )
            || current.rework_count < current.max_auto_rework
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "continue_rework_not_available".to_string(),
            ));
        }

        if let Some(content) = extra_context
            && !content.trim().is_empty()
        {
            self.store
                .create_context_note(&current.id, content.trim().to_string())?;
        }

        let decision = self
            .store
            .latest_analyst_decision(&current.project_id, &current.issue_id, &current.id)?
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "continue_rework_missing_analyst_decision".to_string(),
                )
            })?;
        if decision.verdict != AnalystDecisionVerdict::NeedsFix
            || decision.next_stage != AnalystDecisionNextStage::Coding
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "continue_rework_latest_decision_not_coding".to_string(),
            ));
        }

        let existing = self.store.list_rework_instructions(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        let (summary, fix_hints) = rework_instruction_fields_from_analyst_record(&decision);
        let instruction = CodingReworkInstruction {
            id: next_sequential_id("coding_rework_instruction", existing.len()),
            attempt_id: current.id.clone(),
            source_stage: decision.source_stage.clone(),
            rework_round: decision.rework_round,
            summary,
            fix_hints,
            questions: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            consumed_by_node_id: None,
            consumed_at: None,
        };
        self.store.save_rework_instruction(&instruction)?;

        let running = if current.status == CodingAttemptStatus::Running {
            current.clone()
        } else {
            self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Running,
            )?
        };
        let updated = self.store.increment_attempt_rework_count(
            &running.project_id,
            &running.issue_id,
            &running.id,
        )?;
        self.store
            .update_attempt_stage(
                &updated.project_id,
                &updated.issue_id,
                &updated.id,
                CodingExecutionStage::Coding,
            )
            .map_err(CodingWorkspaceEngineError::from)
    }

    fn latest_missing_required_steps(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Vec<String>, ProductStoreError> {
        let Some(report) = self
            .store
            .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .last()
        else {
            return Ok(Vec::new());
        };
        let mut steps = Vec::new();
        for step in report
            .missing_required_steps
            .into_iter()
            .chain(report.skipped_required_steps)
        {
            if !steps.contains(&step) {
                steps.push(step);
            }
        }
        Ok(steps)
    }

    fn accept_testing_result_for_analyst(
        &self,
        current: &CodingExecutionAttempt,
        gate: &CodingGateRequired,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let report = self.testing_report_for_gate(current, gate)?;
        let running = if current.status == CodingAttemptStatus::Blocked {
            self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Running,
            )?
        } else {
            current.clone()
        };
        let role_run = self.store.create_role_run(
            &running,
            CodingExecutionStage::Rework,
            CodingProviderRole::Analyst,
            CodingRoleRunTrigger::Initial,
            None,
        )?;
        let evidence = testing_report_to_analyst_evidence(&report);
        let evidence_ref = self.store.save_analyst_evidence(&running.id, &evidence)?;
        self.store.update_role_run_refs(
            &running.project_id,
            &running.issue_id,
            &running.id,
            &role_run.id,
            Vec::new(),
            vec![evidence_ref],
        )?;
        Ok(running)
    }

    fn testing_report_for_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        gate: &CodingGateRequired,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        if let Some(report_id) = gate
            .evidence_refs
            .iter()
            .rev()
            .find_map(|reference| reference.strip_suffix(".json"))
        {
            return Ok(self.store.get_testing_report(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                report_id,
            )?);
        }
        self.store
            .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .last()
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "testing_result_review_missing_report".to_string(),
                )
            })
    }

    fn resume_blocked_attempt_at_stage(
        &self,
        current: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let mut updated = if matches!(
            current.status,
            CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
        ) {
            self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Running,
            )?
        } else {
            current.clone()
        };
        if updated.stage != stage {
            updated = self.store.update_attempt_stage(
                &updated.project_id,
                &updated.issue_id,
                &updated.id,
                stage,
            )?;
        }
        Ok(updated)
    }

    fn create_running_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::WorktreePrepare,
            title: "准备 worktree".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Git),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_testing_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            title: "执行测试".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Tester),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_coding_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Coding,
            title: "代码编写".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Author),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_review_request_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::ReviewRequest,
            title: "发起 review request".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Git),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_code_review_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::CodeReview,
            title: "代码审查".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_rework_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
        rework_round: u32,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Rework,
            title: format!("分析官判定 #{}", rework_round),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::System),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_internal_pr_review_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::InternalPrReview,
            title: "内部 PR 审查".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_completed_final_confirm_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let now = Utc::now().to_rfc3339();
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::FinalConfirm,
            title: "最终确认".to_string(),
            status: CodingTimelineNodeStatus::Completed,
            agent_role: Some(CodingAgentRole::System),
            summary: Some("Analyst 最终判定通过，attempt 已完成".to_string()),
            started_at: now.clone(),
            completed_at: Some(now),
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    async fn build_code_review_prompt(
        &self,
        attempt: &CodingExecutionAttempt,
        worktree_path: &Path,
        retry_diagnostic: Option<&str>,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let diff = self
            ._git_service
            .git_diff(worktree_path, &attempt.base_branch)
            .await?;
        let work_item = self.work_item_markdown_for_attempt(attempt)?;
        let evaluation_context_json =
            self.evaluation_context_json_for_role(attempt, EvaluationContextRole::CodeReviewer)?;
        let retry_diagnostic_section = retry_diagnostic
            .map(|summary| format!("\n上一轮 role run 诊断摘要:\n{}\n", summary))
            .unwrap_or_default();
        Ok(format!(
            "Coding Workspace CodeReviewer\n\
             {}\n\
             你是 CodeReviewer，只分析当前变更 diff，不修改代码、不执行写操作。\n\
             Project: {}\n\
             Issue: {}\n\
             Work Item: {}\n\
             Attempt: {}\n\
             Branch: {}\n\
             Base: {}\n\
             \n代码规范:\n\
             - 优先检查正确性、边界条件、测试覆盖、安全、性能和可维护性。\n\
             - findings 必须包含 severity、file_path、line、message、required_action、source_stage=code_review。\n\
             - 如果没有阻塞问题，verdict 使用 approve。\n\
             \n原始需求上下文:\n````markdown\n{}\n````\n\
             \nEvaluationContextPack:\n````json\n{}\n````\n\
             \ngit diff:\n````diff\n{}\n````\n\
             {}\
             \n只输出 JSON：{{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[...]}}\n",
            provider_runtime_contract("CodeReviewer"),
            attempt.project_id,
            attempt.issue_id,
            attempt.work_item_id,
            attempt.id,
            attempt.branch_name,
            attempt.base_branch,
            work_item.unwrap_or_else(
                || "未找到 Work Item markdown，上下文仅包含 attempt 元数据。".to_string()
            ),
            evaluation_context_json,
            truncate_prompt_section(&diff, 30_000),
            retry_diagnostic_section
        ))
    }

    async fn build_internal_pr_review_prompt(
        &self,
        attempt: &CodingExecutionAttempt,
        review_request: &ReviewRequest,
        worktree_path: &Path,
        retry_diagnostic: Option<&str>,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let diff = self
            ._git_service
            .git_diff(worktree_path, &attempt.base_branch)
            .await?;
        let work_item = self.work_item_markdown_for_attempt(attempt)?;
        let evaluation_context_json = self
            .evaluation_context_json_for_role(attempt, EvaluationContextRole::InternalReviewer)?;
        let retry_diagnostic_section = retry_diagnostic
            .map(|summary| format!("\n上一轮 role run 诊断摘要:\n{}\n", summary))
            .unwrap_or_default();
        Ok(format!(
            "Coding Workspace InternalReviewer\n\
             {}\n\
             你是 InternalReviewer，在 ReviewRequest(push) 之后做内部 PR 审查。\n\
             Project: {}\n\
             Issue: {}\n\
             Work Item: {}\n\
             Attempt: {}\n\
             Branch: {}\n\
             Review Request: {}\n\
             Review Remote: {}\n\
             Commit: {}\n\
             \n功能需求上下文:\n````markdown\n{}\n````\n\
             \nEvaluationContextPack:\n````json\n{}\n````\n\
             \n完整变更 git diff:\n````diff\n{}\n````\n\
             {}\
             \n输出要求:\n\
             - 分析影响范围（影响范围/impact_scope）。\n\
             - 给出 PR description 预览。\n\
             - 给出 commit message 建议。\n\
             - findings 必须包含 source_stage=internal_pr_review。\n\
             \n只输出 JSON：{{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[...],\"impact_scope\":[\"...\"],\"pr_description\":\"...\",\"commit_message_suggestion\":\"...\"}}\n",
            provider_runtime_contract("InternalReviewer"),
            attempt.project_id,
            attempt.issue_id,
            attempt.work_item_id,
            attempt.id,
            attempt.branch_name,
            review_request.id,
            review_request.remote,
            review_request.commit_sha,
            work_item.unwrap_or_else(
                || "未找到 Work Item markdown，上下文仅包含 attempt 元数据。".to_string()
            ),
            evaluation_context_json,
            truncate_prompt_section(&diff, 30_000),
            retry_diagnostic_section
        ))
    }

    fn evaluation_context_json_for_role(
        &self,
        attempt: &CodingExecutionAttempt,
        provider_role: EvaluationContextRole,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let context = build_evaluation_context_pack(self.store.paths(), attempt, provider_role)?;
        serde_json::to_string_pretty(&context).map_err(|error| {
            CodingWorkspaceEngineError::ProviderStream(format!(
                "serialize_evaluation_context_failed: {error}"
            ))
        })
    }

    fn retry_diagnostic_for_previous_run(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: &CodingRoleRun,
    ) -> Result<Option<String>, CodingWorkspaceEngineError> {
        let Some(previous_run_id) = role_run.supersedes_run_id.as_deref() else {
            return Ok(None);
        };
        self.store
            .role_run_retry_diagnostic_summary(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                previous_run_id,
            )
            .map_err(CodingWorkspaceEngineError::Store)
    }

    fn work_item_markdown_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Option<String>, ProductStoreError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let sessions = lifecycle.list_workspace_sessions(&attempt.project_id, &attempt.issue_id)?;
        let Some(session) = sessions.iter().rev().find(|session| {
            session.entity_id == attempt.work_item_id
                && session.workspace_type == WorkspaceType::WorkItem
        }) else {
            return Ok(None);
        };
        Ok(lifecycle
            .list_artifact_versions(&session.id)?
            .into_iter()
            .last()
            .map(|version| version.markdown))
    }

    fn build_code_review_report(
        &self,
        attempt: &CodingExecutionAttempt,
        full_output: &str,
        raw_provider_output_ref: Option<String>,
        role_run: &CodingRoleRun,
    ) -> Result<CodeReviewReport, ProductStoreError> {
        let existing = self.store.list_code_review_reports(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let payload = parse_review_payload(full_output, CodingExecutionStage::CodeReview);
        Ok(CodeReviewReport {
            id: next_sequential_id("code_review", existing.len()),
            attempt_id: attempt.id.clone(),
            round: existing.len() as u32 + 1,
            verdict: payload.verdict,
            findings: payload.findings,
            tested_evidence_refs: payload.tested_evidence_refs,
            diff_refs: payload.diff_refs,
            summary: payload.summary,
            created_at: Utc::now().to_rfc3339(),
            raw_provider_output_ref,
            role_run_id: Some(role_run.id.clone()),
            run_no: Some(role_run.run_no),
        })
    }

    fn build_internal_pr_review(
        &self,
        attempt: &CodingExecutionAttempt,
        review_request: &ReviewRequest,
        full_output: &str,
        raw_provider_output_ref: Option<String>,
        role_run: &CodingRoleRun,
    ) -> Result<InternalPrReview, ProductStoreError> {
        let existing = self.store.list_internal_pr_reviews(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let payload = parse_review_payload(full_output, CodingExecutionStage::InternalPrReview);
        Ok(InternalPrReview {
            id: next_sequential_id("internal_review", existing.len()),
            attempt_id: attempt.id.clone(),
            review_request_id: review_request.id.clone(),
            verdict: payload.verdict,
            findings: payload.findings,
            impact_scope: payload.impact_scope,
            pr_description: payload.pr_description,
            commit_message_suggestion: payload.commit_message_suggestion,
            tested_evidence_refs: payload.tested_evidence_refs,
            diff_refs: payload.diff_refs,
            summary: payload.summary,
            created_at: Utc::now().to_rfc3339(),
            raw_provider_output_ref,
            role_run_id: Some(role_run.id.clone()),
            run_no: Some(role_run.run_no),
        })
    }

    async fn emit_code_review_chat_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        report: &CodeReviewReport,
    ) {
        let entry = CodingChatEntry {
            id: format!("{node_id}_code_review_report"),
            attempt_id: attempt.id.clone(),
            node_id: Some(node_id.to_string()),
            role: CodingAgentRole::Reviewer,
            entry_type: CodingEntryType::AssistantMessage,
            content: Some(report.summary.clone()),
            metadata: Some(serde_json::json!({
                "source": "code_review",
                "review_id": &report.id,
                "verdict": &report.verdict,
                "findings_count": report.findings.len(),
                "role_run_id": report.role_run_id,
                "run_no": report.run_no,
            })),
            created_at: Utc::now().to_rfc3339(),
        };
        self.save_and_emit_chat_entry(entry).await;
    }

    async fn emit_internal_pr_review_chat_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        review: &InternalPrReview,
    ) {
        let entry = CodingChatEntry {
            id: format!("{node_id}_internal_pr_review"),
            attempt_id: attempt.id.clone(),
            node_id: Some(node_id.to_string()),
            role: CodingAgentRole::Reviewer,
            entry_type: CodingEntryType::AssistantMessage,
            content: Some(review.summary.clone()),
            metadata: Some(serde_json::json!({
                "source": "internal_pr_review",
                "review_id": &review.id,
                "review_request_id": &review.review_request_id,
                "verdict": &review.verdict,
                "impact_scope": &review.impact_scope,
                "role_run_id": review.role_run_id,
                "run_no": review.run_no,
            })),
            created_at: Utc::now().to_rfc3339(),
        };
        self.save_and_emit_chat_entry(entry).await;
    }

    async fn save_and_emit_chat_entry(&self, entry: CodingChatEntry) {
        let _ = self.store.save_chat_entry(&entry);
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingChatEntryCreated { entry })
            .await;
    }

    fn active_worktree_prepare_node_id(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        Ok(self
            .store
            .get_timeline_nodes(project_id, issue_id, attempt_id)?
            .into_iter()
            .rev()
            .find(|node| {
                node.stage == CodingExecutionStage::WorktreePrepare
                    && node.status == CodingTimelineNodeStatus::Running
            })
            .map(|node| node.id))
    }

    fn active_timeline_node_id(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        Ok(self
            .store
            .get_timeline_nodes(project_id, issue_id, attempt_id)?
            .into_iter()
            .rev()
            .find(|node| {
                matches!(
                    node.status,
                    CodingTimelineNodeStatus::Pending
                        | CodingTimelineNodeStatus::Running
                        | CodingTimelineNodeStatus::Blocked
                )
            })
            .map(|node| node.id))
    }

    fn active_final_confirm_node_id(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        Ok(self
            .store
            .get_timeline_nodes(project_id, issue_id, attempt_id)?
            .into_iter()
            .rev()
            .find(|node| {
                node.stage == CodingExecutionStage::FinalConfirm
                    && matches!(
                        node.status,
                        CodingTimelineNodeStatus::Pending
                            | CodingTimelineNodeStatus::Running
                            | CodingTimelineNodeStatus::Blocked
                    )
            })
            .map(|node| node.id))
    }

    async fn complete_timeline_node(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        node_id: &str,
        status: CodingTimelineNodeStatus,
        summary: Option<String>,
    ) -> Result<(), ProductStoreError> {
        let completed_at = Utc::now().to_rfc3339();
        self.store.update_timeline_node_status(
            project_id,
            issue_id,
            attempt_id,
            node_id,
            status.clone(),
            summary.clone(),
            Some(completed_at.clone()),
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                node_id: node_id.to_string(),
                status,
                summary,
                completed_at: Some(completed_at),
            })
            .await;
        Ok(())
    }

    async fn emit_analyst_verdict_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        rework_round: u32,
        source_stage: &CodingExecutionStage,
        decision: &AnalystDecision,
        role_run: &CodingRoleRun,
    ) {
        let mut metadata = serde_json::json!({
            "source": "analyst",
            "source_stage": source_stage,
            "rework_round": rework_round,
            "role_run_id": role_run.id,
            "run_no": role_run.run_no,
        });
        if let Some(object) = metadata.as_object_mut() {
            object.insert(
                "structured_verdict".to_string(),
                serde_json::json!(&decision.structured_verdict),
            );
            object.insert(
                "next_stage".to_string(),
                serde_json::json!(decision.next_stage.clone().unwrap_or_else(|| {
                    default_next_stage_for_legacy_verdict(
                        &decision.structured_verdict,
                        source_stage,
                    )
                })),
            );
            object.insert("reason".to_string(), serde_json::json!(&decision.reason));
            object.insert(
                "evidence_refs".to_string(),
                serde_json::json!(&decision.evidence_refs),
            );
            object.insert(
                "raw_provider_output_refs".to_string(),
                serde_json::json!(&decision.raw_provider_output_refs),
            );
            if let Some(instructions) = decision.rework_instructions.as_ref() {
                object.insert(
                    "rework_instructions".to_string(),
                    serde_json::json!(instructions),
                );
            }
            if let Some(human_gate) = decision.human_gate.as_ref() {
                object.insert("human_gate".to_string(), serde_json::json!(human_gate));
            }
            if !decision.fix_hints.is_empty() {
                object.insert(
                    "fix_hints".to_string(),
                    serde_json::json!(&decision.fix_hints),
                );
            }
            if !decision.questions.is_empty() {
                object.insert(
                    "questions".to_string(),
                    serde_json::json!(&decision.questions),
                );
            }
            if let Some(parse_error) = decision.parse_error.as_ref() {
                object.insert("parse_error".to_string(), serde_json::json!(parse_error));
            }
        }
        let entry = CodingChatEntry {
            id: format!("{node_id}_analyst_verdict"),
            attempt_id: attempt.id.clone(),
            node_id: Some(node_id.to_string()),
            role: CodingAgentRole::System,
            entry_type: CodingEntryType::AnalystVerdict {
                verdict: decision.verdict.clone(),
            },
            content: Some(decision.summary.clone()),
            metadata: Some(metadata),
            created_at: Utc::now().to_rfc3339(),
        };
        self.save_and_emit_chat_entry(entry).await;
    }

    async fn emit_rewrite_limit_warning_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        rework_round: u32,
        decision: &AnalystDecision,
    ) {
        let message = "已达到自动重写上限，跳过 Coding 并进入 CodeReview。".to_string();
        let entry = CodingChatEntry {
            id: format!("{node_id}_rewrite_limit_warning"),
            attempt_id: attempt.id.clone(),
            node_id: Some(node_id.to_string()),
            role: CodingAgentRole::System,
            entry_type: CodingEntryType::SystemEvent {
                event_type: "exceeded_rewrite_limit".to_string(),
                message: message.clone(),
            },
            content: Some(message),
            metadata: Some(serde_json::json!({
                "source": "analyst",
                "rework_round": rework_round,
                "rework_count": attempt.rework_count,
                "max_auto_rework": attempt.max_auto_rework,
                "analyst_summary": &decision.summary,
            })),
            created_at: Utc::now().to_rfc3339(),
        };
        self.save_and_emit_chat_entry(entry).await;
    }

    async fn complete_attempt_after_final_rework(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.generate_and_save_work_item_handoff_if_missing(attempt)
            .await?;
        self.run_completion_gates(attempt).await?;
        let staged = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::FinalConfirm,
        )?;
        let completed = self.store.update_attempt_status(
            &staged.project_id,
            &staged.issue_id,
            &staged.id,
            CodingAttemptStatus::Completed,
        )?;
        self.mark_work_item_completed_if_present(&completed)?;
        self.mark_issue_shared_worktree_completed_if_present(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.work_item_id,
        )?;
        let node = self.create_completed_final_confirm_timeline_node(&completed)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node })
            .await;
        Ok(completed)
    }

    fn mark_work_item_completed_if_present(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let exists = lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .iter()
            .any(|work_item| work_item.id == attempt.work_item_id);
        if exists {
            lifecycle.update_work_item_execution_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.work_item_id,
                WorkItemStatus::Completed,
            )?;
        }
        Ok(())
    }

    async fn apply_analyst_decision(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        source_stage: &CodingExecutionStage,
        rework_round: u32,
        decision: &AnalystDecision,
    ) -> Result<
        (CodingExecutionAttempt, CodingTimelineNodeStatus, String),
        CodingWorkspaceEngineError,
    > {
        let next_stage = decision.next_stage.clone().unwrap_or_else(|| {
            default_next_stage_for_legacy_verdict(&decision.structured_verdict, source_stage)
        });

        match next_stage {
            AnalystDecisionNextStage::Coding => {
                if attempt.rework_count < attempt.max_auto_rework {
                    let existing = self.store.list_rework_instructions(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                    )?;
                    let instruction_summary = decision
                        .rework_instructions
                        .as_ref()
                        .map(|instruction| instruction.summary.clone())
                        .unwrap_or_else(|| decision.summary.clone());
                    let instruction_fix_hints = decision
                        .rework_instructions
                        .as_ref()
                        .map(|instruction| {
                            instruction
                                .required_changes
                                .iter()
                                .chain(instruction.verification_expectations.iter())
                                .cloned()
                                .collect::<Vec<_>>()
                        })
                        .filter(|items| !items.is_empty())
                        .unwrap_or_else(|| decision.fix_hints.clone());
                    let instruction = CodingReworkInstruction {
                        id: next_sequential_id("coding_rework_instruction", existing.len()),
                        attempt_id: attempt.id.clone(),
                        source_stage: source_stage.clone(),
                        rework_round,
                        summary: instruction_summary,
                        fix_hints: instruction_fix_hints,
                        questions: decision.questions.clone(),
                        created_at: Utc::now().to_rfc3339(),
                        consumed_by_node_id: None,
                        consumed_at: None,
                    };
                    self.store.save_rework_instruction(&instruction)?;
                    let updated = self.store.increment_attempt_rework_count(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                    )?;
                    let updated = self.store.update_attempt_stage(
                        &updated.project_id,
                        &updated.issue_id,
                        &updated.id,
                        CodingExecutionStage::Coding,
                    )?;
                    Ok((
                        updated,
                        CodingTimelineNodeStatus::Completed,
                        format!("NeedsFix: {}", decision.summary),
                    ))
                } else {
                    self.emit_rewrite_limit_warning_entry(attempt, node_id, rework_round, decision)
                        .await;
                    let updated = self.store.update_attempt_status(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        CodingAttemptStatus::Blocked,
                    )?;
                    let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
                        attempt_id: attempt.id.clone(),
                        stage: CodingExecutionStage::Rework,
                        node_id: Some(node_id.to_string()),
                        role: Some(CodingProviderRole::Analyst),
                        title: "Rework limit reached".to_string(),
                        description: format!("{}；已达到自动重写上限", decision.summary),
                        reason_code: Some("max_auto_rework_exceeded".to_string()),
                        evidence_refs: decision.evidence_refs.clone(),
                        raw_provider_output_ref: decision.raw_provider_output_refs.first().cloned(),
                        available_actions: vec![
                            coding_gate_action_for_id("continue_rework")
                                .expect("continue rework action"),
                            coding_gate_action_for_id("provide_context")
                                .expect("provide context action"),
                            coding_gate_action_for_id("manual_continue")
                                .expect("manual continue action"),
                            coding_gate_action_for_id("abort").expect("abort action"),
                        ],
                    })?;
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingGateRequired { gate })
                        .await;
                    match self
                        .ensure_issue_shared_worktree_clean(
                            &attempt.project_id,
                            &attempt.issue_id,
                            &attempt.id,
                            &attempt.work_item_id,
                        )
                        .await
                    {
                        Err(
                            error @ CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(_),
                        ) => {
                            let _ = error;
                        }
                        Err(error) => return Err(error),
                        Ok(()) => {
                            self.release_issue_shared_worktree_lock_if_holder(
                                &attempt.project_id,
                                &attempt.issue_id,
                                &attempt.work_item_id,
                            )?;
                        }
                    }
                    Ok((
                        updated,
                        CodingTimelineNodeStatus::Blocked,
                        format!("NeedsFix: {}；已达到自动重写上限", decision.summary),
                    ))
                }
            }
            AnalystDecisionNextStage::Testing => {
                let updated = self.store.update_attempt_stage(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingExecutionStage::Testing,
                )?;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Completed,
                    format!("RerunTesting: {}", decision.summary),
                ))
            }
            AnalystDecisionNextStage::CodeReview => {
                let updated = self.store.update_attempt_stage(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingExecutionStage::CodeReview,
                )?;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Completed,
                    format!("NextStage CodeReview: {}", decision.summary),
                ))
            }
            AnalystDecisionNextStage::ReviewRequest => {
                let updated = self.store.update_attempt_stage(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingExecutionStage::ReviewRequest,
                )?;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Completed,
                    format!("NextStage ReviewRequest: {}", decision.summary),
                ))
            }
            AnalystDecisionNextStage::InternalPrReview => {
                let updated = self.store.update_attempt_stage(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingExecutionStage::InternalPrReview,
                )?;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Completed,
                    format!("NextStage InternalPrReview: {}", decision.summary),
                ))
            }
            AnalystDecisionNextStage::FinalConfirm => {
                let updated = self.complete_attempt_after_final_rework(attempt).await?;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Completed,
                    format!("NextStage FinalConfirm: {}", decision.summary),
                ))
            }
            AnalystDecisionNextStage::HumanGate => {
                let updated = self.store.update_attempt_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingAttemptStatus::Blocked,
                )?;
                let reason_code = decision
                    .human_gate
                    .as_ref()
                    .and_then(|gate| gate.reason_code.clone())
                    .unwrap_or_else(|| "analyst_human_gate".to_string());
                let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
                    attempt_id: attempt.id.clone(),
                    stage: CodingExecutionStage::Rework,
                    node_id: Some(node_id.to_string()),
                    role: Some(CodingProviderRole::Analyst),
                    title: "Analyst human gate".to_string(),
                    description: decision.reason.clone(),
                    reason_code: Some(reason_code),
                    evidence_refs: decision.evidence_refs.clone(),
                    raw_provider_output_ref: decision.raw_provider_output_refs.first().cloned(),
                    available_actions: analyst_human_gate_actions(decision.human_gate.as_ref()),
                })?;
                let _ = self
                    .event_tx
                    .send(CodingWsOutMessage::CodingGateRequired { gate })
                    .await;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Blocked,
                    format!("HumanGate: {}", decision.summary),
                ))
            }
        }
    }

    async fn emit_tester_tool_result_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        sequence: &mut usize,
        role_run: Option<&CodingRoleRun>,
        result: ProviderToolResult,
    ) {
        let metadata = role_run.map(|role_run| {
            serde_json::json!({
                "tool_use_id": result.tool_use_id.clone(),
                "role_run_id": role_run.id.clone(),
                "run_no": role_run.run_no
            })
        });
        let entry = tester_chat_entry(
            attempt,
            node_id,
            sequence,
            CodingEntryType::ToolResult {
                tool_use_id: result.tool_use_id,
                output: result.output,
                is_error: result.is_error,
            },
            None,
            metadata,
        );
        self.save_and_emit_chat_entry(entry).await;
    }
}

fn worktree_path_for_attempt(repo_path: &Path, attempt: &CodingExecutionAttempt) -> PathBuf {
    if let Some(issue_id) = attempt.branch_name.strip_prefix("aria/issues/") {
        return repo_path
            .join(".worktrees")
            .join("aria-issues")
            .join(issue_id);
    }
    repo_path
        .join(".worktrees")
        .join("aria-work-items")
        .join(&attempt.work_item_id)
        .join(format!("attempt-{}", attempt.attempt_no))
}

fn provider_type_for_name(provider: &ProviderName) -> ProviderType {
    match provider {
        ProviderName::ClaudeCode => ProviderType::ClaudeCode,
        ProviderName::Codex => ProviderType::Codex,
        ProviderName::Fake => ProviderType::Fake,
    }
}

fn build_coding_prompt(
    attempt: &CodingExecutionAttempt,
    context: &CodingExecutionContext,
    rework_instruction: Option<&CodingReworkInstruction>,
    context_notes: Option<&ReworkContextNoteInput>,
) -> String {
    let mut prompt = format!(
        "Coding Workspace\n\
         你是 Coding Workspace author。请在指定 worktree 中完成真实代码修改和测试，不要只输出计划或 Story/Design/Work Item 文档。\n\
         Project: {}\n\
         Issue: {}\n\
         Work Item: {}\n\
         Attempt: {}\n\
         Branch: {}\n",
        attempt.project_id, attempt.issue_id, attempt.work_item_id, attempt.id, attempt.branch_name
    );
    if let Some(worktree_path) = attempt.worktree_path.as_ref() {
        prompt.push_str(&format!("Worktree Path: {}\n", worktree_path.display()));
    }
    if !context.verification_commands.is_empty() {
        prompt.push_str("\n验证命令:\n");
        for command in &context.verification_commands {
            prompt.push_str("- ");
            prompt.push_str(command);
            prompt.push('\n');
        }
    }

    if let Some(markdown) = context.work_item_markdown.as_deref() {
        prompt.push_str("\n已确认 Work Item:\n````markdown\n");
        prompt.push_str(markdown.trim());
        prompt.push_str("\n````\n");
    }
    if let Some(instruction) = rework_instruction {
        prompt.push_str("\n上一轮返修要求:\n");
        prompt.push_str(&format!(
            "- 来源阶段: {:?}\n- 摘要: {}\n",
            instruction.source_stage, instruction.summary
        ));
        if !instruction.fix_hints.is_empty() {
            prompt.push_str("- 修复提示:\n");
            for (index, hint) in instruction.fix_hints.iter().enumerate() {
                prompt.push_str(&format!("  {}. {}\n", index + 1, hint));
            }
        }
        if !instruction.questions.is_empty() {
            prompt.push_str("- 待澄清问题:\n");
            for (index, question) in instruction.questions.iter().enumerate() {
                prompt.push_str(&format!("  {}. {}\n", index + 1, question));
            }
        }
        prompt.push_str(
            "\n本轮必须优先修复上述问题。完成前请检查 git diff/status，确认 reviewer 指出的文件或行为已处理。\n",
        );
    }
    append_coding_context_notes(&mut prompt, context_notes);
    prompt.push_str(dependency_bootstrap_guidance());
    prompt.push_str(
        "\n执行要求:\n\
         - 遵循仓库规则和 TDD 流程。\n\
         - 优先按已确认 Work Item 的文件落点、范围和验证命令执行。\n\
         - 完成后报告修改文件、测试命令和结果。\n",
    );
    prompt
}

fn build_coding_delta_prompt(
    attempt: &CodingExecutionAttempt,
    context: &CodingExecutionContext,
    rework_instruction: Option<&CodingReworkInstruction>,
    context_notes: Option<&ReworkContextNoteInput>,
) -> String {
    let mut prompt = format!(
        "Coding Workspace\n\
         你是 Coding Workspace Coder。请继续在指定 worktree 中完成真实代码修改和测试，不要只输出计划。\n\
         Project: {}\n\
         Issue: {}\n\
         Work Item: {}\n\
         Attempt: {}\n\
         Branch: {}\n",
        attempt.project_id, attempt.issue_id, attempt.work_item_id, attempt.id, attempt.branch_name
    );
    if let Some(worktree_path) = attempt.worktree_path.as_ref() {
        prompt.push_str(&format!("Worktree Path: {}\n", worktree_path.display()));
    }
    prompt.push_str(
        "\n这是对当前 provider 会话的增量代码编写指令。不要重新发送或复述完整 Work Item；请基于本会话已有上下文、当前 worktree 状态和以下新增要求，直接继续修改代码。\n",
    );
    if !context.verification_commands.is_empty() {
        prompt.push_str("\n验证命令:\n");
        for command in &context.verification_commands {
            prompt.push_str("- ");
            prompt.push_str(command);
            prompt.push('\n');
        }
    }

    if let Some(instruction) = rework_instruction {
        prompt.push_str("\n本轮返修要求:\n");
        prompt.push_str(&format!(
            "- 来源阶段: {:?}\n- 摘要: {}\n",
            instruction.source_stage, instruction.summary
        ));
        if !instruction.fix_hints.is_empty() {
            prompt.push_str("- 修复提示:\n");
            for (index, hint) in instruction.fix_hints.iter().enumerate() {
                prompt.push_str(&format!("  {}. {}\n", index + 1, hint));
            }
        }
        if !instruction.questions.is_empty() {
            prompt.push_str("- 待澄清问题:\n");
            for (index, question) in instruction.questions.iter().enumerate() {
                prompt.push_str(&format!("  {}. {}\n", index + 1, question));
            }
        }
        prompt.push_str(
            "\n本轮必须优先修复上述问题。完成前请检查 git diff/status，确认 reviewer 指出的文件或行为已处理。\n",
        );
    } else {
        prompt.push_str(
            "\n本轮没有新增返修要求。请基于当前会话和 worktree 状态继续完成未结束的代码编写任务。\n",
        );
    }
    append_coding_context_notes(&mut prompt, context_notes);
    prompt.push_str(dependency_bootstrap_guidance());
    prompt.push_str(
        "\n执行要求:\n\
         - 遵循仓库规则和 TDD 流程。\n\
         - 不要重新生成 Story/Design/Work Item 文档。\n\
         - 完成后报告修改文件、测试命令和结果。\n",
    );
    prompt
}

fn append_coding_context_notes(
    prompt: &mut String,
    context_notes: Option<&ReworkContextNoteInput>,
) {
    let Some(context_notes) = context_notes else {
        return;
    };
    if context_notes.text.trim().is_empty() || context_notes.text.trim() == "无" {
        return;
    }
    prompt.push_str("\n本轮补充上下文:\n");
    prompt.push_str(&format!(
        "ContextNotes Truncated: {}\n{}\n",
        context_notes.truncated, context_notes.text
    ));
    prompt.push_str(
        "请将这些人工补充要求与本轮返修要求一起执行；如有冲突，优先遵循更具体的人工补充上下文。\n",
    );
}

fn dependency_bootstrap_guidance() -> &'static str {
    "\n依赖初始化诊断要求:\n\
     - 如果前端命令出现 `Local package.json exists, but node_modules missing`、`tsc EACCES`、`vitest EACCES`、`Permission denied` 或 `spawn ... EACCES`，先不要判定 pnpm 环境不可用。\n\
     - 先运行 `pnpm --version` 区分 pnpm 是否存在；只有该命令失败时，才报告 pnpm 不可用。\n\
     - 如果 pnpm 可用且对应 package 目录存在 lockfile，请先运行 `pnpm -C <package-dir> install --frozen-lockfile`，例如 Aria 前端为 `pnpm -C web install --frozen-lockfile`，然后重试 build/test。\n\
     - 不要把缺少 node_modules 误判为 pnpm 不可用。\n"
}

fn build_rework_prompt(
    attempt: &CodingExecutionAttempt,
    evidence: &str,
    source_stage: &CodingExecutionStage,
    rework_round: u32,
    context_notes: &ReworkContextNoteInput,
    evaluation_context_json: &str,
    retry_diagnostic: Option<&str>,
) -> String {
    let retry_diagnostic_section = retry_diagnostic
        .map(|summary| format!("\n上一轮 Analyst role run 诊断摘要:\n{}\n", summary))
        .unwrap_or_default();
    format!(
        "CRITICAL: Return ONLY a single JSON object. No markdown, no explanations, no validation reports, no tables.\n\
         Coding Workspace Rework 分析官\n\
         {}\n\
         你是 Coding Workspace Rework 分析官，只做分析和路由决策。\n\
         严格要求：不要修改代码，不要调用 tool_use，不要执行命令。\n\
         仅根据上一阶段 summary/evidence、本轮新增 ContextNote 与 EvaluationContextPack 输出 AnalystDecision JSON。\n\
         JSON 必须以 {{ 开头，以 }} 结尾。\n\
         JSON 格式：{{\"verdict\":\"needs_fix|rerun_testing|proceed|human_required|blocked\",\"next_stage\":\"coding|testing|code_review|review_request|internal_pr_review|final_confirm|human_gate\",\"reason\":\"...\",\"evidence_refs\":[\"...\"],\"raw_provider_output_refs\":[\"...\"],\"rework_instructions\":null,\"human_gate\":null}}\n\
         路由规则：TestingReport 因 test_plan_missing_json、test_plan_invalid_json 或 test_plan_repair_failed 阻塞时，优先判断为 Tester 输出契约问题；若可重试，输出 verdict=rerun_testing,next_stage=testing；只有环境、权限或需求缺失不可自动处理时才 next_stage=human_gate。\n\
         Project: {}\n\
         Issue: {}\n\
         Work Item: {}\n\
         Attempt: {}\n\
         Branch: {}\n\
         Previous Stage: {:?}\n\
         Rework Round: {}\n\
         ContextNotes Truncated: {}\n\
         \n上一阶段 summary/evidence:\n{}\n\
         \n本轮新增 ContextNote:\n{}\n\
         \nEvaluationContextPack:\n````json\n{}\n````\n\
         {}\
         \nCRITICAL: Return ONLY a single JSON object. Do not summarize validation. Do not include markdown.\n\
         END OF INSTRUCTIONS: output JSON only.",
        provider_runtime_contract("Analyst"),
        attempt.project_id,
        attempt.issue_id,
        attempt.work_item_id,
        attempt.id,
        attempt.branch_name,
        source_stage,
        rework_round,
        context_notes.truncated,
        evidence,
        context_notes.text,
        evaluation_context_json,
        retry_diagnostic_section
    )
}

fn provider_runtime_contract(role: &str) -> String {
    format!(
        "[openspec_contract]\n\
         Role: {role}\n\
         - 使用 Story Spec、Design Spec、Work Item 的追踪关系做判断。\n\
         - 发现 Story Spec、Design Spec、Work Item、diff 或实现之间冲突时，必须 blocked 或请求人工澄清。\n\
         - 不得忽略需求、设计、任务之间的证据链。\n\
         \n\
         [superpowers_contract]\n\
         - 先证据后结论。\n\
         - 验证前置；结论必须能追溯到已执行检查或明确证据。\n\
         - 不用未执行推断替代证据。\n"
    )
}

fn provider_prompt_event(
    node_id: &str,
    provider: &ProviderName,
    prompt: String,
    detail: &str,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: format!("{node_id}_prompt"),
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: WsExecutionEventKind::Output,
        status: WsExecutionEventStatus::Started,
        title: "Provider Prompt".to_string(),
        detail: Some(detail.to_string()),
        command: None,
        cwd: None,
        output: Some(prompt),
        exit_code: None,
    }
}

fn streaming_input_from_adapter(
    input: &AdapterInput,
    working_dir: PathBuf,
) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type: input.provider_type.clone(),
        role: input.role.clone(),
        prompt: input.prompt.clone(),
        working_dir,
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Supervised,
        env_vars: BTreeMap::new(),
        timeout_secs: input.timeout,
    }
}

fn build_tester_execute_plan_prompt(
    attempt: &CodingExecutionAttempt,
    plan: &TestPlan,
    evaluation_context_json: &str,
) -> String {
    let plan_json = serde_json::to_string_pretty(plan).unwrap_or_else(|_| "{}".to_string());
    format!(
        "Tester Provider Runtime\n\
         Phase: execute_test_plan\n\
         Attempt: {}\n\
         Work Item: {}\n\
         \n\
         Execute the following TestPlan. You may execute commands or inspect files yourself.\n\
         Every required TestPlan step must have exactly one corresponding step_results item.\n\
         If you cannot run a required step, emit status=\"blocked\" or status=\"skipped\" with provider_analysis explaining why.\n\
         Do not claim overall success in prose without step_results JSON.\n\
         Tool calls meant to satisfy a plan step must include the exact step_id in their input. Tool calls without step_id are unplanned evidence and cannot satisfy required steps.\n\
         At the end of execute_test_plan, output a JSON object with:\n\
         {{\"step_results\":[{{\"step_id\":\"...\",\"status\":\"passed|failed|blocked|skipped\",\"evidence_refs\":[\"...\"],\"provider_analysis\":\"...\"}}]}}\n\
         \n\
         TestPlan:\n```json\n{}\n```\n\
         \n\
         Evaluation Context JSON:\n```json\n{}\n```\n",
        attempt.id, attempt.work_item_id, plan_json, evaluation_context_json
    )
}

fn record_tester_step_result(
    plan: &TestPlan,
    call: &ProviderToolCall,
    command_result: Option<TestCommand>,
    result: &ProviderToolResult,
    outputs: TesterStepResultOutputs<'_>,
) {
    let Some(step_id) = tool_call_step_id(call) else {
        outputs
            .unplanned_evidence
            .push(unplanned_evidence_from_tool(
                call,
                command_result.as_ref(),
                result,
            ));
        if let Some(command) = command_result {
            outputs.unplanned_commands.push(command);
        }
        return;
    };

    if !plan.steps.iter().any(|step| step.id == step_id) {
        outputs
            .unplanned_evidence
            .push(unplanned_evidence_from_tool(
                call,
                command_result.as_ref(),
                result,
            ));
        if let Some(command) = command_result {
            outputs.unplanned_commands.push(command);
        }
        push_unique_warning(
            outputs.context_warnings,
            format!("unknown_step_id:{step_id}"),
        );
        return;
    }

    let status = command_result
        .as_ref()
        .map(|command| command.status.clone())
        .unwrap_or_else(|| {
            if result.is_error {
                TestCommandStatus::Failed
            } else {
                TestCommandStatus::Passed
            }
        });
    let mut evidence_refs = command_result
        .as_ref()
        .map(|command| {
            [command.stdout_ref.clone(), command.stderr_ref.clone()]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if evidence_refs.is_empty() {
        evidence_refs.push(format!("tool_result:{}", result.tool_use_id));
    }
    let command = command_result
        .as_ref()
        .map(|command| command.command.clone());
    let provider_analysis = (!result.output.trim().is_empty()).then(|| result.output.clone());

    if let Some(existing) = outputs
        .step_results
        .iter_mut()
        .find(|existing| existing.step_id == step_id)
    {
        if existing.status == TestCommandStatus::Passed && status != TestCommandStatus::Passed {
            existing.status = status;
        }
        for evidence_ref in evidence_refs {
            if !existing
                .evidence_refs
                .iter()
                .any(|value| value == &evidence_ref)
            {
                existing.evidence_refs.push(evidence_ref);
            }
        }
        if existing.command.is_none() {
            existing.command = command;
        }
        if let Some(provider_analysis) = provider_analysis {
            existing.provider_analysis = Some(match existing.provider_analysis.take() {
                Some(existing_analysis) => format!("{existing_analysis}\n{provider_analysis}"),
                None => provider_analysis,
            });
        }
        return;
    }

    outputs.step_results.push(TestingStepResult {
        step_id,
        status,
        evidence_refs,
        command,
        provider_analysis,
    });
}

struct TesterStepResultOutputs<'a> {
    step_results: &'a mut Vec<TestingStepResult>,
    unplanned_commands: &'a mut Vec<TestCommand>,
    unplanned_evidence: &'a mut Vec<TestingUnplannedEvidence>,
    context_warnings: &'a mut Vec<String>,
}

fn unplanned_evidence_from_tool(
    call: &ProviderToolCall,
    command_result: Option<&TestCommand>,
    result: &ProviderToolResult,
) -> TestingUnplannedEvidence {
    let status = command_result
        .map(|command| command.status.clone())
        .unwrap_or_else(|| {
            if result.is_error {
                TestCommandStatus::Failed
            } else {
                TestCommandStatus::Passed
            }
        });
    let mut evidence_refs = command_result
        .map(|command| {
            [command.stdout_ref.clone(), command.stderr_ref.clone()]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if evidence_refs.is_empty() {
        evidence_refs.push(format!("tool_result:{}", result.tool_use_id));
    }
    TestingUnplannedEvidence {
        tool_use_id: result.tool_use_id.clone(),
        tool_name: call.tool_name.clone(),
        status,
        evidence_refs,
        provider_analysis: (!result.output.trim().is_empty()).then(|| result.output.clone()),
    }
}

fn tool_call_step_id(call: &ProviderToolCall) -> Option<String> {
    call.input
        .get("step_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn high_risk_test_step_block_reason(
    plan: &TestPlan,
    call: &ProviderToolCall,
) -> Option<&'static str> {
    let step_id = tool_call_step_id(call)?;
    let step = plan.steps.iter().find(|step| step.id == step_id)?;
    (step.required && step.risk_level == TestPlanRiskLevel::High)
        .then_some("high_risk_test_step_requires_permission")
}

/// 区分 `skipped_required_steps`（步骤被阻塞/跳过）与 `missing_required_steps`
/// （步骤缺失），避免归因错误误导排查。显式 reason_code（如高风险权限）优先。
fn derive_testing_blocked_reason_code(
    explicit_reason_code: Option<String>,
    report: &TestingReport,
) -> String {
    if let Some(reason_code) = explicit_reason_code {
        return reason_code;
    }
    if !report.missing_required_steps.is_empty() {
        return "missing_required_steps".to_string();
    }
    if !report.skipped_required_steps.is_empty() {
        return "skipped_required_steps".to_string();
    }
    "testing_blocked".to_string()
}

#[derive(Debug, Deserialize)]
struct ProviderTestingStepResultsPayload {
    #[serde(default)]
    step_results: Vec<TestingStepResult>,
}

fn parse_testing_step_results_from_provider_output(output: &str) -> Vec<TestingStepResult> {
    let Some(json) = extract_json_object(output) else {
        return Vec::new();
    };
    serde_json::from_str::<ProviderTestingStepResultsPayload>(json)
        .map(|payload| payload.step_results)
        .unwrap_or_default()
}

pub fn testing_report_has_execution_evidence(report: &TestingReport) -> bool {
    (!report.steps.is_empty() && report.plan_id.is_some())
        || !report.commands.is_empty()
        || report
            .steps
            .iter()
            .any(|step| !step.evidence_refs.is_empty() || step.command.is_some())
        || report
            .unplanned_commands
            .iter()
            .any(|command| !command.stdout_ref.is_empty() || !command.stderr_ref.is_empty())
}

pub fn testing_report_should_enter_analyst(report: &TestingReport) -> bool {
    match report.overall_status {
        TestingOverallStatus::Failed
        | TestingOverallStatus::Blocked
        | TestingOverallStatus::SkippedByUserDecision
        | TestingOverallStatus::Passed
        | TestingOverallStatus::PassedWithWarnings => true,
    }
}

fn testing_blocked_report_needs_gate(report: &TestingReport, reason_code: &str) -> bool {
    !testing_report_should_enter_analyst(report)
        || matches!(
            reason_code,
            "plan_tests_timeout" | "execute_test_plan_timeout"
        )
}

fn push_unique_warning(warnings: &mut Vec<String>, warning: String) {
    if !warnings.iter().any(|existing| existing == &warning) {
        warnings.push(warning);
    }
}

fn testing_blocked_gate_actions() -> Vec<CodingGateAction> {
    vec![
        CodingGateAction {
            action_id: "retry_test_plan".to_string(),
            label: "重试测试计划".to_string(),
            action_type: CodingGateActionType::RetryTestPlan,
        },
        CodingGateAction {
            action_id: "rerun_missing_steps".to_string(),
            label: "补跑缺失步骤".to_string(),
            action_type: CodingGateActionType::RerunMissingSteps,
        },
        CodingGateAction {
            action_id: "provide_context".to_string(),
            label: "补充上下文".to_string(),
            action_type: CodingGateActionType::ProvideContext,
        },
        CodingGateAction {
            action_id: "manual_continue".to_string(),
            label: "人工继续".to_string(),
            action_type: CodingGateActionType::ManualContinue,
        },
        CodingGateAction {
            action_id: "abort".to_string(),
            label: "终止".to_string(),
            action_type: CodingGateActionType::Abort,
        },
    ]
}

fn testing_result_review_gate_actions() -> Vec<CodingGateAction> {
    vec![
        CodingGateAction {
            action_id: "accept_testing_result".to_string(),
            label: "结果可用，进入 Analyst".to_string(),
            action_type: CodingGateActionType::AcceptTestingResult,
        },
        CodingGateAction {
            action_id: "rerun_testing".to_string(),
            label: "不满意，重新测试".to_string(),
            action_type: CodingGateActionType::RerunTesting,
        },
        CodingGateAction {
            action_id: "abort".to_string(),
            label: "终止".to_string(),
            action_type: CodingGateActionType::Abort,
        },
    ]
}

fn testing_result_review_description(report: &TestingReport) -> String {
    let status = match report.overall_status {
        TestingOverallStatus::Passed => "测试通过",
        TestingOverallStatus::PassedWithWarnings => "测试通过但有警告",
        TestingOverallStatus::Failed => "测试失败",
        TestingOverallStatus::SkippedByUserDecision => "测试由用户决策跳过",
        TestingOverallStatus::Blocked => "测试被阻塞",
    };
    match report.plan_summary.as_deref() {
        Some(summary) if !summary.trim().is_empty() => {
            format!(
                "Tester 已完成测试报告 {}（{}）：{}。请确认是否进入 Analyst 或重新测试。",
                report.id,
                status,
                summary.trim()
            )
        }
        _ => format!(
            "Tester 已完成测试报告 {}（{}）。请确认是否进入 Analyst 或重新测试。",
            report.id, status
        ),
    }
}

fn testing_report_to_analyst_evidence(report: &TestingReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| {
        format!(
            "TestingReport serialization failed; overall_status={:?}",
            report.overall_status
        )
    })
}

fn rework_instruction_fields_from_analyst_record(
    decision: &AnalystDecisionRecord,
) -> (String, Vec<String>) {
    if let Some(instructions) = &decision.rework_instructions {
        let mut fix_hints = instructions
            .required_changes
            .iter()
            .chain(instructions.verification_expectations.iter())
            .cloned()
            .collect::<Vec<_>>();
        if fix_hints.is_empty() {
            fix_hints.push(decision.reason.clone());
        }
        return (instructions.summary.clone(), fix_hints);
    }
    (decision.reason.clone(), vec![decision.reason.clone()])
}

fn analyst_human_gate_actions(
    recommendation: Option<&AnalystHumanGateRecommendation>,
) -> Vec<CodingGateAction> {
    let mut actions = Vec::new();
    if let Some(recommendation) = recommendation {
        for action_id in &recommendation.available_actions {
            if let Some(action) = coding_gate_action_for_id(action_id)
                && !actions
                    .iter()
                    .any(|existing: &CodingGateAction| existing.action_id == action.action_id)
            {
                actions.push(action);
            }
        }
    }
    if actions.is_empty() {
        actions.push(coding_gate_action_for_id("retry_analyst").expect("retry analyst action"));
        actions.push(coding_gate_action_for_id("provide_context").expect("provide context action"));
        actions.push(coding_gate_action_for_id("manual_continue").expect("manual continue action"));
        actions.push(coding_gate_action_for_id("abort").expect("abort action"));
    }
    actions
}

fn coding_gate_action_for_id(action_id: &str) -> Option<CodingGateAction> {
    match action_id {
        "provide_context" => Some(CodingGateAction {
            action_id: "provide_context".to_string(),
            label: "补充上下文".to_string(),
            action_type: CodingGateActionType::ProvideContext,
        }),
        "continue_rework" => Some(CodingGateAction {
            action_id: "continue_rework".to_string(),
            label: "继续返修".to_string(),
            action_type: CodingGateActionType::ContinueRework,
        }),
        "manual_continue" => Some(CodingGateAction {
            action_id: "manual_continue".to_string(),
            label: "人工继续".to_string(),
            action_type: CodingGateActionType::ManualContinue,
        }),
        "accept_risk" => Some(CodingGateAction {
            action_id: "accept_risk".to_string(),
            label: "接受风险".to_string(),
            action_type: CodingGateActionType::AcceptRisk,
        }),
        "retry_test_plan" => Some(CodingGateAction {
            action_id: "retry_test_plan".to_string(),
            label: "重试测试计划".to_string(),
            action_type: CodingGateActionType::RetryTestPlan,
        }),
        "rerun_missing_steps" => Some(CodingGateAction {
            action_id: "rerun_missing_steps".to_string(),
            label: "补跑缺失步骤".to_string(),
            action_type: CodingGateActionType::RerunMissingSteps,
        }),
        "retry_review" => Some(CodingGateAction {
            action_id: "retry_review".to_string(),
            label: "重试审查".to_string(),
            action_type: CodingGateActionType::RetryReview,
        }),
        "retry_analyst" => Some(CodingGateAction {
            action_id: "retry_analyst".to_string(),
            label: "重试 Analyst".to_string(),
            action_type: CodingGateActionType::RetryAnalyst,
        }),
        "retry_internal_review" => Some(CodingGateAction {
            action_id: "retry_internal_review".to_string(),
            label: "重试 Internal Review".to_string(),
            action_type: CodingGateActionType::RetryInternalReview,
        }),
        "send_raw_output_to_analyst" => Some(CodingGateAction {
            action_id: "send_raw_output_to_analyst".to_string(),
            label: "转交分析官".to_string(),
            action_type: CodingGateActionType::SendRawOutputToAnalyst,
        }),
        "accept_testing_result" => Some(CodingGateAction {
            action_id: "accept_testing_result".to_string(),
            label: "结果可用，进入 Analyst".to_string(),
            action_type: CodingGateActionType::AcceptTestingResult,
        }),
        "rerun_testing" => Some(CodingGateAction {
            action_id: "rerun_testing".to_string(),
            label: "不满意，重新测试".to_string(),
            action_type: CodingGateActionType::RerunTesting,
        }),
        "abort" => Some(CodingGateAction {
            action_id: "abort".to_string(),
            label: "终止".to_string(),
            action_type: CodingGateActionType::Abort,
        }),
        _ => None,
    }
}

fn provider_start_is_not_implemented(error: &ProviderAdapterError) -> bool {
    error.stderr == "streaming provider start is not implemented"
}

fn ws_event_from_provider_execution(
    event: ProviderExecutionEvent,
    node_id: &str,
    provider: &ProviderName,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: event.event_id,
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: ws_execution_event_kind(event.kind),
        status: ws_execution_event_status(event.status),
        title: event.title,
        detail: event.detail,
        command: event.command,
        cwd: event.cwd,
        output: event.output,
        exit_code: event.exit_code,
    }
}

fn ws_event_from_tool_call(
    node_id: &str,
    provider: &ProviderName,
    call: ProviderToolCall,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: call.id,
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: WsExecutionEventKind::Command,
        status: WsExecutionEventStatus::Started,
        title: call.tool_name,
        detail: Some(format_tool_call_input(&call.input)),
        command: extract_tool_command(&call.input),
        cwd: None,
        output: None,
        exit_code: None,
    }
}

fn ws_event_from_tool_result(
    node_id: &str,
    provider: &ProviderName,
    title: &str,
    command: Option<String>,
    result: ProviderToolResult,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: result.tool_use_id,
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: WsExecutionEventKind::Command,
        status: if result.is_error {
            WsExecutionEventStatus::Failed
        } else {
            WsExecutionEventStatus::Completed
        },
        title: title.to_string(),
        detail: None,
        command,
        cwd: None,
        output: Some(result.output),
        exit_code: if result.is_error { Some(1) } else { Some(0) },
    }
}

fn ws_event_from_permission_request(
    node_id: &str,
    provider: &ProviderName,
    request: &PermissionRequestData,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: format!("permission_{}", request.id),
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: WsExecutionEventKind::Command,
        status: WsExecutionEventStatus::WaitingApproval,
        title: "Waiting for permission".to_string(),
        detail: Some(request.description.clone()),
        command: Some(request.tool_name.clone()),
        cwd: None,
        output: None,
        exit_code: None,
    }
}

fn ws_event_from_choice_request(
    node_id: &str,
    provider: &ProviderName,
    request: &ChoiceRequestData,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: format!("choice_{}", request.id),
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: WsExecutionEventKind::Provider,
        status: WsExecutionEventStatus::WaitingApproval,
        title: "Waiting for choice".to_string(),
        detail: Some(request.prompt.clone()),
        command: None,
        cwd: None,
        output: None,
        exit_code: None,
    }
}

fn ws_event_from_provider_status(
    node_id: &str,
    provider: &ProviderName,
    status: ProviderStatus,
) -> WsExecutionEvent {
    let status_text = provider_status_text(&status);
    WsExecutionEvent {
        event_id: format!("{node_id}_provider_status_{status_text}"),
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: WsExecutionEventKind::Provider,
        status: ws_status_from_provider_status(status),
        title: format!("Provider {status_text}"),
        detail: None,
        command: None,
        cwd: None,
        output: None,
        exit_code: None,
    }
}

fn ws_execution_event_kind(kind: ProviderExecutionEventKind) -> WsExecutionEventKind {
    match kind {
        ProviderExecutionEventKind::Provider => WsExecutionEventKind::Provider,
        ProviderExecutionEventKind::Turn => WsExecutionEventKind::Turn,
        ProviderExecutionEventKind::Command => WsExecutionEventKind::Command,
        ProviderExecutionEventKind::Output => WsExecutionEventKind::Output,
        ProviderExecutionEventKind::Artifact => WsExecutionEventKind::Artifact,
    }
}

fn ws_execution_event_status(status: ProviderExecutionEventStatus) -> WsExecutionEventStatus {
    match status {
        ProviderExecutionEventStatus::Started => WsExecutionEventStatus::Started,
        ProviderExecutionEventStatus::Running => WsExecutionEventStatus::Running,
        ProviderExecutionEventStatus::WaitingApproval => WsExecutionEventStatus::WaitingApproval,
        ProviderExecutionEventStatus::Completed => WsExecutionEventStatus::Completed,
        ProviderExecutionEventStatus::Failed => WsExecutionEventStatus::Failed,
        ProviderExecutionEventStatus::Aborted => WsExecutionEventStatus::Aborted,
    }
}

fn ws_status_from_provider_status(status: ProviderStatus) -> WsExecutionEventStatus {
    match status {
        ProviderStatus::Starting => WsExecutionEventStatus::Started,
        ProviderStatus::Running => WsExecutionEventStatus::Running,
        ProviderStatus::WaitingApproval => WsExecutionEventStatus::WaitingApproval,
        ProviderStatus::Completed => WsExecutionEventStatus::Completed,
        ProviderStatus::Failed => WsExecutionEventStatus::Failed,
        ProviderStatus::Aborted => WsExecutionEventStatus::Aborted,
    }
}

fn provider_status_text(status: &ProviderStatus) -> &'static str {
    match status {
        ProviderStatus::Starting => "starting",
        ProviderStatus::Running => "running",
        ProviderStatus::WaitingApproval => "waiting_approval",
        ProviderStatus::Completed => "completed",
        ProviderStatus::Failed => "failed",
        ProviderStatus::Aborted => "aborted",
    }
}

fn ws_permission_risk_level(risk_level: RiskLevel) -> WsPermissionRiskLevel {
    match risk_level {
        RiskLevel::Low => WsPermissionRiskLevel::Low,
        RiskLevel::Medium => WsPermissionRiskLevel::Medium,
        RiskLevel::High => WsPermissionRiskLevel::High,
    }
}

fn format_tool_call_input(input: &serde_json::Value) -> String {
    serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string())
}

async fn forward_runner_command_to_provider(
    command: CodingRunnerCommand,
    provider_commands: &mpsc::Sender<ProviderCommand>,
) -> bool {
    match command {
        CodingRunnerCommand::PermissionResponse {
            id,
            approved,
            reason,
        } => provider_commands
            .send(ProviderCommand::PermissionResponse {
                id,
                approved,
                reason,
            })
            .await
            .is_ok(),
        CodingRunnerCommand::ChoiceResponse {
            id,
            selected_option_ids,
            free_text,
        } => provider_commands
            .send(ProviderCommand::ChoiceResponse {
                id,
                selected_option_ids,
                free_text,
            })
            .await
            .is_ok(),
        CodingRunnerCommand::AbortAttempt => {
            provider_commands.send(ProviderCommand::Abort).await.is_ok()
        }
        CodingRunnerCommand::ProviderSelect { .. }
        | CodingRunnerCommand::StageGateConfirm { .. } => true,
    }
}

fn extract_tool_command(input: &serde_json::Value) -> Option<String> {
    let command = input.get("command").or_else(|| input.get("cmd"))?;
    if let Some(command) = command.as_str() {
        return Some(command.to_string());
    }
    command.as_array().and_then(|parts| {
        parts
            .iter()
            .map(serde_json::Value::as_str)
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join(" "))
            .filter(|command| !command.trim().is_empty())
    })
}

fn tester_chat_entry(
    attempt: &CodingExecutionAttempt,
    node_id: &str,
    sequence: &mut usize,
    entry_type: CodingEntryType,
    content: Option<String>,
    metadata: Option<serde_json::Value>,
) -> CodingChatEntry {
    let entry = CodingChatEntry {
        id: format!("coding_chat_entry_{:04}", *sequence),
        attempt_id: attempt.id.clone(),
        node_id: Some(node_id.to_string()),
        role: CodingAgentRole::Tester,
        entry_type,
        content,
        metadata,
        created_at: Utc::now().to_rfc3339(),
    };
    *sequence += 1;
    entry
}

fn bind_test_plan_role_run(plan: &mut TestPlan, role_run: &CodingRoleRun) {
    plan.role_run_id = Some(role_run.id.clone());
    plan.run_no = Some(role_run.run_no);
}

fn bind_testing_report_role_run(report: &mut TestingReport, role_run: &CodingRoleRun) {
    report.role_run_id = Some(role_run.id.clone());
    report.run_no = Some(role_run.run_no);
}

fn testing_role_run_status(report: &TestingReport) -> CodingRoleRunStatus {
    match report.overall_status {
        TestingOverallStatus::Passed | TestingOverallStatus::PassedWithWarnings => {
            CodingRoleRunStatus::Completed
        }
        TestingOverallStatus::Failed => CodingRoleRunStatus::Failed,
        TestingOverallStatus::Blocked => CodingRoleRunStatus::Blocked,
        TestingOverallStatus::SkippedByUserDecision => CodingRoleRunStatus::Completed,
    }
}

fn derive_testing_role_run_reason(report: &TestingReport) -> Option<String> {
    report
        .context_warnings
        .iter()
        .find(|warning| warning.contains("provider_start_failed") || warning.contains("timeout"))
        .cloned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReworkContextNoteInput {
    text: String,
    truncated: bool,
}

fn format_rework_context_notes(
    notes: &[CodingContextNote],
    limit: usize,
) -> ReworkContextNoteInput {
    if notes.is_empty() {
        return ReworkContextNoteInput {
            text: "无".to_string(),
            truncated: false,
        };
    }
    let blocks = notes
        .iter()
        .map(|note| {
            format!(
                "- ContextNote {} ({})\n{}",
                note.id,
                note.created_at,
                note.content.trim()
            )
        })
        .collect::<Vec<_>>();
    let mut remaining = limit;
    let mut selected = Vec::new();
    let mut truncated = false;

    for block in blocks.iter().rev() {
        let block_len = block.chars().count();
        if block_len <= remaining {
            selected.push(block.clone());
            remaining -= block_len;
            continue;
        }

        truncated = true;
        let marker = "[...已截断最早 ContextNote...]\n";
        let marker_len = marker.chars().count();
        if remaining > marker_len {
            let partial = take_last_chars(block, remaining - marker_len);
            selected.push(format!("{marker}{partial}"));
        }
        break;
    }

    if selected.len() < blocks.len() {
        truncated = true;
    }
    selected.reverse();
    let mut text = selected.join("\n");
    if text.chars().count() > limit {
        text = take_last_chars(&text, limit);
        truncated = true;
    }

    ReworkContextNoteInput { text, truncated }
}

fn take_last_chars(value: &str, limit: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let start = chars.len().saturating_sub(limit);
    chars[start..].iter().collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AnalystDecision {
    verdict: AnalystVerdict,
    structured_verdict: AnalystDecisionVerdict,
    next_stage: Option<AnalystDecisionNextStage>,
    summary: String,
    reason: String,
    evidence_refs: Vec<String>,
    raw_provider_output_refs: Vec<String>,
    rework_instructions: Option<AnalystReworkInstructions>,
    human_gate: Option<AnalystHumanGateRecommendation>,
    fix_hints: Vec<String>,
    questions: Vec<String>,
    parse_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnalystProviderPayload {
    verdict: AnalystProviderVerdict,
    #[serde(default)]
    next_stage: Option<AnalystDecisionNextStage>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    evidence_refs: Vec<String>,
    #[serde(default)]
    raw_provider_output_refs: Vec<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_analyst_rework_instructions"
    )]
    rework_instructions: Option<AnalystReworkInstructions>,
    #[serde(default)]
    human_gate: Option<AnalystHumanGateRecommendation>,
    #[serde(default)]
    fix_hints: Vec<String>,
    #[serde(default)]
    questions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AnalystReworkInstructionsInput {
    Structured(AnalystReworkInstructions),
    Summary(String),
}

fn deserialize_optional_analyst_rework_instructions<'de, D>(
    deserializer: D,
) -> Result<Option<AnalystReworkInstructions>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(input) = Option::<AnalystReworkInstructionsInput>::deserialize(deserializer)? else {
        return Ok(None);
    };
    match input {
        AnalystReworkInstructionsInput::Structured(instructions) => Ok(Some(instructions)),
        AnalystReworkInstructionsInput::Summary(summary) => {
            let summary = summary.trim();
            if summary.is_empty() {
                Ok(None)
            } else {
                Ok(Some(AnalystReworkInstructions {
                    summary: summary.to_string(),
                    required_changes: vec![summary.to_string()],
                    verification_expectations: Vec::new(),
                }))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AnalystProviderVerdict {
    NeedsFix,
    NeedsHumanInput,
    NoIssue,
    RerunTesting,
    Proceed,
    HumanRequired,
    Blocked,
}

impl AnalystProviderVerdict {
    fn structured(&self) -> AnalystDecisionVerdict {
        match self {
            Self::NeedsFix => AnalystDecisionVerdict::NeedsFix,
            Self::NeedsHumanInput => AnalystDecisionVerdict::HumanRequired,
            Self::NoIssue => AnalystDecisionVerdict::Proceed,
            Self::RerunTesting => AnalystDecisionVerdict::RerunTesting,
            Self::Proceed => AnalystDecisionVerdict::Proceed,
            Self::HumanRequired => AnalystDecisionVerdict::HumanRequired,
            Self::Blocked => AnalystDecisionVerdict::Blocked,
        }
    }
}

fn default_next_stage_for_legacy_verdict(
    verdict: &AnalystDecisionVerdict,
    source_stage: &CodingExecutionStage,
) -> AnalystDecisionNextStage {
    match verdict {
        AnalystDecisionVerdict::NeedsFix => AnalystDecisionNextStage::Coding,
        AnalystDecisionVerdict::RerunTesting => AnalystDecisionNextStage::Testing,
        AnalystDecisionVerdict::HumanRequired | AnalystDecisionVerdict::Blocked => {
            AnalystDecisionNextStage::HumanGate
        }
        AnalystDecisionVerdict::Proceed => match source_stage {
            CodingExecutionStage::Testing => AnalystDecisionNextStage::CodeReview,
            CodingExecutionStage::CodeReview => AnalystDecisionNextStage::ReviewRequest,
            CodingExecutionStage::InternalPrReview => AnalystDecisionNextStage::FinalConfirm,
            _ => AnalystDecisionNextStage::CodeReview,
        },
    }
}

fn decision_reason(summary: &str, reason: Option<&str>) -> String {
    reason
        .and_then(non_empty_trimmed)
        .unwrap_or_else(|| summary.to_string())
}

fn parse_analyst_verdict(
    full_output: &str,
    source_stage: &CodingExecutionStage,
) -> AnalystDecision {
    let Some(json_text) = extract_json_object(full_output) else {
        let summary = "Analyst 输出不是有效 JSON，已转人工确认。".to_string();
        return AnalystDecision {
            verdict: AnalystVerdict::NeedsHumanInput,
            structured_verdict: AnalystDecisionVerdict::HumanRequired,
            next_stage: Some(AnalystDecisionNextStage::HumanGate),
            summary: summary.clone(),
            reason: summary,
            evidence_refs: Vec::new(),
            raw_provider_output_refs: Vec::new(),
            rework_instructions: None,
            human_gate: None,
            fix_hints: Vec::new(),
            questions: vec!["请人工确认 Analyst 输出并补充下一步处理意见。".to_string()],
            parse_error: Some("missing_json_object".to_string()),
        };
    };

    match serde_json::from_str::<AnalystProviderPayload>(json_text) {
        Ok(payload) => {
            let structured_verdict = payload.verdict.structured();
            let summary = payload
                .summary
                .as_deref()
                .and_then(non_empty_trimmed)
                .or_else(|| {
                    payload
                        .rework_instructions
                        .as_ref()
                        .and_then(|instruction| non_empty_trimmed(&instruction.summary))
                })
                .unwrap_or_else(|| default_analyst_decision_summary(&structured_verdict));
            let next_stage = payload.next_stage.unwrap_or_else(|| {
                default_next_stage_for_legacy_verdict(&structured_verdict, source_stage)
            });
            let reason = decision_reason(&summary, payload.reason.as_deref());
            AnalystDecision {
                verdict: structured_verdict.legacy_chat_verdict(),
                structured_verdict,
                next_stage: Some(next_stage),
                summary,
                reason,
                evidence_refs: payload.evidence_refs,
                raw_provider_output_refs: payload.raw_provider_output_refs,
                rework_instructions: payload.rework_instructions,
                human_gate: payload.human_gate,
                fix_hints: payload.fix_hints,
                questions: payload.questions,
                parse_error: None,
            }
        }
        Err(error) => {
            let summary = "Analyst 输出不是有效 JSON，已转人工确认。".to_string();
            AnalystDecision {
                verdict: AnalystVerdict::NeedsHumanInput,
                structured_verdict: AnalystDecisionVerdict::HumanRequired,
                next_stage: Some(AnalystDecisionNextStage::HumanGate),
                summary: summary.clone(),
                reason: summary,
                evidence_refs: Vec::new(),
                raw_provider_output_refs: Vec::new(),
                rework_instructions: None,
                human_gate: None,
                fix_hints: Vec::new(),
                questions: vec!["请人工确认 Analyst 输出并补充下一步处理意见。".to_string()],
                parse_error: Some(error.to_string()),
            }
        }
    }
}

fn extract_json_object(value: &str) -> Option<&str> {
    let start = value.find('{')?;
    let end = value.rfind('}')?;
    (start <= end).then(|| &value[start..=end])
}

fn default_analyst_decision_summary(verdict: &AnalystDecisionVerdict) -> String {
    match verdict {
        AnalystDecisionVerdict::NeedsFix => "Analyst 判定需要自动修复".to_string(),
        AnalystDecisionVerdict::RerunTesting => "Analyst 判定需要重跑测试".to_string(),
        AnalystDecisionVerdict::Proceed => "Analyst 未发现阻塞问题".to_string(),
        AnalystDecisionVerdict::HumanRequired => "Analyst 判定需要人工补充信息".to_string(),
        AnalystDecisionVerdict::Blocked => "Analyst 判定当前流程被阻塞".to_string(),
    }
}

#[derive(Debug)]
struct CodeReviewProviderPayload {
    verdict: ReviewVerdict,
    summary: String,
    findings: Vec<ReviewFinding>,
    impact_scope: Vec<String>,
    pr_description: String,
    commit_message_suggestion: String,
    tested_evidence_refs: Vec<String>,
    diff_refs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawCodeReviewProviderPayload {
    verdict: ReviewVerdict,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    findings: Vec<RawReviewFinding>,
    #[serde(default)]
    impact_scope: Vec<String>,
    #[serde(default)]
    pr_description: String,
    #[serde(default)]
    commit_message_suggestion: String,
    #[serde(default)]
    tested_evidence_refs: Vec<String>,
    #[serde(default)]
    diff_refs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawReviewFinding {
    #[serde(default)]
    severity: Option<crate::product::coding_models::FindingSeverity>,
    #[serde(default, alias = "file")]
    file_path: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default, alias = "description", alias = "failure_scenario")]
    message: Option<String>,
    #[serde(default, alias = "recommendation", alias = "fix")]
    required_action: Option<String>,
    #[serde(default)]
    source_stage: Option<CodingExecutionStage>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    evidence: Vec<String>,
    #[serde(default)]
    related_requirements: Vec<String>,
    #[serde(default)]
    related_design_constraints: Vec<String>,
    #[serde(default)]
    related_work_item_tasks: Vec<String>,
}

fn parse_review_payload(
    full_output: &str,
    default_source_stage: CodingExecutionStage,
) -> CodeReviewProviderPayload {
    let json = extract_json_object(full_output).unwrap_or(full_output);
    match serde_json::from_str::<RawCodeReviewProviderPayload>(json) {
        Ok(raw) => raw.into_payload(default_source_stage),
        Err(_) => blocked_review_payload(full_output),
    }
}

impl RawCodeReviewProviderPayload {
    fn into_payload(self, default_source_stage: CodingExecutionStage) -> CodeReviewProviderPayload {
        let verdict = self.verdict;
        CodeReviewProviderPayload {
            summary: non_empty_trimmed(&self.summary)
                .unwrap_or_else(|| default_review_summary(&verdict)),
            verdict,
            findings: self
                .findings
                .into_iter()
                .map(|finding| finding.into_review_finding(default_source_stage.clone()))
                .collect(),
            impact_scope: self.impact_scope,
            pr_description: self.pr_description,
            commit_message_suggestion: self.commit_message_suggestion,
            tested_evidence_refs: self.tested_evidence_refs,
            diff_refs: self.diff_refs,
        }
    }
}

impl RawReviewFinding {
    fn into_review_finding(self, default_source_stage: CodingExecutionStage) -> ReviewFinding {
        ReviewFinding {
            severity: self
                .severity
                .unwrap_or(crate::product::coding_models::FindingSeverity::Warning),
            file_path: self.file_path,
            line: self.line,
            message: self
                .message
                .or(self.title)
                .unwrap_or_else(|| "review finding".to_string()),
            required_action: self.required_action,
            source_stage: self.source_stage.unwrap_or(default_source_stage),
            evidence: self.evidence,
            related_requirements: self.related_requirements,
            related_design_constraints: self.related_design_constraints,
            related_work_item_tasks: self.related_work_item_tasks,
        }
    }
}

fn blocked_review_payload(full_output: &str) -> CodeReviewProviderPayload {
    CodeReviewProviderPayload {
        verdict: ReviewVerdict::Blocked,
        summary: format!(
            "review 输出不是有效 JSON，已阻塞并等待人工确认: {}",
            non_empty_trimmed(full_output).unwrap_or_else(|| "<empty>".to_string())
        ),
        findings: Vec::new(),
        impact_scope: Vec::new(),
        pr_description: String::new(),
        commit_message_suggestion: String::new(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
    }
}

fn default_review_summary(verdict: &ReviewVerdict) -> String {
    match verdict {
        ReviewVerdict::Approve => "review 通过".to_string(),
        ReviewVerdict::RequestChanges => "review 要求修改".to_string(),
        ReviewVerdict::Blocked => "review 被阻塞".to_string(),
    }
}

fn non_empty_trimmed(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn truncate_prompt_section(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated: String = value.chars().take(max_chars).collect();
    truncated.push_str("\n[truncated]");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::streaming_provider::ProviderSession;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::CreateCodingAttemptInput;
    use crate::product::coding_models::CodingProviderRole;
    use crate::product::models::{ProviderConversationRef, ProviderConversationRole};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;
    use tempfile::tempdir;

    fn blocked_report_with(missing: Vec<String>, skipped: Vec<String>) -> TestingReport {
        TestingReport {
            id: "testing_report_0001".to_string(),
            attempt_id: "coding_attempt_0001".to_string(),
            role_run_id: None,
            run_no: None,
            commands: Vec::new(),
            overall_status: TestingOverallStatus::Blocked,
            provider_claim: None,
            backend_verified: true,
            started_at: "2026-06-10T00:00:00Z".to_string(),
            completed_at: Some("2026-06-10T00:00:01Z".to_string()),
            plan_id: Some("test_plan_0001".to_string()),
            plan_summary: Some("plan".to_string()),
            steps: Vec::new(),
            unplanned_commands: Vec::new(),
            unplanned_evidence: Vec::new(),
            missing_required_steps: missing,
            skipped_required_steps: skipped,
            context_warnings: Vec::new(),
            raw_provider_output_ref: None,
        }
    }

    #[test]
    fn derive_reason_code_prefers_explicit() {
        let report = blocked_report_with(Vec::new(), vec!["S018".to_string()]);
        let reason = derive_testing_blocked_reason_code(
            Some("high_risk_test_step_requires_permission".to_string()),
            &report,
        );
        assert_eq!(reason, "high_risk_test_step_requires_permission");
    }

    #[test]
    fn derive_reason_code_uses_missing_when_present() {
        let report = blocked_report_with(vec!["unit".to_string()], vec!["S018".to_string()]);
        assert_eq!(
            derive_testing_blocked_reason_code(None, &report),
            "missing_required_steps"
        );
    }

    #[test]
    fn derive_reason_code_uses_skipped_when_only_skipped() {
        let report = blocked_report_with(Vec::new(), vec!["S018".to_string(), "S027".to_string()]);
        assert_eq!(
            derive_testing_blocked_reason_code(None, &report),
            "skipped_required_steps"
        );
    }

    #[test]
    fn derive_reason_code_falls_back_to_testing_blocked() {
        let report = blocked_report_with(Vec::new(), Vec::new());
        assert_eq!(
            derive_testing_blocked_reason_code(None, &report),
            "testing_blocked"
        );
    }

    struct NonProviderDrivenTestingProvider;

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for NonProviderDrivenTestingProvider {}

    struct ProviderDrivenTestingNoToolCallProvider;

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for ProviderDrivenTestingNoToolCallProvider {
        fn supports_provider_driven_testing(&self) -> bool {
            true
        }

        async fn start(
            &self,
            input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            tokio::spawn(async move {
                let output = if input.prompt.contains("Phase: plan_tests") {
                    serde_json::json!({
                        "summary": "provider planned tests",
                        "steps": [{
                            "id": "unit",
                            "title": "Unit tests",
                            "intent": "verify unit behavior",
                            "required": true,
                            "tool": "provider_managed",
                            "risk_level": "low",
                            "command_or_tool_input": {
                                "command": ["cargo", "test", "--locked", "--lib", "some_filter"]
                            },
                            "evidence_expectation": "provider supplies evidence",
                            "related_requirements": ["REQ-UNIT"],
                            "related_design_constraints": ["DEC-UNIT"],
                            "related_work_item_tasks": ["TASK-UNIT"]
                        }]
                    })
                    .to_string()
                } else {
                    serde_json::json!({
                        "step_results": [{
                            "step_id": "unit",
                            "status": "passed",
                            "evidence_refs": ["provider-managed-unit.log"],
                            "provider_analysis": "unit evidence accepted"
                        }]
                    })
                    .to_string()
                };
                let _ = event_tx
                    .send(ProviderEvent::TextDelta {
                        content: output.clone(),
                    })
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: output,
                        provider_session_id: None,
                    })
                    .await;
            });
            Ok(ProviderSession {
                events: event_rx,
                commands: command_tx,
            })
        }
    }

    struct ProviderDrivenTestingStartFailsProvider;

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for ProviderDrivenTestingStartFailsProvider {
        fn supports_provider_driven_testing(&self) -> bool {
            true
        }

        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            Err(ProviderAdapterError::command_missing(
                "tester provider command not found".to_string(),
            ))
        }
    }

    struct ProviderDrivenTestingMissingStepResultsProvider;

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for ProviderDrivenTestingMissingStepResultsProvider {
        fn supports_provider_driven_testing(&self) -> bool {
            true
        }

        async fn start(
            &self,
            input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            tokio::spawn(async move {
                let output = if input.prompt.contains("Phase: plan_tests") {
                    serde_json::json!({
                        "summary": "provider planned tests",
                        "steps": [{
                            "id": "unit",
                            "title": "Unit tests",
                            "intent": "verify unit behavior",
                            "required": true,
                            "tool": "provider_managed",
                            "risk_level": "low",
                            "command_or_tool_input": {"command": ["cargo", "test"]},
                            "evidence_expectation": "provider supplies evidence",
                            "related_requirements": ["REQ-UNIT"],
                            "related_design_constraints": ["DEC-UNIT"],
                            "related_work_item_tasks": ["TASK-UNIT"]
                        }]
                    })
                    .to_string()
                } else {
                    "I ran the tests and they passed.".to_string()
                };
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: output,
                        provider_session_id: None,
                    })
                    .await;
            });
            Ok(ProviderSession {
                events: event_rx,
                commands: command_tx,
            })
        }
    }

    #[test]
    fn coding_provider_role_maps_to_provider_conversation_role() {
        assert_eq!(
            provider_conversation_role_for_coding_role(&CodingProviderRole::Coder),
            ProviderConversationRole::Coder
        );
        assert_eq!(
            provider_conversation_role_for_coding_role(&CodingProviderRole::Tester),
            ProviderConversationRole::Tester
        );
        assert_eq!(
            provider_conversation_role_for_coding_role(&CodingProviderRole::Analyst),
            ProviderConversationRole::Analyst
        );
        assert_eq!(
            provider_conversation_role_for_coding_role(&CodingProviderRole::CodeReviewer),
            ProviderConversationRole::CodeReviewer
        );
        assert_eq!(
            provider_conversation_role_for_coding_role(&CodingProviderRole::InternalReviewer),
            ProviderConversationRole::InternalReviewer
        );
    }

    #[test]
    fn coding_provider_resume_session_id_is_isolated_by_role_and_provider() {
        let store = CodingAttemptStore::new(ProductAppPaths::new(
            tempdir().expect("tempdir").path().join(".aria"),
        ));
        let (tx, _rx) = mpsc::channel(8);
        let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
        let mut attempt = test_attempt("coding_attempt_0001");
        attempt.provider_conversations = vec![
            ProviderConversationRef {
                role: ProviderConversationRole::Coder,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "coder-session".to_string(),
                updated_at: "2026-06-01T00:00:00Z".to_string(),
                last_node_id: Some("coder-node".to_string()),
            },
            ProviderConversationRef {
                role: ProviderConversationRole::Tester,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "tester-session".to_string(),
                updated_at: "2026-06-01T00:01:00Z".to_string(),
                last_node_id: Some("tester-node".to_string()),
            },
        ];

        assert_eq!(
            engine.provider_resume_session_id_for_attempt(
                &attempt,
                &CodingProviderRole::Coder,
                &ProviderName::ClaudeCode,
            ),
            Some("coder-session".to_string())
        );
        assert_eq!(
            engine.provider_resume_session_id_for_attempt(
                &attempt,
                &CodingProviderRole::Tester,
                &ProviderName::ClaudeCode,
            ),
            None
        );
        assert_eq!(
            engine.provider_resume_session_id_for_attempt(
                &attempt,
                &CodingProviderRole::Coder,
                &ProviderName::Codex,
            ),
            None
        );
    }

    #[tokio::test]
    async fn testing_without_provider_driven_capability_routes_blocked_report_to_analyst() {
        let (_root, store, attempt) = running_attempt_with_worktree();
        let specs = vec![TestCommandSpec {
            id: "legacy_true".to_string(),
            command: vec!["true".to_string()],
        }];
        let (tx, _rx) = mpsc::channel(16);
        let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

        let report = engine
            .execute_testing_with_provider(
                &attempt,
                &NonProviderDrivenTestingProvider,
                &CodingExecutionContext::default(),
                &specs,
                TesterAgentOptions::default(),
            )
            .await
            .expect("blocked testing report");

        assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
        assert!(report.commands.is_empty());
        assert_eq!(report.plan_id, None);
        assert!(report.steps.is_empty());
        assert_eq!(report.raw_provider_output_ref, None);
        assert!(
            report
                .context_warnings
                .iter()
                .any(|warning| warning.contains("provider_driven_testing_not_supported"))
        );
        let updated = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("attempt");
        assert_eq!(updated.status, CodingAttemptStatus::Running);
        assert_eq!(
            store
                .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .expect("open gates")
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn real_provider_driven_testing_accepts_final_step_results_without_tool_calls() {
        let (_root, store, attempt) = running_attempt_with_worktree();
        let (tx, _rx) = mpsc::channel(16);
        let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

        let report = engine
            .execute_testing_with_provider(
                &attempt,
                &ProviderDrivenTestingNoToolCallProvider,
                &CodingExecutionContext::default(),
                &[],
                TesterAgentOptions::default(),
            )
            .await
            .expect("provider-driven testing");

        assert_eq!(report.overall_status, TestingOverallStatus::Passed);
        assert!(report.plan_id.is_some());
        assert_eq!(report.steps.len(), 1);
        assert_eq!(report.steps[0].step_id, "unit");
        assert_eq!(
            report.steps[0].evidence_refs,
            vec!["provider-managed-unit.log"]
        );
        assert!(report.commands.is_empty());
        assert!(report.raw_provider_output_ref.is_some());

        let chat_entries = store
            .list_chat_entries(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("chat entries");
        assert!(chat_entries.iter().any(|entry| {
            entry.role == CodingAgentRole::Tester
                && entry.entry_type == CodingEntryType::AssistantMessage
                && entry
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains("provider planned tests"))
                && entry
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get("phase"))
                    .and_then(|phase| phase.as_str())
                    == Some("test_plan")
        }));
        assert!(chat_entries.iter().any(|entry| {
            entry.role == CodingAgentRole::Tester
                && entry.entry_type == CodingEntryType::AssistantMessage
                && entry
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains("provider-managed-unit.log"))
                && entry
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get("phase"))
                    .and_then(|phase| phase.as_str())
                    == Some("testing_result")
        }));
    }

    #[tokio::test]
    async fn provider_driven_testing_blocks_when_provider_start_fails() {
        let (_root, store, attempt) = running_attempt_with_worktree();
        let (tx, _rx) = mpsc::channel(16);
        let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

        let report = engine
            .execute_testing_with_provider(
                &attempt,
                &ProviderDrivenTestingStartFailsProvider,
                &CodingExecutionContext::default(),
                &[],
                TesterAgentOptions::default(),
            )
            .await
            .expect("blocked testing report");

        assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
        assert!(report.commands.is_empty());
        assert!(
            report
                .context_warnings
                .iter()
                .any(|warning| warning.contains("provider_start_failed"))
        );
    }

    #[tokio::test]
    async fn provider_driven_testing_blocks_when_execute_output_has_no_step_results() {
        let (_root, _store, attempt) = running_attempt_with_worktree();
        let (tx, _rx) = mpsc::channel(16);
        let engine = CodingWorkspaceEngine::new(_store, GitWorkspaceService::new(), tx);

        let report = engine
            .execute_testing_with_provider(
                &attempt,
                &ProviderDrivenTestingMissingStepResultsProvider,
                &CodingExecutionContext::default(),
                &[],
                TesterAgentOptions::default(),
            )
            .await
            .expect("provider-driven testing");

        assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
        assert_eq!(report.missing_required_steps, vec!["unit"]);
        assert!(report.raw_provider_output_ref.is_some());
    }

    #[test]
    fn coding_prompt_guides_pnpm_install_when_frontend_dependencies_are_missing() {
        let attempt = test_attempt("coding_attempt_0001");
        let context = CodingExecutionContext::default();

        let prompt = build_coding_prompt(&attempt, &context, None, None);

        assert!(prompt.contains("node_modules missing"));
        assert!(prompt.contains("tsc EACCES"));
        assert!(prompt.contains("vitest EACCES"));
        assert!(prompt.contains("pnpm --version"));
        assert!(prompt.contains("pnpm -C <package-dir> install --frozen-lockfile"));
        assert!(prompt.contains("不要把缺少 node_modules 误判为 pnpm 不可用"));
    }

    #[test]
    fn coding_delta_prompt_guides_pnpm_install_when_frontend_dependencies_are_missing() {
        let attempt = test_attempt("coding_attempt_0001");
        let context = CodingExecutionContext::default();

        let prompt = build_coding_delta_prompt(&attempt, &context, None, None);

        assert!(prompt.contains("node_modules missing"));
        assert!(prompt.contains("pnpm -C <package-dir> install --frozen-lockfile"));
        assert!(prompt.contains("不要把缺少 node_modules 误判为 pnpm 不可用"));
    }

    #[test]
    fn review_parser_preserves_findings_with_common_aliases() {
        let payload = r#"{
          "verdict": "request_changes",
          "summary": "needs changes",
          "findings": [
            {
              "file": "src/lib.rs",
              "line": 42,
              "description": "missing validation",
              "recommendation": "add validation"
            }
          ]
        }"#;

        let parsed = parse_review_payload(payload, CodingExecutionStage::CodeReview);

        assert_eq!(parsed.verdict, ReviewVerdict::RequestChanges);
        assert_eq!(parsed.findings.len(), 1);
        assert_eq!(
            parsed.findings[0].severity,
            crate::product::coding_models::FindingSeverity::Warning
        );
        assert_eq!(
            parsed.findings[0].source_stage,
            CodingExecutionStage::CodeReview
        );
        assert_eq!(parsed.findings[0].file_path.as_deref(), Some("src/lib.rs"));
        assert_eq!(parsed.findings[0].message, "missing validation");
        assert_eq!(
            parsed.findings[0].required_action.as_deref(),
            Some("add validation")
        );
    }

    #[test]
    fn rework_and_internal_review_prompts_require_openspec_and_superpowers() {
        let attempt = test_attempt("coding_attempt_0001");
        let context_notes = ReworkContextNoteInput {
            text: "manual context".to_string(),
            truncated: false,
        };
        let prompt = build_rework_prompt(
            &attempt,
            "testing blocked",
            &CodingExecutionStage::Testing,
            1,
            &context_notes,
            "{}",
            None,
        );

        assert!(prompt.contains("[openspec_contract]"));
        assert!(prompt.contains("[superpowers_contract]"));
        assert!(prompt.contains("Story Spec"));
        assert!(prompt.contains("Design Spec"));
        assert!(prompt.contains("Work Item"));

        let internal_contract = provider_runtime_contract("InternalReviewer");
        assert!(internal_contract.contains("InternalReviewer"));
        assert!(internal_contract.contains("[openspec_contract]"));
        assert!(internal_contract.contains("[superpowers_contract]"));
    }

    #[test]
    fn tester_tool_results_without_step_id_remain_unplanned_evidence() {
        let plan = TestPlan {
            id: "test_plan_0001".to_string(),
            attempt_id: "coding_attempt_0001".to_string(),
            role_run_id: None,
            run_no: None,
            summary: "unit checks".to_string(),
            context_warnings: Vec::new(),
            assumptions: Vec::new(),
            steps: vec![crate::product::coding_models::TestPlanStep {
                id: "unit".to_string(),
                title: "Unit tests".to_string(),
                intent: "verify unit behavior".to_string(),
                required: true,
                tool: crate::product::coding_models::TestPlanTool::RunCommand,
                risk_level: crate::product::coding_models::TestPlanRiskLevel::Low,
                command_or_tool_input: serde_json::json!({"command": ["true"]}),
                evidence_expectation: "exit 0".to_string(),
                related_requirements: Vec::new(),
                related_design_constraints: Vec::new(),
                related_work_item_tasks: Vec::new(),
            }],
            created_at: "2026-06-10T00:00:00Z".to_string(),
            raw_provider_output_ref: None,
        };
        let calls = [
            ProviderToolCall {
                id: "read_file_0001".to_string(),
                tool_name: "read_file".to_string(),
                input: serde_json::json!({"path": "src/lib.rs"}),
            },
            ProviderToolCall {
                id: "search_code_0001".to_string(),
                tool_name: "search_code".to_string(),
                input: serde_json::json!({"query": "unsafe"}),
            },
            ProviderToolCall {
                id: "run_command_0001".to_string(),
                tool_name: "run_command".to_string(),
                input: serde_json::json!({"command": ["true"]}),
            },
        ];
        let mut step_results = Vec::new();
        let mut unplanned_commands = Vec::new();
        let mut unplanned_evidence = Vec::new();
        let mut context_warnings = Vec::new();
        for call in &calls {
            let command = (call.tool_name == "run_command").then(|| TestCommand {
                command: vec!["true".to_string()],
                cwd: PathBuf::from("/tmp/worktree"),
                exit_code: Some(0),
                duration_ms: 1,
                stdout_ref: "stdout.log".to_string(),
                stderr_ref: "stderr.log".to_string(),
                status: TestCommandStatus::Passed,
            });
            let result = ProviderToolResult {
                tool_use_id: call.id.clone(),
                output: format!("{} ok", call.tool_name),
                is_error: false,
            };
            record_tester_step_result(
                &plan,
                call,
                command,
                &result,
                TesterStepResultOutputs {
                    step_results: &mut step_results,
                    unplanned_commands: &mut unplanned_commands,
                    unplanned_evidence: &mut unplanned_evidence,
                    context_warnings: &mut context_warnings,
                },
            );
        }

        let mut report = build_plan_based_testing_report(
            "testing_report_0001",
            "coding_attempt_0001",
            &plan,
            step_results,
            unplanned_commands,
            None,
            None,
        );
        report.unplanned_evidence = unplanned_evidence;

        assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
        assert_eq!(report.missing_required_steps, vec!["unit"]);
        assert!(report.steps.is_empty());
        assert_eq!(report.unplanned_commands.len(), 1);
        assert_eq!(report.unplanned_evidence.len(), 3);
    }

    #[tokio::test]
    async fn blocked_gate_response_is_idempotent_across_reconnects() {
        let store = CodingAttemptStore::new(ProductAppPaths::new(
            tempdir().expect("tempdir").path().join(".aria"),
        ));
        let attempt = store
            .create_attempt(
                crate::product::coding_attempt_store::CreateCodingAttemptInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    work_item_id: "work_item_0001".to_string(),
                    base_branch: "main".to_string(),
                    branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                    worktree_path: None,
                    provider_config_snapshot: ProviderConfigSnapshot {
                        author: ProviderName::Codex,
                        reviewer: Some(ProviderName::ClaudeCode),
                        review_rounds: 1,
                    },
                    max_auto_rework: 2,
                },
            )
            .expect("create attempt");
        let attempt = store
            .update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Running,
            )
            .expect("running");
        let attempt = store
            .update_attempt_stage(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingExecutionStage::Testing,
            )
            .expect("testing");
        let attempt = store
            .update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )
            .expect("blocked");
        let gate = store
            .create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Testing,
                node_id: Some("coding_node_0001".to_string()),
                role: Some(CodingProviderRole::Tester),
                title: "Testing blocked".to_string(),
                description: "missing required step".to_string(),
                reason_code: Some("missing_required_steps".to_string()),
                evidence_refs: vec!["testing_report_0001.json".to_string()],
                raw_provider_output_ref: None,
                available_actions: testing_blocked_gate_actions(),
            })
            .expect("blocked gate");
        assert_eq!(
            store
                .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .expect("open gates")
                .len(),
            1
        );
        let (tx, _rx) = mpsc::channel(8);
        let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

        let _updated = engine
            .handle_blocked_gate_response(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &gate.gate_id,
                "retry_test_plan",
                None,
            )
            .await
            .expect("first response");
        assert!(
            store
                .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .expect("open gates after resolve")
                .is_empty()
        );

        let second = engine
            .handle_blocked_gate_response(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &gate.gate_id,
                "retry_test_plan",
                None,
            )
            .await
            .expect("second response is idempotent");
        assert_eq!(second.status, CodingAttemptStatus::Running);
        assert_eq!(second.stage, CodingExecutionStage::Testing);
    }

    #[tokio::test]
    async fn manual_continue_persists_quality_bypass_audit_and_injects_reviewer_context() {
        let paths = ProductAppPaths::new(tempdir().expect("tempdir").path().join(".aria"));
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_attempt(
                crate::product::coding_attempt_store::CreateCodingAttemptInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    work_item_id: "work_item_0001".to_string(),
                    base_branch: "main".to_string(),
                    branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                    worktree_path: None,
                    provider_config_snapshot: ProviderConfigSnapshot {
                        author: ProviderName::Codex,
                        reviewer: Some(ProviderName::ClaudeCode),
                        review_rounds: 1,
                    },
                    max_auto_rework: 2,
                },
            )
            .expect("create attempt");
        let attempt = store
            .update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Running,
            )
            .expect("running");
        let attempt = store
            .update_attempt_stage(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingExecutionStage::Testing,
            )
            .expect("testing");
        let attempt = store
            .update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )
            .expect("blocked");
        store
            .save_testing_report(&TestingReport {
                id: "testing_report_0001".to_string(),
                attempt_id: attempt.id.clone(),
                role_run_id: None,
                run_no: None,
                commands: Vec::new(),
                overall_status: TestingOverallStatus::Blocked,
                provider_claim: None,
                backend_verified: true,
                started_at: "2026-06-10T00:00:00Z".to_string(),
                completed_at: Some("2026-06-10T00:00:01Z".to_string()),
                plan_id: Some("test_plan_0001".to_string()),
                plan_summary: Some("unit checks".to_string()),
                steps: Vec::new(),
                unplanned_commands: Vec::new(),
                unplanned_evidence: Vec::new(),
                missing_required_steps: vec!["unit".to_string()],
                skipped_required_steps: Vec::new(),
                context_warnings: Vec::new(),
                raw_provider_output_ref: None,
            })
            .expect("testing report");
        let gate = store
            .create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Testing,
                node_id: Some("coding_node_0001".to_string()),
                role: Some(CodingProviderRole::Tester),
                title: "Testing blocked".to_string(),
                description: "missing required step".to_string(),
                reason_code: Some("missing_required_steps".to_string()),
                evidence_refs: vec!["testing_report_0001.json".to_string()],
                raw_provider_output_ref: None,
                available_actions: testing_blocked_gate_actions(),
            })
            .expect("blocked gate");
        let (tx, _rx) = mpsc::channel(8);
        let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

        assert!(
            engine
                .handle_blocked_gate_response(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &gate.gate_id,
                    "manual_continue",
                    None,
                )
                .await
                .is_err()
        );

        let updated = engine
            .handle_blocked_gate_response(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &gate.gate_id,
                "manual_continue",
                Some("operator accepts residual risk".to_string()),
            )
            .await
            .expect("manual continue");
        assert_eq!(updated.status, CodingAttemptStatus::Running);
        assert_eq!(updated.stage, CodingExecutionStage::Testing);

        let audits = store
            .list_quality_bypass_audits(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("audits");
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].gate_id, gate.gate_id);
        assert_eq!(audits[0].skipped_required_steps, vec!["unit"]);
        assert_eq!(audits[0].operator_context, "operator accepts residual risk");

        let updated = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("attempt");
        let pack =
            build_evaluation_context_pack(paths, &updated, EvaluationContextRole::CodeReviewer)
                .expect("evaluation context");
        assert_eq!(pack.quality_bypass_audits.len(), 1);
        assert_eq!(
            pack.quality_bypass_audits[0].skipped_required_steps,
            vec!["unit"]
        );
    }

    #[tokio::test]
    async fn continue_rework_after_limit_persists_instruction_without_quality_bypass() {
        let paths = ProductAppPaths::new(tempdir().expect("tempdir").path().join(".aria"));
        let store = CodingAttemptStore::new(paths);
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Codex,
                    reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                },
                max_auto_rework: 2,
            })
            .expect("create attempt");
        let mut attempt = store
            .update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Running,
            )
            .expect("running");
        attempt = store
            .increment_attempt_rework_count(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("first rework");
        attempt = store
            .increment_attempt_rework_count(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("second rework");
        attempt = store
            .update_attempt_stage(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingExecutionStage::Rework,
            )
            .expect("rework stage");
        attempt = store
            .update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )
            .expect("blocked");
        store
            .save_analyst_decision(&AnalystDecisionRecord {
                id: "analyst_decision_0001".to_string(),
                attempt_id: attempt.id.clone(),
                source_stage: CodingExecutionStage::CodeReview,
                rework_round: 3,
                verdict: AnalystDecisionVerdict::NeedsFix,
                next_stage: AnalystDecisionNextStage::Coding,
                reason: "CodeReview 仍有阻塞问题".to_string(),
                evidence_refs: vec!["code_review_0001/findings[0]".to_string()],
                raw_provider_output_refs: vec![
                    "provider-raw/code_review/code_review_0001.txt".to_string(),
                ],
                rework_instructions: Some(AnalystReworkInstructions {
                    summary: "修复 provider install 契约".to_string(),
                    required_changes: vec!["改为 202 installing".to_string()],
                    verification_expectations: vec!["补并发安装测试".to_string()],
                }),
                human_gate: None,
                created_at: "2026-06-14T00:00:00Z".to_string(),
                parse_error: None,
                role_run_id: None,
                run_no: Some(1),
            })
            .expect("analyst decision");
        let gate = store
            .create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Rework,
                node_id: Some("coding_node_0001".to_string()),
                role: Some(CodingProviderRole::Analyst),
                title: "Rework limit reached".to_string(),
                description: "已达到自动重写上限".to_string(),
                reason_code: Some("max_auto_rework_exceeded".to_string()),
                evidence_refs: vec!["code_review_0001/findings[0]".to_string()],
                raw_provider_output_ref: Some(
                    "provider-raw/code_review/code_review_0001.txt".to_string(),
                ),
                available_actions: vec![
                    coding_gate_action_for_id("continue_rework").expect("continue rework action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            })
            .expect("blocked gate");
        let (tx, _rx) = mpsc::channel(8);
        let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

        let updated = engine
            .handle_blocked_gate_response(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &gate.gate_id,
                "continue_rework",
                Some("继续修 CodeReview findings".to_string()),
            )
            .await
            .expect("continue rework");

        assert_eq!(updated.status, CodingAttemptStatus::Running);
        assert_eq!(updated.stage, CodingExecutionStage::Coding);
        assert_eq!(updated.rework_count, 3);
        assert!(
            store
                .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .expect("open gates")
                .is_empty()
        );
        let instructions = store
            .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("rework instructions");
        assert_eq!(instructions.len(), 1);
        assert_eq!(instructions[0].summary, "修复 provider install 契约");
        assert_eq!(
            instructions[0].fix_hints,
            vec!["改为 202 installing", "补并发安装测试"]
        );
        let notes = store
            .list_context_notes(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("context notes");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].content, "继续修 CodeReview findings");
        assert!(
            store
                .list_quality_bypass_audits(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .expect("quality bypass audits")
                .is_empty()
        );
    }

    #[test]
    fn dangerous_test_plan_step_requires_permission_or_blocks() {
        let plan = TestPlan {
            id: "test_plan_0001".to_string(),
            attempt_id: "coding_attempt_0001".to_string(),
            role_run_id: None,
            run_no: None,
            summary: "dangerous checks".to_string(),
            context_warnings: Vec::new(),
            assumptions: Vec::new(),
            steps: vec![crate::product::coding_models::TestPlanStep {
                id: "destructive".to_string(),
                title: "destructive command".to_string(),
                intent: "should require approval".to_string(),
                required: true,
                tool: crate::product::coding_models::TestPlanTool::RunCommand,
                risk_level: crate::product::coding_models::TestPlanRiskLevel::High,
                command_or_tool_input: serde_json::json!({
                    "command": ["rm", "-rf", "/tmp/some-target"]
                }),
                evidence_expectation: "must not run without approval".to_string(),
                related_requirements: Vec::new(),
                related_design_constraints: Vec::new(),
                related_work_item_tasks: Vec::new(),
            }],
            created_at: "2026-06-10T00:00:00Z".to_string(),
            raw_provider_output_ref: None,
        };
        let call = ProviderToolCall {
            id: "run_command_0001".to_string(),
            tool_name: "run_command".to_string(),
            input: serde_json::json!({
                "step_id": "destructive",
                "command": ["rm", "-rf", "/tmp/some-target"]
            }),
        };

        assert_eq!(
            high_risk_test_step_block_reason(&plan, &call),
            Some("high_risk_test_step_requires_permission")
        );
    }

    fn running_attempt_with_worktree() -> (
        tempfile::TempDir,
        CodingAttemptStore,
        CodingExecutionAttempt,
    ) {
        let root = tempdir().expect("tempdir");
        let worktree = root.path().join("worktree");
        std::fs::create_dir_all(&worktree).expect("worktree dir");
        let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                worktree_path: Some(worktree),
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Codex,
                    reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                },
                max_auto_rework: 2,
            })
            .expect("create attempt");
        let attempt = store
            .update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Running,
            )
            .expect("running attempt");
        (root, store, attempt)
    }

    fn test_attempt(id: &str) -> CodingExecutionAttempt {
        CodingExecutionAttempt {
            id: id.to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            attempt_no: 1,
            status: CodingAttemptStatus::Running,
            stage: CodingExecutionStage::Coding,
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
            },
            provider_conversations: Vec::new(),
            rework_count: 0,
            max_auto_rework: 2,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            created_at: "2026-06-01T00:00:00Z".to_string(),
            updated_at: "2026-06-01T00:00:00Z".to_string(),
            completed_at: None,
        }
    }
}
