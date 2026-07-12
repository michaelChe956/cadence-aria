use super::*;
use crate::product::workspace_engine::types::ReviewProviderRunFailure;

impl WorkspaceEngine {
    pub async fn drive_review_session(
        &mut self,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
        let reviewer = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let input = match self.build_review_input() {
            Ok(input) => input,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
        if let Some(node_id) = self.active_node_id.clone() {
            let _ = self
                .persist_prompt_snapshot(&node_id, input.prompt.clone())
                .await;
            self.emit_execution_event(
                provider_prompt_event(
                    &node_id,
                    input.prompt.clone(),
                    "发送给 Workspace provider 的完整提示词",
                ),
                Some(node_id),
                Some(reviewer.clone()),
            )
            .await;
        }
        let mut command_rx = command_rx;
        let first_session = provider.start(input.clone(), self.cancel.clone()).await;
        let first_completion = match self
            .drive_reviewer_provider_session_once(first_session, &mut command_rx, &reviewer)
            .await
        {
            ReviewProviderRunResult::Completed(completion) => completion,
            ReviewProviderRunResult::Aborted => return,
            ReviewProviderRunResult::Failed(failure) => {
                self.finish_review_provider_run_failure(failure).await;
                return;
            }
        };

        match self.parse_review_completion_for_active_node(&first_completion) {
            Ok(verdict) => self.complete_review(first_completion, verdict).await,
            Err(first_error) if first_error.is_repairable() => {
                let repair_input = match self.build_review_repair_input(
                    &input,
                    &first_completion,
                    &first_error,
                    first_completion.provider_session_id.clone(),
                ) {
                    Ok(input) => input,
                    Err(_) => {
                        let verdict =
                            fallback_review_verdict(&first_completion, &first_error, false);
                        self.complete_review(first_completion, verdict).await;
                        return;
                    }
                };
                let repair_node_id = self.active_node_id.clone();
                self.emit_execution_event(
                    structured_output_repair_event(
                        ProviderExecutionEventStatus::Started,
                        first_error.code(),
                    ),
                    repair_node_id.clone(),
                    Some(reviewer.clone()),
                )
                .await;
                let repair_session = provider.start(repair_input, self.cancel.clone()).await;
                let repair_result = self
                    .drive_reviewer_provider_session_once(
                        repair_session,
                        &mut command_rx,
                        &reviewer,
                    )
                    .await;
                let repaired_completion = match repair_result {
                    ReviewProviderRunResult::Completed(completion) => completion,
                    ReviewProviderRunResult::Aborted => {
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Failed,
                                first_error.code(),
                            ),
                            repair_node_id,
                            Some(reviewer.clone()),
                        )
                        .await;
                        return;
                    }
                    ReviewProviderRunResult::Failed(_) => {
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Failed,
                                first_error.code(),
                            ),
                            repair_node_id,
                            Some(reviewer.clone()),
                        )
                        .await;
                        let verdict =
                            fallback_review_verdict(&first_completion, &first_error, true);
                        self.complete_review(first_completion, verdict).await;
                        return;
                    }
                };

                match self.parse_review_completion_for_active_node(&repaired_completion) {
                    Ok(mut verdict)
                        if repair_payload_is_compatible(&first_error, &repaired_completion) =>
                    {
                        verdict.structured_output_diagnostic =
                            Some(success_diagnostic(&first_error));
                        let normalized = ProviderCompletion {
                            full_output: format!(
                                "{}\n{}",
                                first_completion.full_output, repaired_completion.full_output
                            ),
                            readable_output: first_completion.readable_output,
                            structured_output: repaired_completion.structured_output,
                            provider_session_id: repaired_completion.provider_session_id,
                        };
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Completed,
                                first_error.code(),
                            ),
                            repair_node_id.clone(),
                            Some(reviewer.clone()),
                        )
                        .await;
                        self.complete_review(normalized, verdict).await;
                    }
                    Ok(_) => {
                        let error = ReviewCompletionError::RepairPayloadChanged;
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Failed,
                                error.code(),
                            ),
                            repair_node_id.clone(),
                            Some(reviewer.clone()),
                        )
                        .await;
                        let verdict = fallback_review_verdict(&first_completion, &error, true);
                        self.complete_review(first_completion, verdict).await;
                    }
                    Err(second_error) => {
                        let normalized = ProviderCompletion {
                            full_output: format!(
                                "{}\n{}",
                                first_completion.full_output, repaired_completion.full_output
                            ),
                            readable_output: first_completion.readable_output,
                            structured_output: repaired_completion.structured_output,
                            provider_session_id: repaired_completion.provider_session_id,
                        };
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Failed,
                                second_error.code(),
                            ),
                            repair_node_id,
                            Some(reviewer.clone()),
                        )
                        .await;
                        let verdict = fallback_review_verdict(&normalized, &second_error, true);
                        self.complete_review(normalized, verdict).await;
                    }
                }
            }
            Err(error) => {
                let verdict = fallback_review_verdict(&first_completion, &error, false);
                self.complete_review(first_completion, verdict).await;
            }
        }
    }

    pub async fn drive_revision_session(
        &mut self,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
        let author = self.session.author_provider.clone();
        let node_id = self.active_node_id.clone();
        let input = match self.build_revision_input() {
            Ok(input) => input,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
        if let Some(node_id) = node_id.clone() {
            let _ = self
                .persist_prompt_snapshot(&node_id, input.prompt.clone())
                .await;
            self.emit_execution_event(
                provider_prompt_event(
                    &node_id,
                    input.prompt.clone(),
                    "发送给 Workspace provider 的完整提示词",
                ),
                Some(node_id),
                Some(author.clone()),
            )
            .await;
        }
        let retry_context = ArtifactRetryContext {
            provider: provider.clone(),
            input: input.clone(),
            attempted: false,
        };
        let revision_resume_fallback = if input.resume_provider_session_id.is_some()
            && self.session.author_provider == ProviderName::Codex
        {
            Some(RevisionResumeFallbackContext {
                provider: provider.clone(),
                attempted: false,
            })
        } else {
            None
        };
        let session = provider.start(input, self.cancel.clone()).await;
        self.drive_provider_session(ProviderSessionDriveInput {
            session,
            command_rx,
            node_id,
            agent: Some(author),
            role: ProviderConversationRole::Author,
            artifact_retry: Some(retry_context),
            revision_resume_fallback,
        })
        .await;
    }

    #[cfg(test)]
    pub(crate) async fn drive_reviewer_provider_session(
        &mut self,
        session: Result<
            ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        >,
        mut command_rx: mpsc::Receiver<ProviderCommand>,
        reviewer: ProviderName,
    ) {
        match self
            .drive_reviewer_provider_session_once(session, &mut command_rx, &reviewer)
            .await
        {
            ReviewProviderRunResult::Completed(completion) => {
                let verdict = match self.parse_review_completion_for_active_node(&completion) {
                    Ok(verdict) => verdict,
                    Err(error) => fallback_review_verdict(&completion, &error, false),
                };
                self.complete_review(completion, verdict).await;
            }
            ReviewProviderRunResult::Aborted => {}
            ReviewProviderRunResult::Failed(failure) => {
                self.finish_review_provider_run_failure(failure).await;
            }
        }
    }

    async fn finish_review_provider_run_failure(&mut self, failure: ReviewProviderRunFailure) {
        match failure {
            ReviewProviderRunFailure::Start(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                if let Some(node_id) = self.active_node_id.clone() {
                    self.update_timeline_node(
                        &node_id,
                        TimelineNodeStatus::Failed,
                        Some("Provider 启动失败".to_string()),
                    )
                    .await;
                }
                self.finish_failed_run().await;
            }
            ReviewProviderRunFailure::EmptyOutput => {
                self.finish_empty_assistant_output().await;
            }
            ReviewProviderRunFailure::Provider(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                if let Some(node_id) = self.active_node_id.clone() {
                    self.update_timeline_node(
                        &node_id,
                        TimelineNodeStatus::Failed,
                        Some("Provider 运行失败".to_string()),
                    )
                    .await;
                }
                self.finish_failed_run().await;
            }
            ReviewProviderRunFailure::PermissionTimeout(permission_id) => {
                self.handle_permission_timeout(permission_id, self.active_node_id.clone())
                    .await;
            }
        }
    }

    pub(crate) async fn drive_reviewer_provider_session_once(
        &mut self,
        session: Result<
            ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        >,
        command_rx: &mut mpsc::Receiver<ProviderCommand>,
        reviewer: &ProviderName,
    ) -> ReviewProviderRunResult {
        let mut session = match session {
            Ok(session) => session,
            Err(error) => {
                return ReviewProviderRunResult::Failed(ReviewProviderRunFailure::Start(
                    error.details,
                ));
            }
        };

        let node_id = self.active_node_id.clone();
        let mut full_content = String::new();
        let cancel = self.cancel.clone();
        let mut events_open = true;
        let mut commands_open = true;
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();

        while events_open {
            tokio::select! {
                _ = cancel.cancelled() => {
                    if let Some(node_id) = node_id.as_deref() {
                        let _ = self.flush_stream_buffer(node_id).await;
                    }
                    self.finish_aborted_run().await;
                    return ReviewProviderRunResult::Aborted;
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
                            return ReviewProviderRunResult::Aborted;
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
                            if session
                                .commands
                                .send(ProviderCommand::PermissionResponse {
                                    id,
                                    approved,
                                    reason,
                                })
                                .await
                                .is_err()
                            {
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
                            eprintln!(
                                "[aria-choice-diag] engine forwarding reviewer choice_response id={} selected={:?} free_text_present={}",
                                choice_id,
                                selected_option_ids,
                                free_text.as_ref().is_some_and(|text| !text.trim().is_empty())
                            );
                            if session
                                .commands
                                .send(ProviderCommand::ChoiceResponse {
                                    id,
                                    selected_option_ids,
                                    free_text,
                                    answers,
                                })
                                .await
                                .is_err()
                            {
                                eprintln!(
                                    "[aria-choice-diag] engine failed to forward reviewer choice_response id={} to provider session",
                                    choice_id
                                );
                                commands_open = false;
                            } else {
                                eprintln!(
                                    "[aria-choice-diag] engine forwarded reviewer choice_response id={} to provider session",
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
                                    role: "reviewer".to_string(),
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
                                    agent: Some(reviewer.clone()),
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
                            self
                                .emit_execution_event(
                                    event,
                                    node_id.clone(),
                                    Some(reviewer.clone()),
                                )
                                .await;
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
                                    Some(reviewer.clone()),
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
                                    Some(reviewer.clone()),
                                )
                                .await;
                        }
                        ProviderEvent::Completed(completion) => {
                            let provider_session_id = completion.provider_session_id.clone();
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            self.record_provider_session(
                                ProviderConversationRole::Reviewer,
                                reviewer.clone(),
                                provider_session_id,
                                node_id.clone(),
                            )
                            .await;
                            if completion.full_output.is_empty() {
                                return ReviewProviderRunResult::Failed(
                                    ReviewProviderRunFailure::EmptyOutput,
                                );
                            }
                            return ReviewProviderRunResult::Completed(completion);
                        }
                        ProviderEvent::Failed { message } => {
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            return ReviewProviderRunResult::Failed(
                                ReviewProviderRunFailure::Provider(message),
                            );
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
                            if let Some(node_id) = node_id.as_deref() {
                                let _ = self.flush_stream_buffer(node_id).await;
                            }
                            return ReviewProviderRunResult::Failed(
                                ReviewProviderRunFailure::PermissionTimeout(permission_id),
                            );
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
            ReviewProviderRunResult::Aborted
        } else if full_content.is_empty() {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            ReviewProviderRunResult::Failed(ReviewProviderRunFailure::EmptyOutput)
        } else {
            if let Some(node_id) = node_id.as_deref() {
                let _ = self.flush_stream_buffer(node_id).await;
            }
            let completion = ProviderCompletion::plain(full_content, None);
            ReviewProviderRunResult::Completed(completion)
        }
    }
}
