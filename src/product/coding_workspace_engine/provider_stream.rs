use super::*;

mod persistence;

impl CodingWorkspaceEngine {
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
            fresh_retry,
            timeout,
            timeout_reason_code,
            suppress_failure_side_effects,
        } = run;
        self.store.ensure_provider_run_allowed(attempt)?;
        let mut active_legacy_input = legacy_input.clone();
        let mut active_input = input;
        let mut fresh_retry = fresh_retry;
        'provider_attempt: loop {
            let cancel = self.cancellation.child_token();
            self.record_role_run_event(
                attempt,
                role_run,
                CodingRoleRunEventType::ProviderPrompt,
                json!({
                    "provider": provider_name,
                    "role": format!("{provider_role:?}"),
                    "output_schema": active_legacy_input.output_schema.clone(),
                    "prompt": active_legacy_input.prompt.clone()
                }),
            );
            let start_result = if let Some(duration) = timeout {
                tokio::select! {
                    biased;
                    result = provider.start(active_input.clone(), cancel.clone()) => result,
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
                    _ = self.cancellation.cancelled() => {
                        cancel.cancel();
                        self.persist_provider_cancellation(attempt, role_run, "provider_start")?;
                        return Err(CodingWorkspaceEngineError::Aborted);
                    }
                }
            } else {
                tokio::select! {
                    biased;
                    result = provider.start(active_input.clone(), cancel.clone()) => result,
                    _ = self.cancellation.cancelled() => {
                        cancel.cancel();
                        self.persist_provider_cancellation(attempt, role_run, "provider_start")?;
                        return Err(CodingWorkspaceEngineError::Aborted);
                    }
                }
            };
            let mut session = match start_result {
                Ok(session) => {
                    if let Err(error) = self.record_provider_start_required(
                        attempt,
                        role_run,
                        json!({
                            "provider": provider_name,
                            "role": format!("{provider_role:?}")
                        }),
                    ) {
                        let message = error.to_string();
                        cancel.cancel();
                        drop(session);
                        return self
                            .fail_provider_stream_with_ownership(
                                attempt,
                                node_id,
                                suppress_failure_side_effects,
                                message,
                            )
                            .await;
                    }
                    session
                }
                Err(error)
                    if provider_start_is_not_implemented(&error)
                        && allow_legacy_stream_fallback =>
                {
                    return self
                        .run_legacy_stream_to_completion(
                            attempt,
                            node_id,
                            role_run,
                            provider,
                            &active_legacy_input,
                            provider_name,
                            provider_role,
                            suppress_failure_side_effects,
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
                    return self
                        .fail_provider_stream_with_ownership(
                            attempt,
                            node_id,
                            suppress_failure_side_effects,
                            message,
                        )
                        .await;
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
                    biased;
                    _ = self.cancellation.cancelled() => {
                        let _ = session.commands.try_send(ProviderCommand::Abort);
                        cancel.cancel();
                        self.persist_provider_cancellation(attempt, role_run, "provider_stream")?;
                        return Err(CodingWorkspaceEngineError::Aborted);
                    }
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
                                let _ = session.commands.try_send(ProviderCommand::Abort);
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
                                self.persist_provider_cancellation(
                                    attempt,
                                    role_run,
                                    "provider_command",
                                )?;
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
                                if send_provider_command_with_cancellation(
                                    &session.commands,
                                    ProviderCommand::ChoiceResponse {
                                        id: id.clone(),
                                        selected_option_ids: selected_option_ids.clone(),
                                        free_text: free_text.clone(),
                                        answers: vec![],
                                    },
                                    &self.cancellation,
                                )
                                .await
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
                                if !forward_runner_command_to_provider(
                                    command,
                                    &session.commands,
                                    &self.cancellation,
                                ).await {
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
                            return self
                                .fail_provider_stream_ended_with_ownership(
                                    attempt,
                                    node_id,
                                    suppress_failure_side_effects,
                                )
                                .await;
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
                            ProviderEvent::Completed(completion) => {
                                let completed_output = completion.full_output;
                                let provider_session_id = completion.provider_session_id;
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
                                if provider_role == CodingProviderRole::Coder
                                    && provider_name == &ProviderName::Codex
                                    && active_input.resume_provider_session_id.is_some()
                                    && crate::cross_cutting::codex_provider::is_resume_stall_failure(
                                        &message,
                                    )
                                    && let Some(fresh) = fresh_retry.take()
                                {
                                    self.record_role_run_event(
                                        attempt,
                                        role_run,
                                        CodingRoleRunEventType::ProviderFailed,
                                        json!({
                                            "code": "codex_resume_stall_fresh_retry",
                                            "message": message,
                                            "resume_provider_session_id": active_input
                                                .resume_provider_session_id
                                                .clone()
                                        }),
                                    );
                                    cancel.cancel();
                                    self.clear_attempt_provider_conversation(
                                        attempt,
                                        &CodingProviderRole::Coder,
                                        provider_name,
                                    )?;
                                    active_legacy_input = fresh.legacy_input;
                                    active_input = fresh.input;
                                    continue 'provider_attempt;
                                }
                                self.record_role_run_event(
                                    attempt,
                                    role_run,
                                    CodingRoleRunEventType::ProviderFailed,
                                    json!({
                                        "message": message.clone()
                                    }),
                                );
                                return self
                                    .fail_provider_stream_with_ownership(
                                        attempt,
                                        node_id,
                                        suppress_failure_side_effects,
                                        message,
                                    )
                                    .await;
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
                                return self
                                    .fail_provider_protocol_with_ownership(
                                        attempt,
                                        node_id,
                                        suppress_failure_side_effects,
                                        message,
                                    )
                                    .await;
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
                                    .fail_provider_stream_with_ownership(
                                        attempt,
                                        node_id,
                                        suppress_failure_side_effects,
                                        message,
                                    )
                                    .await;
                            }
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_legacy_stream_to_completion(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        role_run: Option<&CodingRoleRun>,
        provider: &dyn StreamingProviderAdapter,
        input: &AdapterInput,
        provider_name: &ProviderName,
        provider_role: CodingProviderRole,
        suppress_failure_side_effects: bool,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let cancel = self.cancellation.child_token();
        let mut stream = tokio::select! {
            biased;
            result = provider.run_streaming(input, cancel.clone()) => result?,
            _ = self.cancellation.cancelled() => {
                cancel.cancel();
                self.persist_provider_cancellation(attempt, role_run, "legacy_provider_start")?;
                return Err(CodingWorkspaceEngineError::Aborted);
            }
        };
        if let Err(error) = self.record_provider_start_required(
            attempt,
            role_run,
            json!({
                "provider": provider_name,
                "role": format!("{provider_role:?}"),
                "mode": "legacy_stream"
            }),
        ) {
            let message = error.to_string();
            cancel.cancel();
            drop(stream);
            return self
                .fail_provider_stream_with_ownership(
                    attempt,
                    node_id,
                    suppress_failure_side_effects,
                    message,
                )
                .await;
        }
        let mut full_output = String::new();
        loop {
            let chunk = tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => {
                    cancel.cancel();
                    self.persist_provider_cancellation(attempt, role_run, "legacy_provider_stream")?;
                    return Err(CodingWorkspaceEngineError::Aborted);
                }
                chunk = stream.recv() => chunk,
            };
            let Some(chunk) = chunk else {
                break;
            };
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
                    return self
                        .fail_provider_stream_with_ownership(
                            attempt,
                            node_id,
                            suppress_failure_side_effects,
                            message,
                        )
                        .await;
                }
            }
        }

        self.fail_provider_stream_ended_with_ownership(
            attempt,
            node_id,
            suppress_failure_side_effects,
        )
        .await
    }

    async fn fail_provider_protocol_with_ownership<T>(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        suppress_failure_side_effects: bool,
        message: String,
    ) -> Result<T, CodingWorkspaceEngineError> {
        if suppress_failure_side_effects {
            Err(CodingWorkspaceEngineError::ProviderProtocol(message))
        } else {
            self.fail_provider_stream(attempt, node_id, message).await
        }
    }

    async fn fail_provider_stream_with_ownership<T>(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        suppress_failure_side_effects: bool,
        message: String,
    ) -> Result<T, CodingWorkspaceEngineError> {
        if suppress_failure_side_effects {
            Err(CodingWorkspaceEngineError::ProviderStream(message))
        } else {
            self.fail_provider_stream(attempt, node_id, message).await
        }
    }

    async fn fail_provider_stream_ended_with_ownership<T>(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        suppress_failure_side_effects: bool,
    ) -> Result<T, CodingWorkspaceEngineError> {
        self.fail_provider_stream_with_ownership(
            attempt,
            node_id,
            suppress_failure_side_effects,
            "provider stream ended before completion".to_string(),
        )
        .await
    }
}
