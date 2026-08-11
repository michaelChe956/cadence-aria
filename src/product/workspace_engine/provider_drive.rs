use super::*;

mod artifact_retry;
mod choice_audit;

use crate::product::lifecycle_store::spec::ExistingSpecRecord;
use crate::product::lifecycle_store::{AggregateDesignSpecScope, AggregateStorySpecScope};
use crate::product::logical_codebase::PlanningContextSnapshotStore;
use crate::product::workspace_engine::aggregate_output_parser::{
    parse_design_aggregate_output, parse_story_aggregate_output,
};
use choice_audit::ChoiceResponseAuditInput;

impl WorkspaceEngine {
    pub async fn handle_user_message(
        &mut self,
        content: String,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
        self.handle_author_message_with_prompt_mode(
            content,
            provider,
            command_rx,
            AuthorPromptMode::FullConversation,
        )
        .await;
    }

    pub async fn handle_author_choice_followup_message(
        &mut self,
        content: String,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
        self.handle_author_message_with_prompt_mode(
            content,
            provider,
            command_rx,
            AuthorPromptMode::DeltaOnly,
        )
        .await;
    }

    pub(crate) async fn handle_author_message_with_prompt_mode(
        &mut self,
        content: String,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
        prompt_mode: AuthorPromptMode,
    ) {
        let content = normalize_generation_prompt(content, &self.session.workspace_type);
        let msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();

        let user_msg = SessionMessage {
            id: msg_id.clone(),
            role: "user".to_string(),
            content: content.clone(),
            checkpoint_id: None,
            created_at: now.clone(),
        };
        self.session.messages.push(user_msg);
        if let Some(store) = &self.lifecycle_store {
            let _ = store.append_workspace_message(
                &self.session.session_id,
                "user".to_string(),
                content.clone(),
            );
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Running,
            );
        }

        if self.session.stage != WorkspaceStage::Running {
            self.complete_active_node(Some("上下文已确认".to_string()))
                .await;
            self.transition_stage(WorkspaceStage::Running).await;
        }

        let generation_node_id = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::AuthorRun,
                agent: Some(self.session.author_provider.clone()),
                stage: WorkspaceStage::Running,
                round: None,
                title: format!(
                    "{} 生成",
                    workspace_type_title(&self.session.workspace_type)
                ),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await;

        let input = match self.build_streaming_input(&content, prompt_mode) {
            Ok(input) => input,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
        let _ = self
            .persist_prompt_snapshot(&generation_node_id, input.prompt.clone())
            .await;
        self.emit_execution_event(
            provider_prompt_event(
                &generation_node_id,
                input.prompt.clone(),
                prompt_mode.prompt_event_detail(),
            ),
            Some(generation_node_id.clone()),
            Some(self.session.author_provider.clone()),
        )
        .await;

        let retry_context =
            (self.session.author_provider != ProviderName::Pi).then(|| ArtifactRetryContext {
                provider: provider.clone(),
                input: input.clone(),
                attempted: false,
            });
        let session = provider.start(input, self.cancel.clone()).await;
        self.drive_provider_session(ProviderSessionDriveInput {
            session,
            command_rx,
            node_id: Some(generation_node_id),
            agent: Some(self.session.author_provider.clone()),
            role: ProviderConversationRole::Author,
            artifact_retry: retry_context,
            revision_resume_fallback: None,
        })
        .await;
    }

    /// Task 11:逻辑代码库 planning 栈入口。与 `handle_author_message_with_prompt_mode`
    /// 对称,但 provider 会话改由 `LogicalCodebaseProviderGateway::start_streaming` 启动,
    /// 使真实启动唯一由 gateway 产出并留 audit。
    ///
    /// 调用方(Web 接入 task)在确认 issue 属于逻辑代码库后,构造
    /// `SessionLaunchRequest`、经 `gateway.validate` 产出 validated policy,再组装
    /// `ValidatedStreamingProviderInput` 传入;本方法仅消费 validated input 启动并驱动。
    /// 传统单仓/非逻辑 issue 仍走 `handle_user_message`/`handle_author_message_with_prompt_mode`
    /// 的直接 `provider.start` 路径。
    #[allow(dead_code)]
    pub(crate) async fn drive_author_provider_session_via_gateway(
        &mut self,
        validated_input: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        command_rx: mpsc::Receiver<ProviderCommand>,
        generation_node_id: String,
    ) {
        let gateway = self
            .logical_provider_gateway
            .clone()
            .expect("logical provider gateway must be injected before driving via gateway");
        let session = gateway
            .start_streaming(validated_input, self.cancel.clone())
            .await
            .map_err(
                |error| crate::cross_cutting::provider_adapter::ProviderAdapterError {
                    code: crate::protocol::provider_errors::ProviderErrorCode::ProviderUnavailable,
                    details: error.to_string(),
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: None,
                    timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
                    duration_ms: 0,
                },
            );
        self.drive_provider_session(ProviderSessionDriveInput {
            session,
            command_rx,
            node_id: Some(generation_node_id),
            agent: Some(self.session.author_provider.clone()),
            role: ProviderConversationRole::Author,
            artifact_retry: None,
            revision_resume_fallback: None,
        })
        .await;
    }

    pub(crate) fn should_retry_missing_workspace_artifact(&self, full_content: &str) -> bool {
        if !self.workspace_requires_artifact_gate() || full_content.trim().is_empty() {
            return false;
        }

        let artifact_markdown = extract_artifact_content(full_content);
        !content_has_complete_workspace_artifact(&artifact_markdown, &self.session.workspace_type)
            && detect_author_choice_request(full_content, &self.session.workspace_type).is_none()
    }

    pub(crate) async fn drive_provider_session(&mut self, input: ProviderSessionDriveInput) {
        let ProviderSessionDriveInput {
            session,
            mut command_rx,
            mut node_id,
            agent,
            role,
            mut artifact_retry,
            mut revision_resume_fallback,
        } = input;
        let mut session = match session {
            Ok(session) => session,
            Err(error) => {
                let _ = self
                    .event_tx
                    .send(EngineEvent::Error {
                        message: error.details.clone(),
                    })
                    .await;
                self.finish_failed_run().await;
                return;
            }
        };

        let assistant_msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let mut full_content = String::new();
        let cancel = self.cancel.clone();
        let mut events_open = true;
        let mut commands_open = true;
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();
        let mut pending_choice_requests: HashMap<String, ChoiceRequestData> = HashMap::new();

        while events_open {
            tokio::select! {
                _ = cancel.cancelled() => {
                    if let Some(node_id) = node_id.as_deref() {
                        let _ = self.flush_stream_buffer(node_id).await;
                    }
                    self.finish_aborted_run().await;
                    return;
                }
                command = command_rx.recv(), if commands_open => {
                    match command {
                        Some(ProviderCommand::Abort) => {
                            let _ = session.commands.send(ProviderCommand::Abort).await;
                            cancel.cancel();
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            self.finish_aborted_run().await;
                            return;
                        }
                        Some(ProviderCommand::PermissionResponse {
                            id,
                            approved,
                            reason,
                        }) => {
                            tracing::info!(permission_id = %id, "engine forwarding permission response");
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self
                                    .persist_permission_response(
                                        node_id,
                                        id.clone(),
                                        serde_json::json!({
                                            "approved": approved,
                                            "reason": reason.clone(),
                                        }),
                                    )
                                    .await;
                            }
                            if session.commands.send(ProviderCommand::PermissionResponse {
                                id,
                                approved,
                                reason,
                            }).await.is_err() {
                                commands_open = false;
                            }
                        }
                        Some(ProviderCommand::ChoiceResponse {
                            id,
                            selected_option_ids,
                            free_text,
                            answers,
                        }) => {
                            tracing::info!(choice_id = %id, "engine forwarding choice response");
                            let choice_id = id.clone();
                            let choice_request = pending_choice_requests.remove(&id);
                            self.record_choice_response_audit(ChoiceResponseAuditInput {
                                request: choice_request.as_ref(),
                                choice_id: &id,
                                selected_option_ids: &selected_option_ids,
                                free_text: free_text.as_deref(),
                                answers: &answers,
                                node_id: node_id.as_deref(),
                                agent: agent.as_ref(),
                                role: &role,
                            });
                            eprintln!(
                                "[aria-choice-diag] engine forwarding author choice_response id={} selected={:?} free_text_present={}",
                                choice_id,
                                selected_option_ids,
                                free_text.as_ref().is_some_and(|text| !text.trim().is_empty())
                            );
                            if session.commands.send(ProviderCommand::ChoiceResponse {
                                id,
                                selected_option_ids,
                                free_text,
                                answers,
                            }).await.is_err() {
                                eprintln!(
                                    "[aria-choice-diag] engine failed to forward author choice_response id={} to provider session",
                                    choice_id
                                );
                                commands_open = false;
                            } else {
                                eprintln!(
                                    "[aria-choice-diag] engine forwarded author choice_response id={} to provider session",
                                    choice_id
                                );
                            }
                        }
                        Some(ProviderCommand::ToolResult(_)) => {}
                        None => commands_open = false,
                    }
                }
                event = session.events.recv() => {
                    let Some(event) = event else {
                        events_open = false;
                        continue;
                    };

                    match event {
                        ProviderEvent::TextDelta { content } => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.buffer_stream_chunk(node_id, content.clone()).await;
                            }
                            full_content.push_str(&content);
                            let _ = self
                                .event_tx
                                .send(EngineEvent::StreamChunk {
                                    role: "assistant".to_string(),
                                    content,
                                    node_id: node_id.clone(),
                                })
                                .await;
                        }
                        ProviderEvent::PermissionRequest(request) => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self
                                    .persist_permission_request(
                                        node_id,
                                        request.id.clone(),
                                        serde_json::json!({
                                            "tool_name": request.tool_name.clone(),
                                            "description": request.description.clone(),
                                            "risk_level": risk_level_text(&request.risk_level),
                                        }),
                                    )
                                    .await;
                            }
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ExecutionEvent {
                                    event: ProviderExecutionEvent {
                                        event_id: format!("permission_{}", request.id),
                                        kind: ProviderExecutionEventKind::Command,
                                        status: ProviderExecutionEventStatus::WaitingApproval,
                                        title: "Waiting for permission".to_string(),
                                        detail: Some(request.description.clone()),
                                        command: Some(request.tool_name.clone()),
                                        cwd: self
                                            .session
                                            .repository_path
                                            .as_ref()
                                            .map(|path| path.display().to_string()),
                                        output: None,
                                        exit_code: None,
                                    },
                                    node_id: node_id.clone(),
                                    agent: agent.clone(),
                                })
                                .await;
                            let _ = self
                                .event_tx
                                .send(EngineEvent::PermissionRequest {
                                    id: request.id,
                                    tool_name: request.tool_name,
                                    description: request.description,
                                    risk_level: request.risk_level,
                                })
                                .await;
                        }
                        ProviderEvent::ChoiceRequest(request) => {
                            let questions = request.effective_questions();
                            pending_choice_requests.insert(request.id.clone(), request.clone());
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ChoiceRequest {
                                    id: request.id,
                                    prompt: request.prompt,
                                    options: request.options,
                                    allow_multiple: request.allow_multiple,
                                    allow_free_text: request.allow_free_text,
                                    questions,
                                    source: request.source,
                                })
                                .await;
                        }
                        ProviderEvent::StatusChanged(status) => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ProviderStatus { status })
                                .await;
                        }
                        ProviderEvent::Execution(event) => {
                            self.emit_execution_event(event, node_id.clone(), agent.clone()).await;
                        }
                        ProviderEvent::ToolCall(call) => {
                            tool_call_titles.insert(call.id.clone(), call.tool_name.clone());
                            if let Some(command) = extract_tool_command(&call.input) {
                                tool_call_commands.insert(call.id.clone(), command);
                            }
                            self
                                .emit_execution_event(
                                    execution_event_from_tool_call(call),
                                    node_id.clone(),
                                    agent.clone(),
                                )
                                .await;
                        }
                        ProviderEvent::ToolResult(result) => {
                            let title = tool_call_titles
                                .get(&result.tool_use_id)
                                .cloned()
                                .unwrap_or_else(|| "Tool result".to_string());
                            let command = tool_call_commands.get(&result.tool_use_id).cloned();
                            self
                                .emit_execution_event(
                                    execution_event_from_tool_result(result, title, command),
                                    node_id.clone(),
                                    agent.clone(),
                                )
                                .await;
                        }
                        ProviderEvent::Completed(completion) => {
                            let full_output = completion.full_output;
                            let provider_session_id = completion.provider_session_id;
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            let completed_provider_session_id = provider_session_id.clone();
                            if let Some(provider) = agent.clone() {
                                self.record_provider_session(
                                    role.clone(),
                                    provider,
                                    provider_session_id,
                                    node_id.clone(),
                                )
                                .await;
                            }
                            let completed_output = if self.workspace_requires_artifact_gate()
                                && !content_has_complete_workspace_artifact(
                                    &extract_artifact_content(&full_output),
                                    &self.session.workspace_type,
                                )
                                && content_has_complete_workspace_artifact(
                                    &extract_artifact_content(&full_content),
                                    &self.session.workspace_type,
                                ) {
                                full_content.clone()
                            } else {
                                full_output
                            };

                            let retry_start = if self
                                .should_retry_missing_workspace_artifact(&completed_output)
                            {
                                if let Some(context) = artifact_retry.as_mut() {
                                    if context.attempted {
                                        None
                                    } else {
                                        context.attempted = true;
                                        let retry_input = self.build_artifact_retry_input(
                                            &context.input,
                                            &completed_output,
                                            completed_provider_session_id.clone(),
                                        );
                                        context.input = retry_input.clone();
                                        Some((context.provider.clone(), retry_input))
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            if let Some((provider, retry_input)) = retry_start {
                                let blocking_reasons = self
                                    .workspace_artifact_blocking_reasons(&completed_output);
                                node_id = self
                                    .begin_artifact_retry_node(
                                        node_id.as_deref(),
                                        agent.clone(),
                                        &blocking_reasons,
                                    )
                                    .await
                                    .or(node_id);
                                if let Some(node_id) = node_id.as_deref() {
                                    let _ = self
                                        .persist_prompt_snapshot(node_id, retry_input.prompt.clone())
                                        .await;
                                    self.emit_execution_event(
                                        provider_prompt_event(
                                            node_id,
                                            retry_input.prompt.clone(),
                                            "自动续写缺失 artifact 的提示词",
                                        ),
                                        Some(node_id.to_string()),
                                        agent.clone(),
                                    )
                                    .await;
                                }
                                match provider.start(retry_input, self.cancel.clone()).await {
                                    Ok(next_session) => {
                                        session = next_session;
                                        full_content.clear();
                                        tool_call_titles.clear();
                                        tool_call_commands.clear();
                                        continue;
                                    }
                                    Err(error) => {
                                        let _ = self
                                            .event_tx
                                            .send(EngineEvent::Error {
                                                message: error.details.clone(),
                                            })
                                            .await;
                                        if let Some(node_id) = node_id.as_deref() {
                                            self.update_timeline_node(
                                                node_id,
                                                TimelineNodeStatus::Failed,
                                                Some("Provider 自动续写启动失败".to_string()),
                                            )
                                            .await;
                                        }
                                        self.finish_failed_run().await;
                                        return;
                                    }
                                }
                            }

                            let artifact_retry_attempted =
                                artifact_retry.as_ref().is_some_and(|context| context.attempted);
                            self.complete_assistant_message(
                                assistant_msg_id,
                                completed_output,
                                artifact_retry_attempted,
                            )
                                .await;
                            return;
                        }
                        ProviderEvent::Failed { message } => {
                            let retry_provider =
                                revision_resume_fallback.as_mut().and_then(|context| {
                                    if !context.attempted && is_codex_resume_stall_failure(&message)
                                    {
                                        context.attempted = true;
                                        Some(context.provider.clone())
                                    } else {
                                        None
                                    }
                                });
                            if let Some(provider) = retry_provider {
                                let retry_input = match self.build_revision_input_without_resume() {
                                    Ok(input) => input,
                                    Err(error) => {
                                        let _ = self
                                            .event_tx
                                            .send(EngineEvent::Error { message: error })
                                            .await;
                                        self.finish_failed_run().await;
                                        return;
                                    }
                                };
                                if let Some(context) = artifact_retry.as_mut() {
                                    context.input = retry_input.clone();
                                }
                                if let Some(node_id) = node_id.as_deref() {
                                    let _ = self
                                        .persist_prompt_snapshot(node_id, retry_input.prompt.clone())
                                        .await;
                                    self.emit_execution_event(
                                        provider_prompt_event(
                                            node_id,
                                            retry_input.prompt.clone(),
                                            "Codex resume 无事件，改用新 thread 的完整返修提示词",
                                        ),
                                        Some(node_id.to_string()),
                                        agent.clone(),
                                    )
                                    .await;
                                }
                                match provider.start(retry_input, self.cancel.clone()).await {
                                    Ok(next_session) => {
                                        session = next_session;
                                        full_content.clear();
                                        tool_call_titles.clear();
                                        tool_call_commands.clear();
                                        continue;
                                    }
                                    Err(error) => {
                                        let _ = self
                                            .event_tx
                                            .send(EngineEvent::Error {
                                                message: error.details.clone(),
                                            })
                                            .await;
                                        if let Some(node_id) = node_id.as_deref() {
                                            self.update_timeline_node(
                                                node_id,
                                                TimelineNodeStatus::Failed,
                                                Some("Provider fresh retry 启动失败".to_string()),
                                            )
                                            .await;
                                        }
                                        self.finish_failed_run().await;
                                        return;
                                    }
                                }
                            }
                            let _ = self
                                .event_tx
                                .send(EngineEvent::Error { message })
                                .await;
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                                self.update_timeline_node(
                                    node_id,
                                    TimelineNodeStatus::Failed,
                                    Some("Provider 运行失败".to_string()),
                                )
                                .await;
                            }
                            self.finish_failed_run().await;
                            return;
                        }
                        ProviderEvent::ProtocolError {
                            code,
                            message,
                            context,
                        } => {
                            let _ = self
                                .event_tx
                                .send(EngineEvent::ProtocolError {
                                    code,
                                    message,
                                    context,
                                })
                                .await;
                        }
                        ProviderEvent::PermissionTimeout { permission_id } => {
                            self.handle_permission_timeout(permission_id, node_id.clone())
                                .await;
                            return;
                        }
                    }
                }
            }
        }

        if cancel.is_cancelled() {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            self.finish_aborted_run().await;
            return;
        }

        if full_content.is_empty() {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            self.finish_empty_assistant_output().await;
        } else {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            self.complete_assistant_message(assistant_msg_id, full_content, false)
                .await;
        }
    }

    pub(crate) async fn complete_assistant_message(
        &mut self,
        assistant_msg_id: String,
        full_content: String,
        artifact_retry_attempted: bool,
    ) {
        if self.cancel.is_cancelled() {
            self.finish_aborted_run().await;
            return;
        }

        if full_content.is_empty() {
            self.finish_empty_assistant_output().await;
            return;
        }

        let assistant_msg = SessionMessage {
            id: assistant_msg_id.clone(),
            role: "assistant".to_string(),
            content: full_content.clone(),
            checkpoint_id: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        self.session.messages.push(assistant_msg);
        if let Some(store) = &self.lifecycle_store {
            let _ = store.append_workspace_message(
                &self.session.session_id,
                "assistant".to_string(),
                full_content.clone(),
            );
        }

        if let Some(choice) =
            detect_author_choice_request(&full_content, &self.session.workspace_type).map(
                |(prompt, options)| PendingAuthorChoice {
                    id: format!("author_choice_{}", assistant_msg_id),
                    prompt,
                    options,
                    source_node_id: self.active_node_id.clone(),
                },
            )
        {
            if let Some(node_id) = choice.source_node_id.as_deref() {
                self.update_timeline_node(
                    node_id,
                    TimelineNodeStatus::Paused,
                    Some("等待用户选择".to_string()),
                )
                .await;
            }
            self.pending_author_choice = Some(choice.clone());
            let _ = self
                .event_tx
                .send(EngineEvent::ChoiceRequest {
                    id: choice.id,
                    prompt: choice.prompt.clone(),
                    options: choice.options.clone(),
                    allow_multiple: false,
                    allow_free_text: true,
                    questions: vec![ChoiceQuestionData {
                        id: "default".to_string(),
                        prompt: choice.prompt,
                        options: choice.options,
                        allow_multiple: false,
                        allow_free_text: true,
                    }],
                    source: ChoiceRequestSource::TextFallback,
                })
                .await;
            return;
        }

        self.pending_author_choice = None;
        let artifact_markdown = extract_artifact_content(&full_content);
        if self.workspace_requires_artifact_gate() {
            let report = validate_workspace_artifact_constraints(
                &artifact_markdown,
                &self.session.workspace_type,
            );
            if !report.passed {
                let blocking_reasons = report.blocking_reasons();
                if artifact_retry_attempted {
                    self.finish_invalid_workspace_artifact_after_retry(&blocking_reasons)
                        .await;
                } else {
                    self.finish_invalid_workspace_artifact(&blocking_reasons)
                        .await;
                }
                return;
            }
        }
        let aggregate_write_back_diagnostic = if let Some(store) = &self.lifecycle_store
            && matches!(
                self.session.workspace_type,
                WorkspaceType::Story | WorkspaceType::Design
            ) {
            // 方案X阶段2：AI run 后解析 structured output，回写 involved/change_order
            // 到 Spec record（复用 Task 3 parse_* 与 Task 2 update_*）。
            // 约束4：回写失败/缺 tag 不沿用 `let _ =` 静默吞，转为可见诊断；
            // 单仓（logical_codebase_ref 为 None）回写跳过，append_version 行为不变。
            let diagnostic = match self.write_back_aggregate_output(store, &artifact_markdown) {
                Ok(Some(message)) => Some(message),
                Ok(None) => None,
                Err(error) => Some(format!("聚合代码库 involved 回写失败：{error}")),
            };
            let _ = store.append_version(AppendSpecVersionInput {
                project_id: self.session.project_id.clone(),
                issue_id: self.session.issue_id.clone(),
                entity_id: self.session.entity_id.clone(),
                markdown: artifact_markdown.clone(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            });
            diagnostic
        } else {
            None
        };
        self.update_artifact(ArtifactPayload::Markdown {
            markdown: artifact_markdown.clone(),
            diff: None,
        })
        .await;

        let message_index = self.session.messages.len() as u32;
        let artifact_snapshot = self.session.artifact.as_ref();
        let checkpoint = self.checkpoint_store.create_checkpoint(
            &self.session.session_id,
            message_index,
            artifact_snapshot,
            WorkspaceStage::AuthorConfirm.as_str(),
        );

        let checkpoint_id = match checkpoint {
            Ok(cp) => {
                if let Some(last) = self.session.messages.last_mut() {
                    last.checkpoint_id = Some(cp.id.clone());
                }
                cp.id
            }
            Err(e) => {
                let _ = self
                    .event_tx
                    .send(EngineEvent::Error {
                        message: format!("checkpoint error: {e}"),
                    })
                    .await;
                return;
            }
        };

        // 约束4：聚合回写诊断在 checkpoint 之后追加，避免干扰 checkpoint 对最后一条
        // assistant 消息的 checkpoint_id 绑定。
        if let Some(diagnostic) = aggregate_write_back_diagnostic {
            self.append_aggregate_write_back_diagnostic(&diagnostic);
        }

        let node_id = self.active_node_id.clone();
        let _ = self
            .event_tx
            .send(EngineEvent::MessageComplete {
                message_id: assistant_msg_id,
                checkpoint_id,
                node_id,
            })
            .await;
        self.complete_active_node(Some("生成完成".to_string()))
            .await;
        self.enter_author_confirm(Some("等待用户确认 author 结果".to_string()))
            .await;
    }

    /// 方案X阶段2：AI run 完成后解析 structured output，将 AI 声明的 involved/change_order
    /// 回写到 Spec record（Task 4 集成点，复用 Task 3 的 parse_* 与 Task 2 的 update_*）。
    ///
    /// 仅对**聚合代码库** Story/Design 生效（record.logical_codebase_ref 非空）；传统单仓
    /// （logical_codebase_ref 为 None）不执行回写，保持既有 append_version 行为不变。
    ///
    /// 返回：
    /// - `Ok(None)`：回写成功，或回写不适用（单仓/非 Story/Design），无需诊断。
    /// - `Ok(Some(message))`：回写被跳过但需可见——AI 未产出结构化 involved（缺 tag /
    ///   schema 非法，REQ-PLN-04）或规划上下文 snapshot 缺失，消息供调用方记录诊断。
    /// - `Err(ProductStoreError)`：真实 store 错误（加载 record/snapshot 失败、update
    ///   校验失败），由调用方转为可见诊断（约束4：不沿用 `let _ =` 静默吞）。
    fn write_back_aggregate_output(
        &self,
        store: &LifecycleStore,
        artifact_markdown: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        if !matches!(
            self.session.workspace_type,
            WorkspaceType::Story | WorkspaceType::Design
        ) {
            return Ok(None);
        }
        let project_id = &self.session.project_id;
        let issue_id = &self.session.issue_id;
        let entity_id = &self.session.entity_id;

        // 从 record 读 logical_codebase_ref（单仓 None → 跳过回写）。
        let record = store.load_existing_spec(project_id, issue_id, entity_id)?;
        let logical_codebase_ref = match &record {
            ExistingSpecRecord::Story { record, .. } => record.logical_codebase_ref,
            ExistingSpecRecord::Design { record, .. } => record.logical_codebase_ref,
        };
        let Some(logical_codebase_ref) = logical_codebase_ref else {
            return Ok(None);
        };

        // 从 snapshot 读 effective_member_ids（权威；缺失 → 可见诊断，无法校验）。
        let effective_member_ids = match PlanningContextSnapshotStore::new(store.app_paths())
            .load(project_id, issue_id)?
        {
            Some(snapshot) => snapshot.effective_member_ids,
            None => {
                return Ok(Some(
                    "聚合代码库回写跳过：规划上下文 snapshot 缺失，无法校验 effective_member_ids"
                        .to_string(),
                ));
            }
        };

        match self.session.workspace_type {
            WorkspaceType::Story => {
                let output = match parse_story_aggregate_output(artifact_markdown) {
                    Ok(output) => output,
                    Err(error) => {
                        return Ok(Some(format!(
                            "Story 聚合回写跳过：AI 未产出结构化 involved（REQ-PLN-04）：{error:?}"
                        )));
                    }
                };
                let scope = AggregateStorySpecScope {
                    logical_codebase_ref,
                    effective_member_ids,
                    involved_repository_ids: output.involved_repository_ids,
                    focus_repository_id: output.focus_repository_id,
                };
                store.update_story_spec_aggregate(project_id, issue_id, entity_id, &scope)?;
            }
            WorkspaceType::Design => {
                let output = match parse_design_aggregate_output(artifact_markdown) {
                    Ok(output) => output,
                    Err(error) => {
                        return Ok(Some(format!(
                            "Design 聚合回写跳过：AI 未产出结构化 involved（REQ-PLN-04）：{error:?}"
                        )));
                    }
                };
                let scope = AggregateDesignSpecScope {
                    logical_codebase_ref,
                    effective_member_ids,
                    involved_repository_ids: output.involved_repository_ids,
                    change_order: output.change_order,
                };
                store.update_design_spec_aggregate(project_id, issue_id, entity_id, &scope)?;
            }
            _ => return Ok(None),
        }
        Ok(None)
    }

    /// 将聚合回写诊断追加为 session 的 system 消息（约束4：回写失败可见，不静默吞）。
    /// 需在 checkpoint 创建**之后**调用：避免改变 `session.messages.len()` 或末尾消息，
    /// 干扰 checkpoint 对最后一条 assistant 消息的 checkpoint_id 绑定。
    fn append_aggregate_write_back_diagnostic(&mut self, message: &str) {
        tracing::warn!(message, "aggregate involved write-back diagnostic");
        let msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();
        self.session.messages.push(SessionMessage {
            id: msg_id,
            role: "system".to_string(),
            content: message.to_string(),
            checkpoint_id: None,
            created_at: now,
        });
        if let Some(store) = &self.lifecycle_store {
            let _ = store.append_workspace_message(
                &self.session.session_id,
                "system".to_string(),
                message.to_string(),
            );
        }
    }
}

mod work_item_plan;
