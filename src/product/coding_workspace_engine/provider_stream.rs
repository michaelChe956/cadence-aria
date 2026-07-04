use super::*;

impl CodingWorkspaceEngine {
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
        let role_run = self.store.create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some(node.id.clone()),
        )?;

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
        let stream_result = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                role_run: Some(&role_run),
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
            .await;
        let full_output = match stream_result {
            Ok(output) => output,
            Err(error) => {
                let (status, reason_code) = coder_role_run_failure_status(&error);
                let _ = self.store.update_role_run_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &role_run.id,
                    status,
                    reason_code,
                );
                return Err(error);
            }
        };
        let raw_provider_output_ref = self.store.save_provider_raw_output(
            &attempt.id,
            CodingExecutionStage::Coding,
            "coder_output",
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
        let completed_role_run = self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            CodingRoleRunStatus::Completed,
            None,
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Completed,
            Some("代码编写完成".to_string()),
        )
        .await?;
        self.emit_coder_output_chat_entry(
            &attempt,
            &node.id,
            &coder_provider,
            &completed_role_run,
            &full_output,
            &raw_provider_output_ref,
        )
        .await;
        Ok(attempt)
    }

    pub(crate) fn record_role_run_event(
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

    pub(crate) fn unresolved_provider_choice_error(
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

    pub(crate) async fn run_provider_stream_to_completion(
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
                    .run_legacy_stream_to_completion(
                        attempt,
                        node_id,
                        role_run,
                        provider,
                        legacy_input,
                        provider_name,
                        provider_role,
                    )
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
                                    answers: vec![],
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

    pub(crate) async fn run_legacy_stream_to_completion(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        role_run: Option<&CodingRoleRun>,
        provider: &dyn StreamingProviderAdapter,
        input: &AdapterInput,
        provider_name: &ProviderName,
        provider_role: CodingProviderRole,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let mut stream = provider
            .run_streaming(input, CancellationToken::new())
            .await?;
        self.record_role_run_event(
            attempt,
            role_run,
            CodingRoleRunEventType::ProviderStart,
            json!({
                "provider": provider_name,
                "role": format!("{provider_role:?}"),
                "mode": "legacy_stream"
            }),
        );
        let mut full_output = String::new();
        while let Some(chunk) = stream.recv().await {
            match chunk {
                StreamChunk::Text(content) => {
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
                StreamChunk::Done {
                    full_output: completed_output,
                } => {
                    let output = if completed_output.trim().is_empty() {
                        full_output
                    } else {
                        completed_output
                    };
                    let output_bytes = output.len();
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
                            "provider_session_id": null,
                            "output_bytes": output_bytes
                        }),
                    );
                    return Ok(output);
                }
                StreamChunk::Error(message) => {
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
            }
        }

        self.fail_provider_stream_ended(attempt, node_id).await
    }

    async fn emit_coder_output_chat_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        provider_name: &ProviderName,
        role_run: &CodingRoleRun,
        full_output: &str,
        raw_provider_output_ref: &str,
    ) {
        let completed_at = role_run
            .completed_at
            .clone()
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        let entry = CodingChatEntry {
            id: format!("{node_id}_coder_output"),
            attempt_id: attempt.id.clone(),
            node_id: Some(node_id.to_string()),
            role: CodingAgentRole::Author,
            entry_type: CodingEntryType::AssistantMessage,
            content: Some(full_output.to_string()),
            metadata: Some(serde_json::json!({
                "source": "coding",
                "provider": provider_name,
                "role_run_id": role_run.id,
                "run_no": role_run.run_no,
                "raw_provider_output_ref": raw_provider_output_ref,
                "started_at": role_run.started_at,
                "completed_at": completed_at
            })),
            created_at: completed_at,
        };
        self.save_and_emit_chat_entry(entry).await;
    }
}

fn coder_role_run_failure_status(
    error: &CodingWorkspaceEngineError,
) -> (CodingRoleRunStatus, Option<String>) {
    match error {
        CodingWorkspaceEngineError::Aborted => (
            CodingRoleRunStatus::Aborted,
            Some("abort_attempt".to_string()),
        ),
        CodingWorkspaceEngineError::ProviderStream(message)
            if message == "provider_choice_unresolved" =>
        {
            (
                CodingRoleRunStatus::Blocked,
                Some("provider_choice_unresolved".to_string()),
            )
        }
        CodingWorkspaceEngineError::ProviderStream(message) => {
            (CodingRoleRunStatus::Failed, Some(message.clone()))
        }
        other => (CodingRoleRunStatus::Failed, Some(other.to_string())),
    }
}
