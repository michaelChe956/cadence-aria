use super::*;

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

        let retry_context = ArtifactRetryContext {
            provider: provider.clone(),
            input: input.clone(),
            attempted: false,
        };
        let session = provider.start(input, self.cancel.clone()).await;
        self.drive_provider_session(ProviderSessionDriveInput {
            session,
            command_rx,
            node_id: Some(generation_node_id),
            agent: Some(self.session.author_provider.clone()),
            role: ProviderConversationRole::Author,
            artifact_retry: Some(retry_context),
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
                        ProviderEvent::Completed {
                            full_output,
                            provider_session_id,
                        } => {
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

    fn record_choice_response_audit(&mut self, input: ChoiceResponseAuditInput<'_>) {
        let content = build_choice_response_audit_message(&input);
        let msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();
        self.session.messages.push(SessionMessage {
            id: msg_id,
            role: "system".to_string(),
            content: content.clone(),
            checkpoint_id: None,
            created_at: now,
        });
        if let Some(store) = &self.lifecycle_store {
            let _ = store.append_workspace_message(
                &self.session.session_id,
                "system".to_string(),
                content,
            );
        }
    }

    pub(crate) fn workspace_artifact_blocking_reasons(&self, full_content: &str) -> Vec<String> {
        let artifact_markdown = extract_artifact_content(full_content);
        validate_workspace_artifact_constraints(&artifact_markdown, &self.session.workspace_type)
            .blocking_reasons()
    }

    pub(crate) async fn begin_artifact_retry_node(
        &mut self,
        previous_node_id: Option<&str>,
        agent: Option<ProviderName>,
        blocking_reasons: &[String],
    ) -> Option<String> {
        let previous_node = previous_node_id.and_then(|node_id| {
            self.timeline_nodes
                .iter()
                .find(|node| node.node_id == node_id)
                .cloned()
        })?;
        let _ = self.flush_stream_buffer(&previous_node.node_id).await;
        let artifact_name = workspace_type_title(&self.session.workspace_type);
        let summary = if blocking_reasons.is_empty() {
            format!("Provider 未返回有效的 {artifact_name} artifact")
        } else {
            format!(
                "Provider 未返回有效的 {artifact_name} artifact：{}",
                blocking_reasons.join("；")
            )
        };
        self.update_timeline_node(
            &previous_node.node_id,
            TimelineNodeStatus::Failed,
            Some(summary),
        )
        .await;

        let retry_attempt = previous_node
            .retry
            .as_ref()
            .map(|retry| retry.retry_attempt + 1)
            .unwrap_or(1);
        let retry_error_message = if blocking_reasons.is_empty() {
            format!("缺失或无效的 {artifact_name} artifact")
        } else {
            blocking_reasons.join("；")
        };
        Some(
            self.create_timeline_node_with_retry(
                TimelineNodeDraft {
                    node_type: previous_node.node_type.clone(),
                    agent,
                    stage: workspace_stage_from_ws_stage(&previous_node.stage),
                    round: previous_node.round,
                    title: format!("{artifact_name} 自动续写"),
                    summary: None,
                    status: TimelineNodeStatus::Active,
                },
                Some(TimelineNodeRetry {
                    retry_of_node_id: previous_node.node_id.clone(),
                    retry_attempt,
                    retry_reason: "自动续写缺失或无效 artifact".to_string(),
                    retry_error: TimelineNodeRetryError {
                        code: "workspace_artifact_invalid".to_string(),
                        message: retry_error_message,
                    },
                }),
            )
            .await,
        )
    }

    pub(crate) fn build_artifact_retry_input(
        &self,
        base_input: &StreamingProviderInput,
        previous_output: &str,
        provider_session_id: Option<String>,
    ) -> StreamingProviderInput {
        let mut input = base_input.clone();
        let blocking_reasons = self.workspace_artifact_blocking_reasons(previous_output);
        input.prompt = build_artifact_retry_prompt(
            &self.session.workspace_type,
            previous_output,
            &blocking_reasons,
        );
        if let Some(provider_session_id) = provider_session_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
        {
            input.resume_provider_session_id = Some(provider_session_id);
        }
        input
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
        if let Some(store) = &self.lifecycle_store
            && matches!(
                self.session.workspace_type,
                WorkspaceType::Story | WorkspaceType::Design
            )
        {
            let _ = store.append_version(AppendSpecVersionInput {
                project_id: self.session.project_id.clone(),
                issue_id: self.session.issue_id.clone(),
                entity_id: self.session.entity_id.clone(),
                markdown: artifact_markdown.clone(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            });
        }
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
}

struct ChoiceResponseAuditInput<'a> {
    request: Option<&'a ChoiceRequestData>,
    choice_id: &'a str,
    selected_option_ids: &'a [String],
    free_text: Option<&'a str>,
    answers: &'a [ChoiceAnswerData],
    node_id: Option<&'a str>,
    agent: Option<&'a ProviderName>,
    role: &'a ProviderConversationRole,
}

fn build_choice_response_audit_message(input: &ChoiceResponseAuditInput<'_>) -> String {
    let source = input
        .request
        .map(|request| request.source.as_str())
        .unwrap_or("unknown");
    let mut message = String::new();
    message.push_str("结构化交互审计记录（daemon 捕获）\n");
    message.push_str("- audit_kind: provider_choice_response\n");
    message.push_str(&format!("- choice_id: {}\n", input.choice_id));
    message.push_str(&format!("- source: {source}\n"));
    message.push_str(&format!("- provider_role: {}\n", role_label(input.role)));
    if let Some(agent) = input.agent {
        message.push_str(&format!("- agent: {agent:?}\n"));
    }
    if let Some(node_id) = input.node_id {
        message.push_str(&format!("- node_id: {node_id}\n"));
    }
    if let Some(request) = input.request {
        message.push_str(&format!(
            "- request_prompt: {}\n",
            audit_one_line(&request.prompt)
        ));
    }
    message.push_str("- answers:\n");
    append_choice_answers_audit(
        &mut message,
        input.request,
        input.selected_option_ids,
        input.free_text,
        input.answers,
    );
    message.push_str(
        "\n说明：以上记录由 daemon 在 ProviderEvent::ChoiceRequest 与 \
         ProviderCommand::ChoiceResponse 之间捕获，是后续 reviewer 核对 artifact 中 \
         author-decision/source claims 的可审计来源。\n",
    );
    message
}

fn append_choice_answers_audit(
    message: &mut String,
    request: Option<&ChoiceRequestData>,
    selected_option_ids: &[String],
    free_text: Option<&str>,
    answers: &[ChoiceAnswerData],
) {
    let questions = request
        .map(|request| request.effective_questions())
        .unwrap_or_default();
    let fallback_options = request
        .map(|request| request.options.as_slice())
        .unwrap_or_default();
    let fallback_free_text = free_text.and_then(trimmed_non_empty).map(str::to_string);
    let normalized_answers = if answers.is_empty() {
        if selected_option_ids.is_empty() && fallback_free_text.is_none() {
            Vec::new()
        } else {
            vec![ChoiceAnswerData {
                question_id: "default".to_string(),
                selected_option_ids: selected_option_ids.to_vec(),
                free_text: fallback_free_text,
            }]
        }
    } else {
        answers.to_vec()
    };

    if normalized_answers.is_empty() {
        message.push_str("  - 未捕获到显式答案。\n");
        return;
    }

    for answer in normalized_answers {
        let question = questions
            .iter()
            .find(|question| question.id == answer.question_id);
        let options = question
            .map(|question| question.options.as_slice())
            .unwrap_or(fallback_options);
        message.push_str(&format!("  - question_id: {}\n", answer.question_id));
        if let Some(question) = question {
            message.push_str(&format!(
                "    question: {}\n",
                audit_one_line(&question.prompt)
            ));
        }
        if !answer.selected_option_ids.is_empty() {
            let labels = answer
                .selected_option_ids
                .iter()
                .map(|option_id| choice_option_label(option_id, options))
                .collect::<Vec<_>>()
                .join("; ");
            message.push_str(&format!("    selected: {labels}\n"));
        }
        if let Some(free_text) = answer.free_text.as_deref().and_then(trimmed_non_empty) {
            message.push_str(&format!("    free_text: {}\n", audit_one_line(free_text)));
        }
    }
}

fn choice_option_label(option_id: &str, options: &[ChoiceOptionData]) -> String {
    match options.iter().find(|option| option.id == option_id) {
        Some(option) if option.label.trim().is_empty() => option_id.to_string(),
        Some(option) => format!("{option_id} = {}", audit_one_line(&option.label)),
        None => format!("{option_id} = <unknown option>"),
    }
}

fn audit_one_line(value: &str) -> String {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn trimmed_non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn role_label(role: &ProviderConversationRole) -> &'static str {
    match role {
        ProviderConversationRole::Author => "author",
        ProviderConversationRole::Reviewer => "reviewer",
        ProviderConversationRole::Coder => "coder",
        ProviderConversationRole::Tester => "tester",
        ProviderConversationRole::Analyst => "analyst",
        ProviderConversationRole::CodeReviewer => "code_reviewer",
        ProviderConversationRole::InternalReviewer => "internal_reviewer",
    }
}

mod work_item_plan;
