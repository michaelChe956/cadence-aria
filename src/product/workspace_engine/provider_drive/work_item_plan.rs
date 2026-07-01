use super::*;

impl WorkspaceEngine {
    pub async fn drive_work_item_plan_provider_session_to_output(
        &mut self,
        session: Result<
            ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        >,
        command_rx: &mut mpsc::Receiver<ProviderCommand>,
        node_id: String,
        agent: ProviderName,
    ) -> Result<String, String> {
        let mut session = match session {
            Ok(session) => session,
            Err(error) => {
                let message = error.details.clone();
                let _ = self
                    .event_tx
                    .send(EngineEvent::Error {
                        message: message.clone(),
                    })
                    .await;
                self.update_timeline_node(
                    &node_id,
                    TimelineNodeStatus::Failed,
                    Some("Provider 启动失败".to_string()),
                )
                .await;
                self.finish_failed_run().await;
                return Err(message);
            }
        };

        let cancel = self.cancel.clone();
        let mut full_content = String::new();
        let mut events_open = true;
        let mut commands_open = true;
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();
        let mut display_filter = StructuredOutputDisplayFilter::new();

        while events_open {
            tokio::select! {
                _ = cancel.cancelled() => {
                    let display_content = display_filter.finish();
                    self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                    let _ = self.flush_stream_buffer(&node_id).await;
                    self.finish_aborted_run().await;
                    return Err("provider run aborted".to_string());
                }
                command = command_rx.recv(), if commands_open => {
                    match command {
                        Some(ProviderCommand::Abort) => {
                            let _ = session.commands.send(ProviderCommand::Abort).await;
                            cancel.cancel();
                            let display_content = display_filter.finish();
                            self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                            let _ = self.flush_stream_buffer(&node_id).await;
                            self.finish_aborted_run().await;
                            return Err("provider run aborted".to_string());
                        }
                        Some(ProviderCommand::PermissionResponse {
                            id,
                            approved,
                            reason,
                        }) => {
                            let _ = self
                                .persist_permission_response(
                                    &node_id,
                                    id.clone(),
                                    serde_json::json!({
                                        "approved": approved,
                                        "reason": reason.clone(),
                                    }),
                                )
                                .await;
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
                            if session.commands.send(ProviderCommand::ChoiceResponse {
                                id,
                                selected_option_ids,
                                free_text,
                                answers,
                            }).await.is_err() {
                                commands_open = false;
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
                            full_content.push_str(&content);
                            let display_content = display_filter.push(&content);
                            self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                        }
                        ProviderEvent::PermissionRequest(request) => {
                            let _ = self
                                .persist_permission_request(
                                    &node_id,
                                    request.id.clone(),
                                    serde_json::json!({
                                        "tool_name": request.tool_name.clone(),
                                        "description": request.description.clone(),
                                        "risk_level": risk_level_text(&request.risk_level),
                                    }),
                                )
                                .await;
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
                                    node_id: Some(node_id.clone()),
                                    agent: Some(agent.clone()),
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
                                    Some(node_id.clone()),
                                    Some(agent.clone()),
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
                                    Some(node_id.clone()),
                                    Some(agent.clone()),
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
                                    Some(node_id.clone()),
                                    Some(agent.clone()),
                                )
                                .await;
                        }
                        ProviderEvent::Completed {
                            full_output,
                            provider_session_id,
                        } => {
                            let display_content = display_filter.finish();
                            self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                            let _ = self.flush_stream_buffer(&node_id).await;
                            self
                                .record_provider_session(
                                    ProviderConversationRole::Author,
                                    agent,
                                    provider_session_id,
                                    Some(node_id),
                                )
                                .await;
                            return Ok(full_output);
                        }
                        ProviderEvent::Failed { message } => {
                            let display_content = display_filter.finish();
                            self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                            let _ = self.flush_stream_buffer(&node_id).await;
                            let _ = self
                                .event_tx
                                .send(EngineEvent::Error {
                                    message: message.clone(),
                                })
                                .await;
                            self.update_timeline_node(
                                &node_id,
                                TimelineNodeStatus::Failed,
                                Some("Provider 运行失败".to_string()),
                            )
                            .await;
                            self.finish_failed_run().await;
                            return Err(message);
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
                            self
                                .handle_permission_timeout(
                                    permission_id.clone(),
                                    Some(node_id.clone()),
                                )
                                .await;
                            return Err(format!("permission timeout: {permission_id}"));
                        }
                    }
                }
            }
        }

        let display_content = display_filter.finish();
        self.emit_work_item_plan_display_chunk(&node_id, display_content)
            .await;
        let _ = self.flush_stream_buffer(&node_id).await;
        if full_content.is_empty() {
            self.finish_empty_assistant_output().await;
            Err("provider completed without output".to_string())
        } else {
            Ok(full_content)
        }
    }

    pub(crate) async fn emit_work_item_plan_display_chunk(
        &mut self,
        node_id: &str,
        content: String,
    ) {
        if content.is_empty() {
            return;
        }
        let _ = self.buffer_stream_chunk(node_id, content.clone()).await;
        let _ = self
            .event_tx
            .send(EngineEvent::StreamChunk {
                role: "assistant".to_string(),
                content,
                node_id: Some(node_id.to_string()),
            })
            .await;
    }

    pub(crate) async fn emit_execution_event(
        &mut self,
        event: ProviderExecutionEvent,
        node_id: Option<String>,
        agent: Option<ProviderName>,
    ) {
        if let Some(node_id) = node_id.as_deref() {
            let event_json = execution_event_json(&event);
            let _ = self
                .update_node_detail(node_id, |detail| {
                    upsert_execution_event_json(&mut detail.execution_events, event_json);
                })
                .await;
        }
        let _ = self
            .event_tx
            .send(EngineEvent::ExecutionEvent {
                event,
                node_id,
                agent,
            })
            .await;
    }

    pub async fn emit_provider_prompt_event(
        &mut self,
        node_id: &str,
        prompt: String,
        detail: &'static str,
        agent: Option<ProviderName>,
    ) {
        let _ = self.persist_prompt_snapshot(node_id, prompt.clone()).await;
        self.emit_execution_event(
            provider_prompt_event(node_id, prompt, detail),
            Some(node_id.to_string()),
            agent,
        )
        .await;
    }
}
