use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ChoiceRequestData, PermissionRequestData, ProviderCommand, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, ProviderStatus, ProviderToolCall, ProviderToolResult, RiskLevel,
    StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    AnalystVerdict, CodeReviewReport, CodingAgentRole, CodingAttemptStatus, CodingChatEntry,
    CodingContextNote, CodingEntryType, CodingExecutionAttempt, CodingExecutionStage,
    CodingProviderRole, CodingReworkInstruction, CodingTimelineNode, CodingTimelineNodeStatus,
    InternalPrReview, PushStatus, ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict,
    TestingOverallStatus, TestingReport,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::{GitWorkspaceError, GitWorkspaceService};
use crate::product::id::next_sequential_id;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    ProviderConversationRef, ProviderConversationRole, ProviderName, WorkItemStatus, WorkspaceType,
};
use crate::product::test_executor::{TestCommandSpec, TestExecutorError, run_all_tests};
use crate::product::tester_agent_loop::{
    TesterAgentOptions, build_tester_system_prompt, build_testing_report, execute_tester_tool_call,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};
use crate::web::coding_ws_handler::CodingWsOutMessage;
use crate::web::workspace_ws_types::{
    ChoiceOption, WsExecutionEvent, WsExecutionEventKind, WsExecutionEventStatus,
    WsPermissionRiskLevel,
};

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
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodingExecutionContext {
    pub work_item_markdown: Option<String>,
    pub verification_commands: Vec<String>,
}

struct CodingProviderStreamRun<'a> {
    attempt: &'a CodingExecutionAttempt,
    node_id: &'a str,
    provider: &'a dyn StreamingProviderAdapter,
    legacy_input: &'a AdapterInput,
    input: StreamingProviderInput,
    provider_name: &'a ProviderName,
    provider_role: CodingProviderRole,
    command_rx: &'a mut mpsc::Receiver<CodingRunnerCommand>,
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

#[derive(Debug, Clone)]
pub struct CodingWorkspaceEngine {
    store: CodingAttemptStore,
    _git_service: GitWorkspaceService,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
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
        let prompt_mode = if resume_provider_session_id.is_some() {
            CodingPromptMode::DeltaOnly
        } else {
            CodingPromptMode::FullConversation
        };
        let prompt = match prompt_mode {
            CodingPromptMode::FullConversation => {
                build_coding_prompt(&attempt, context, rework_instruction.as_ref())
            }
            CodingPromptMode::DeltaOnly => {
                build_coding_delta_prompt(&attempt, context, rework_instruction.as_ref())
            }
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

        let legacy_input = AdapterInput {
            provider_type: provider_type_for_name(&coder_provider),
            role: AdapterRole::Executor,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt,
            context_files: Vec::new(),
            output_schema: "coding_workspace_markdown".to_string(),
            timeout: 2400,
            max_retries: 0,
        };
        let input = StreamingProviderInput {
            provider_type: legacy_input.provider_type.clone(),
            role: legacy_input.role.clone(),
            prompt: legacy_input.prompt.clone(),
            working_dir: worktree_path.clone(),
            workspace_session_id: Some(attempt.id.clone()),
            resume_provider_session_id,
            permission_mode: ProviderPermissionMode::Auto,
            env_vars: BTreeMap::new(),
            timeout_secs: legacy_input.timeout,
        };
        let _full_output = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                provider,
                legacy_input: &legacy_input,
                input,
                provider_name: &coder_provider,
                provider_role: CodingProviderRole::Coder,
                command_rx,
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

    async fn run_provider_stream_to_completion(
        &self,
        run: CodingProviderStreamRun<'_>,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let CodingProviderStreamRun {
            attempt,
            node_id,
            provider,
            legacy_input,
            input,
            provider_name,
            provider_role,
            command_rx,
        } = run;
        let cancel = CancellationToken::new();
        let mut session = match provider.start(input, cancel.clone()).await {
            Ok(session) => session,
            Err(error) if provider_start_is_not_implemented(&error) => {
                return self
                    .run_legacy_stream_to_completion(attempt, node_id, provider, legacy_input)
                    .await;
            }
            Err(error) => {
                return self
                    .fail_provider_stream(attempt, node_id, error.details)
                    .await;
            }
        };
        let mut commands_open = true;
        let mut full_output = String::new();
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();
        loop {
            tokio::select! {
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
                            return Err(CodingWorkspaceEngineError::Aborted);
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
                        return self.fail_provider_stream_ended(attempt, node_id).await;
                    };
                    match event {
                        ProviderEvent::TextDelta { content } => {
                            full_output.push_str(&content);
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingStreamChunk {
                                    content,
                                    node_id: Some(node_id.to_string()),
                                })
                                .await;
                        }
                        ProviderEvent::Execution(event) => {
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
                        }
                        ProviderEvent::ToolCall(call) => {
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
                        }
                        ProviderEvent::ToolResult(result) => {
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
                        }
                        ProviderEvent::PermissionRequest(request) => {
                            self.emit_permission_request(node_id, provider_name, request).await;
                        }
                        ProviderEvent::ChoiceRequest(request) => {
                            self.emit_choice_request(node_id, provider_name, request).await;
                        }
                        ProviderEvent::StatusChanged(status) => {
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
                        }
                        ProviderEvent::Completed {
                            full_output: completed_output,
                            provider_session_id,
                        } => {
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
                            return Ok(full_output);
                        }
                        ProviderEvent::Failed { message } => {
                            return self.fail_provider_stream(attempt, node_id, message).await;
                        }
                        ProviderEvent::ProtocolError { message, .. } => {
                            return self.fail_provider_stream(attempt, node_id, message).await;
                        }
                        ProviderEvent::PermissionTimeout { permission_id } => {
                            return self
                                .fail_provider_stream(
                                    attempt,
                                    node_id,
                                    format!("Permission request {permission_id} timed out"),
                                )
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
            CodingAttemptStatus::Blocked,
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
        node_id: &str,
        provider: &ProviderName,
        request: ChoiceRequestData,
    ) {
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
        ) {
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )?;
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
        context: &CodingExecutionContext,
        specs: &[TestCommandSpec],
        options: TesterAgentOptions,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        if !provider.supports_tool_calls() {
            return self.execute_testing(attempt, specs).await;
        }
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

        let tester_provider = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .tester;
        let prompt = build_tester_system_prompt(&attempt, context, specs);
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &tester_provider,
                    prompt.clone(),
                    CodingPromptMode::FullConversation.event_detail(),
                ),
            })
            .await;

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
            permission_mode: ProviderPermissionMode::Auto,
            env_vars: BTreeMap::new(),
            timeout_secs: options.timeout.as_secs().max(1),
        };
        let cancel = CancellationToken::new();
        let mut session = provider.start(input, cancel.clone()).await?;
        let timeout = tokio::time::sleep(options.timeout);
        tokio::pin!(timeout);
        let mut full_output = String::new();
        let mut commands = Vec::new();
        let mut consecutive_failures = 0usize;
        let mut blocked_summary = None;
        let mut chat_entry_sequence = 1usize;
        let mut commands_open = true;
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    cancel.cancel();
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
                            return Err(CodingWorkspaceEngineError::Aborted);
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
                        blocked_summary = Some("Tester Provider stream ended before completion".to_string());
                        break;
                    };
                    match event {
                        ProviderEvent::TextDelta { content } => {
                            full_output.push_str(&content);
                            let _ = self
                                .event_tx
                                .send(CodingWsOutMessage::CodingStreamChunk {
                                    content,
                                    node_id: Some(node.id.clone()),
                                })
                                .await;
                        }
                        ProviderEvent::ToolCall(call) => {
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
                                Some(serde_json::json!({ "tool_use_id": call.id.clone() })),
                            );
                            self.save_and_emit_chat_entry(entry).await;

                            let artifact_output_root = self.store.attempt_test_output_root(
                                &attempt.project_id,
                                &attempt.issue_id,
                                &attempt.id,
                            );
                            let outcome =
                                execute_tester_tool_call(&call, worktree_path, artifact_output_root)
                                    .await?;
                            if let Some(command) = outcome.command.clone() {
                                commands.push(command);
                            }
                            let result = outcome.result;
                            let is_error = result.is_error;
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
                                result,
                            )
                            .await;

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
                                result,
                            )
                            .await;
                        }
                        ProviderEvent::Execution(event) => {
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
                        }
                        ProviderEvent::Completed {
                            full_output: completed_output,
                            provider_session_id,
                        } => {
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
                            break;
                        }
                        ProviderEvent::Failed { message } => {
                            blocked_summary = Some(message);
                            break;
                        }
                        ProviderEvent::ProtocolError { message, .. } => {
                            blocked_summary = Some(message);
                            break;
                        }
                        ProviderEvent::PermissionTimeout { permission_id } => {
                            blocked_summary =
                                Some(format!("Permission request {permission_id} timed out"));
                            break;
                        }
                        ProviderEvent::PermissionRequest(request) => {
                            self.emit_permission_request(&node.id, &tester_provider, request).await;
                        }
                        ProviderEvent::ChoiceRequest(request) => {
                            self.emit_choice_request(&node.id, &tester_provider, request).await;
                        }
                        ProviderEvent::StatusChanged(status) => {
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
                        }
                    }
                }
            }
        }

        let report = build_testing_report(&attempt.id, commands, &full_output, blocked_summary);
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
        ) {
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )?;
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

        let reviewer = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .code_reviewer;
        let prompt = self
            .build_code_review_prompt(&attempt, worktree_path)
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
            timeout: 2400,
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
        let full_output = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                provider,
                legacy_input: &input,
                input: provider_input,
                provider_name: &reviewer,
                provider_role: CodingProviderRole::CodeReviewer,
                command_rx,
            })
            .await?;
        let report = self.build_code_review_report(&attempt, &full_output)?;
        self.store.save_code_review_report(&report)?;
        self.emit_code_review_chat_entry(&attempt, &node.id, &report)
            .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodeReviewComplete {
                report: Box::new(report.clone()),
            })
            .await;
        let (node_status, summary) = match report.verdict {
            ReviewVerdict::Approve => (
                CodingTimelineNodeStatus::Completed,
                Some("code review 通过".to_string()),
            ),
            ReviewVerdict::RequestChanges => (
                CodingTimelineNodeStatus::Failed,
                Some("code review 要求修改".to_string()),
            ),
            ReviewVerdict::Blocked => {
                self.store.update_attempt_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingAttemptStatus::Blocked,
                )?;
                (
                    CodingTimelineNodeStatus::Blocked,
                    Some("code review 被阻塞".to_string()),
                )
            }
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

        let notes = self.store.list_unconsumed_context_notes(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let note_ids = notes.iter().map(|note| note.id.clone()).collect::<Vec<_>>();
        let context_note_input =
            format_rework_context_notes(&notes, REWORK_CONTEXT_NOTE_CHAR_LIMIT);
        let analyst_provider = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .analyst;
        let prompt = build_rework_prompt(
            &attempt,
            evidence,
            &source_stage,
            rework_round,
            &context_note_input,
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
            timeout: 2400,
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
        let full_output = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                provider,
                legacy_input: &input,
                input: provider_input,
                provider_name: &analyst_provider,
                provider_role: CodingProviderRole::Analyst,
                command_rx,
            })
            .await?;
        if !note_ids.is_empty() {
            self.store.mark_context_notes_consumed(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &note_ids,
                rework_round,
            )?;
        }
        let decision = parse_analyst_verdict(&full_output);
        self.emit_analyst_verdict_entry(&attempt, &node.id, rework_round, &source_stage, &decision)
            .await;
        let (updated, node_status, summary) = self
            .apply_analyst_decision(&attempt, &node.id, &source_stage, rework_round, &decision)
            .await?;
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

        let reviewer = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .internal_reviewer;
        let prompt = self
            .build_internal_pr_review_prompt(&attempt, &review_request, worktree_path)
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
            timeout: 2400,
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
        let full_output = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                provider,
                legacy_input: &input,
                input: provider_input,
                provider_name: &reviewer,
                provider_role: CodingProviderRole::InternalReviewer,
                command_rx,
            })
            .await?;
        let review = self.build_internal_pr_review(&attempt, &review_request, &full_output)?;
        self.store.save_internal_pr_review(&review)?;
        self.emit_internal_pr_review_chat_entry(&attempt, &node.id, &review)
            .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::InternalPrReviewComplete {
                review: Box::new(review.clone()),
            })
            .await;
        let (node_status, summary) = match review.verdict {
            ReviewVerdict::Approve => (
                CodingTimelineNodeStatus::Completed,
                Some("internal PR review 通过".to_string()),
            ),
            ReviewVerdict::RequestChanges => (
                CodingTimelineNodeStatus::Failed,
                Some("internal PR review 要求修改".to_string()),
            ),
            ReviewVerdict::Blocked => {
                self.store.update_attempt_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingAttemptStatus::Blocked,
                )?;
                (
                    CodingTimelineNodeStatus::Blocked,
                    Some("internal PR review 被阻塞".to_string()),
                )
            }
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
        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Aborted,
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
    ) -> Result<String, CodingWorkspaceEngineError> {
        let diff = self
            ._git_service
            .git_diff(worktree_path, &attempt.base_branch)
            .await?;
        let work_item = self.work_item_markdown_for_attempt(attempt)?;
        Ok(format!(
            "Coding Workspace CodeReviewer\n\
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
             \ngit diff:\n````diff\n{}\n````\n\
             \n只输出 JSON：{{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[...]}}\n",
            attempt.project_id,
            attempt.issue_id,
            attempt.work_item_id,
            attempt.id,
            attempt.branch_name,
            attempt.base_branch,
            work_item.unwrap_or_else(
                || "未找到 Work Item markdown，上下文仅包含 attempt 元数据。".to_string()
            ),
            truncate_prompt_section(&diff, 30_000)
        ))
    }

    async fn build_internal_pr_review_prompt(
        &self,
        attempt: &CodingExecutionAttempt,
        review_request: &ReviewRequest,
        worktree_path: &Path,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let diff = self
            ._git_service
            .git_diff(worktree_path, &attempt.base_branch)
            .await?;
        let work_item = self.work_item_markdown_for_attempt(attempt)?;
        Ok(format!(
            "Coding Workspace InternalReviewer\n\
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
             \n完整变更 git diff:\n````diff\n{}\n````\n\
             \n输出要求:\n\
             - 分析影响范围（影响范围/impact_scope）。\n\
             - 给出 PR description 预览。\n\
             - 给出 commit message 建议。\n\
             - findings 必须包含 source_stage=internal_pr_review。\n\
             \n只输出 JSON：{{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[...],\"impact_scope\":[\"...\"],\"pr_description\":\"...\",\"commit_message_suggestion\":\"...\"}}\n",
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
            truncate_prompt_section(&diff, 30_000)
        ))
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
        })
    }

    fn build_internal_pr_review(
        &self,
        attempt: &CodingExecutionAttempt,
        review_request: &ReviewRequest,
        full_output: &str,
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
    ) {
        let mut metadata = serde_json::json!({
            "source": "analyst",
            "source_stage": source_stage,
            "rework_round": rework_round,
        });
        if let Some(object) = metadata.as_object_mut() {
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
        match decision.verdict {
            AnalystVerdict::NeedsFix => {
                if attempt.rework_count < attempt.max_auto_rework {
                    let existing = self.store.list_rework_instructions(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                    )?;
                    let instruction = CodingReworkInstruction {
                        id: next_sequential_id("coding_rework_instruction", existing.len()),
                        attempt_id: attempt.id.clone(),
                        source_stage: source_stage.clone(),
                        rework_round,
                        summary: decision.summary.clone(),
                        fix_hints: decision.fix_hints.clone(),
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
                    let updated = self.store.update_attempt_stage(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        CodingExecutionStage::CodeReview,
                    )?;
                    Ok((
                        updated,
                        CodingTimelineNodeStatus::Completed,
                        format!("NeedsFix: {}；已达到自动重写上限", decision.summary),
                    ))
                }
            }
            AnalystVerdict::NeedsHumanInput => {
                let updated = self.store.update_attempt_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingAttemptStatus::WaitingForHuman,
                )?;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Blocked,
                    format!("NeedsHumanInput: {}", decision.summary),
                ))
            }
            AnalystVerdict::NoIssue => {
                let updated = match source_stage {
                    CodingExecutionStage::Testing => self.store.update_attempt_stage(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        CodingExecutionStage::CodeReview,
                    )?,
                    CodingExecutionStage::CodeReview => self.store.update_attempt_stage(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        CodingExecutionStage::ReviewRequest,
                    )?,
                    CodingExecutionStage::InternalPrReview => {
                        self.complete_attempt_after_final_rework(attempt).await?
                    }
                    _ => self.store.update_attempt_stage(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        CodingExecutionStage::CodeReview,
                    )?,
                };
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Completed,
                    format!("NoIssue: {}", decision.summary),
                ))
            }
        }
    }

    async fn emit_tester_tool_result_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        sequence: &mut usize,
        result: ProviderToolResult,
    ) {
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
            None,
        );
        self.save_and_emit_chat_entry(entry).await;
    }
}

fn worktree_path_for_attempt(repo_path: &Path, attempt: &CodingExecutionAttempt) -> PathBuf {
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
    prompt.push_str(
        "\n执行要求:\n\
         - 遵循仓库规则和 TDD 流程。\n\
         - 不要重新生成 Story/Design/Work Item 文档。\n\
         - 完成后报告修改文件、测试命令和结果。\n",
    );
    prompt
}

fn build_rework_prompt(
    attempt: &CodingExecutionAttempt,
    evidence: &str,
    source_stage: &CodingExecutionStage,
    rework_round: u32,
    context_notes: &ReworkContextNoteInput,
) -> String {
    format!(
        "Coding Workspace Rework 分析官\n\
         你是 Coding Workspace Rework 分析官，只做分析和路由决策。\n\
         严格要求：不要修改代码，不要调用 tool_use，不要执行命令。\n\
         仅根据上一阶段 summary/evidence 与本轮新增 ContextNote 输出 JSON AnalystVerdict。\n\
         JSON 格式：{{\"verdict\":\"needs_fix|needs_human_input|no_issue\",\"summary\":\"...\",\"fix_hints\":[\"...\"],\"questions\":[\"...\"]}}\n\
         Project: {}\n\
         Issue: {}\n\
         Work Item: {}\n\
         Attempt: {}\n\
         Branch: {}\n\
         Previous Stage: {:?}\n\
         Rework Round: {}\n\
         ContextNotes Truncated: {}\n\
         \n上一阶段 summary/evidence:\n{}\n\
         \n本轮新增 ContextNote:\n{}\n",
        attempt.project_id,
        attempt.issue_id,
        attempt.work_item_id,
        attempt.id,
        attempt.branch_name,
        source_stage,
        rework_round,
        context_notes.truncated,
        evidence,
        context_notes.text
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
        permission_mode: ProviderPermissionMode::Auto,
        env_vars: BTreeMap::new(),
        timeout_secs: input.timeout,
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
    summary: String,
    fix_hints: Vec<String>,
    questions: Vec<String>,
    parse_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnalystProviderPayload {
    verdict: AnalystVerdict,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    fix_hints: Vec<String>,
    #[serde(default)]
    questions: Vec<String>,
}

fn parse_analyst_verdict(full_output: &str) -> AnalystDecision {
    let Some(json_text) = extract_json_object(full_output) else {
        return AnalystDecision {
            verdict: AnalystVerdict::NeedsHumanInput,
            summary: "Analyst 输出不是有效 JSON，已转人工确认。".to_string(),
            fix_hints: Vec::new(),
            questions: vec!["请人工确认 Analyst 输出并补充下一步处理意见。".to_string()],
            parse_error: Some("missing_json_object".to_string()),
        };
    };

    match serde_json::from_str::<AnalystProviderPayload>(json_text) {
        Ok(payload) => {
            let summary = payload
                .summary
                .as_deref()
                .and_then(non_empty_trimmed)
                .unwrap_or_else(|| default_analyst_summary(&payload.verdict));
            AnalystDecision {
                verdict: payload.verdict,
                summary,
                fix_hints: payload.fix_hints,
                questions: payload.questions,
                parse_error: None,
            }
        }
        Err(error) => AnalystDecision {
            verdict: AnalystVerdict::NeedsHumanInput,
            summary: "Analyst 输出不是有效 JSON，已转人工确认。".to_string(),
            fix_hints: Vec::new(),
            questions: vec!["请人工确认 Analyst 输出并补充下一步处理意见。".to_string()],
            parse_error: Some(error.to_string()),
        },
    }
}

fn extract_json_object(value: &str) -> Option<&str> {
    let start = value.find('{')?;
    let end = value.rfind('}')?;
    (start <= end).then(|| &value[start..=end])
}

fn default_analyst_summary(verdict: &AnalystVerdict) -> String {
    match verdict {
        AnalystVerdict::NeedsFix => "Analyst 判定需要自动修复".to_string(),
        AnalystVerdict::NeedsHumanInput => "Analyst 判定需要人工补充信息".to_string(),
        AnalystVerdict::NoIssue => "Analyst 未发现阻塞问题".to_string(),
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
    severity: crate::product::coding_models::FindingSeverity,
    #[serde(default, alias = "file")]
    file_path: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default, alias = "description")]
    message: Option<String>,
    #[serde(default, alias = "recommendation")]
    required_action: Option<String>,
    #[serde(default)]
    source_stage: Option<CodingExecutionStage>,
    #[serde(default)]
    title: Option<String>,
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
            severity: self.severity,
            file_path: self.file_path,
            line: self.line,
            message: self
                .message
                .or(self.title)
                .unwrap_or_else(|| "review finding".to_string()),
            required_action: self.required_action,
            source_stage: self.source_stage.unwrap_or(default_source_stage),
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
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_models::CodingProviderRole;
    use crate::product::models::{ProviderConversationRef, ProviderConversationRole};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;
    use tempfile::tempdir;

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
