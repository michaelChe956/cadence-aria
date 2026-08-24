use super::cross_target_check::capture_cross_target_baseline;
use super::*;
use crate::cross_cutting::structured_output::StructuredOutputState;
use std::sync::{Arc, Mutex};

mod persistence;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderStreamOutcome {
    pub(crate) full_output: String,
    pub(crate) structured_output: StructuredOutputState,
}

impl CodingWorkspaceEngine {
    /// 供外层重试协调器使用的单次调用入口；stream 层不在此路径创建失败门禁。
    #[allow(dead_code)]
    pub(crate) async fn run_provider_stream_invocation(
        &self,
        mut run: CodingProviderStreamRun<'_>,
    ) -> ProviderInvocationOutcome {
        run.suppress_failure_side_effects = true;
        let attempt = run.attempt;
        let role_run = run.role_run;
        let partial_output_observer = Arc::new(Mutex::new(String::new()));
        let result = self
            .run_structured_provider_stream_to_completion_with_partial_output(
                run,
                Some(partial_output_observer.clone()),
            )
            .await;
        let partial_output = partial_output_observer
            .lock()
            .map(|output| output.clone())
            .unwrap_or_default();
        let output_for_persistence = result
            .as_ref()
            .map(|outcome| outcome.full_output.as_str())
            .unwrap_or(partial_output.as_str());
        if let Some(role_run) = role_run
            && let Err(error) =
                self.persist_invocation_partial_output(attempt, role_run, output_for_persistence)
        {
            return ProviderInvocationOutcome::NonRetryable {
                reason_code: "provider_raw_output_persistence".to_string(),
                error,
                interaction_wait: false,
            };
        }
        ProviderInvocationOutcome::from_result(result, partial_output)
    }

    fn persist_invocation_partial_output(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: &CodingRoleRun,
        partial_output: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let raw_provider_output_ref = self.store.save_provider_raw_output(
            attempt,
            role_run.stage.clone(),
            "provider_stream_attempt",
            partial_output,
        )?;
        self.store.update_role_run_refs(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            vec![raw_provider_output_ref],
            Vec::new(),
        )?;
        Ok(())
    }

    pub(crate) async fn run_provider_stream_to_completion(
        &self,
        run: CodingProviderStreamRun<'_>,
    ) -> Result<String, CodingWorkspaceEngineError> {
        Ok(self
            .run_structured_provider_stream_to_completion(run)
            .await?
            .full_output)
    }

    pub(crate) async fn run_structured_provider_stream_to_completion(
        &self,
        run: CodingProviderStreamRun<'_>,
    ) -> Result<ProviderStreamOutcome, CodingWorkspaceEngineError> {
        self.run_structured_provider_stream_to_completion_with_partial_output(run, None)
            .await
    }

    async fn run_structured_provider_stream_to_completion_with_partial_output(
        &self,
        run: CodingProviderStreamRun<'_>,
        partial_output_observer: Option<Arc<Mutex<String>>>,
    ) -> Result<ProviderStreamOutcome, CodingWorkspaceEngineError> {
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
            suppress_failure_side_effects,
            validated_input,
        } = run;
        self.store.ensure_provider_run_allowed(attempt)?;
        // Task 12:逻辑代码库 target(`attempt.target_snapshot.is_some()`)的 provider
        // 必须经 `LogicalCodebaseProviderGateway`(表现为 `validated_input` 非空)。禁止:
        // 1. 在无 gateway(`validated_input` 为 `None`)时直接启动 provider(不论是否为
        //    Fake)——这是「裸 `StreamingProviderInput` 不得启动真实 provider」的
        //    fail-closed 门。
        // 2. 回落到 legacy `run_streaming` bridge——逻辑 target 的
        //    `allow_legacy_stream_fallback` 被强制为 `false`,使 `start` 未实现时不会
        //    调用 `provider.run_streaming`。
        let is_logical_target = attempt.target_snapshot.is_some();
        let allow_legacy_stream_fallback = if is_logical_target {
            false
        } else {
            allow_legacy_stream_fallback
        };
        if is_logical_target && validated_input.is_none() {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "logical_provider_gateway_required".to_string(),
            ));
        }
        // Task 14:逻辑代码库 target 的每个 provider role run 启动前采集跨仓越界
        // baseline（只监控各成员主 checkout 的 HEAD/status）。run 结束后由 Task 15
        // 统一门在交付前调用 detect 比对。Legacy 路径（`target_snapshot` 为 `None`）
        // 不采，保持现状。
        if is_logical_target && let Some(role_run) = role_run {
            capture_cross_target_baseline(&self.store.paths(), attempt, &role_run.id).map_err(
                |code| {
                    CodingWorkspaceEngineError::ProviderStream(format!(
                        "cross_target_baseline_capture_failed: {code}"
                    ))
                },
            )?;
        }
        let active_legacy_input = legacy_input.clone();
        let active_input = input;
        // Task 11:逻辑代码库真实 provider 启动经 gateway。仅当 `validated_input` 非空
        // 且引擎注入了 gateway 时,启动改为 `gateway.start_streaming`;传统/非逻辑
        // 路径保留直接 `provider.start`。`StreamingProviderInput` 从 `validated_input`
        // 中取出后复用(调用方保证它与 `input` 同源)。
        let gateway_ref = self.logical_provider_gateway.clone();
        {
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
                    result = launch_provider_session(
                        provider,
                        active_input.clone(),
                        cancel.clone(),
                        validated_input.clone(),
                        gateway_ref.as_ref(),
                    ) => result,
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
                    result = launch_provider_session(
                        provider,
                        active_input.clone(),
                        cancel.clone(),
                        validated_input.clone(),
                        gateway_ref.as_ref(),
                    ) => result,
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
                            partial_output_observer,
                        )
                        .await;
                }
                Err(error) if !allow_legacy_stream_fallback => {
                    let message = error.details.clone();
                    self.record_role_run_event(
                        attempt,
                        role_run,
                        CodingRoleRunEventType::ProviderFailed,
                        json!({
                            "phase": "provider_start",
                            "message": message.clone()
                        }),
                    );
                    return Err(if suppress_failure_side_effects {
                        CodingWorkspaceEngineError::ProviderAdapter(error)
                    } else {
                        CodingWorkspaceEngineError::ProviderStream(message)
                    });
                }
                Err(error) => {
                    let message = error.details.clone();
                    if suppress_failure_side_effects {
                        return Err(CodingWorkspaceEngineError::ProviderAdapter(error));
                    }
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
                        let waiting_for_choice = !open_choice_ids.is_empty();
                        let reason_code = if waiting_for_choice {
                            "choice_timeout"
                        } else {
                            timeout_reason_code.unwrap_or("provider_stream_timeout")
                        };
                        self.record_role_run_event(
                            attempt,
                            role_run,
                            CodingRoleRunEventType::Timeout,
                            json!({
                                "phase": "provider_stream",
                                "reason_code": reason_code,
                                "choice_ids": open_choice_ids
                            }),
                        );
                        return Err(CodingWorkspaceEngineError::ProviderStream(
                            reason_code.to_string(),
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
                                        self.store
                                            .admit_and_transition_attempt_to_executable(
                                                &attempt.project_id,
                                                &attempt.issue_id,
                                                &attempt.id,
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
                                    append_partial_output(
                                        partial_output_observer.as_ref(),
                                        &content,
                                    );
                                    return Err(self.unresolved_provider_choice_error(
                                        attempt,
                                        role_run,
                                        "provider_text_delta",
                                        &open_choice_ids,
                                    ));
                                }
                                let content_for_event = content.clone();
                                full_output.push_str(&content);
                                append_partial_output(
                                    partial_output_observer.as_ref(),
                                    &content_for_event,
                                );
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
                                let structured_output = completion.structured_output.clone();
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
                                return Ok(ProviderStreamOutcome {
                                    full_output,
                                    structured_output,
                                });
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
                            // token 用量采集当前仅覆盖 workspace_engine 主事件循环；coding
                            // workspace 链路暂不消费 usage（best-effort，缺失不报错）。
                            ProviderEvent::UsageReport(_) => {}
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
        partial_output_observer: Option<Arc<Mutex<String>>>,
    ) -> Result<ProviderStreamOutcome, CodingWorkspaceEngineError> {
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
                    append_partial_output(partial_output_observer.as_ref(), &content_for_event);
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
                    return Ok(ProviderStreamOutcome {
                        full_output: output,
                        structured_output: StructuredOutputState::NotRequested,
                    });
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

/// Task 11:按是否携带 validated input + 是否注入 gateway 选择 provider 启动路径。
///
/// - 两者均存在:经 `LogicalCodebaseProviderGateway::start_streaming` 启动,使政策
///   校验、canonical 复验、resume fail-closed 都在 gateway 内完成并留 audit。
/// - 否则(传统/非逻辑 issue):直接 `provider.start`,保留既有行为。
///
/// 返回 boxed future 以便外层 `tokio::select!` 统一内联。gateway 错误映射为
/// `ProviderAdapterError`,使其与直接 adapter 错误在 stream 层等价处理。
fn launch_provider_session<'a>(
    provider: &'a dyn StreamingProviderAdapter,
    input: StreamingProviderInput,
    cancel: CancellationToken,
    validated: Option<crate::cross_cutting::session_launch::ValidatedStreamingProviderInput>,
    gateway: Option<
        &std::sync::Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
    >,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<
                    crate::cross_cutting::streaming_provider::ProviderSession,
                    ProviderAdapterError,
                >,
            > + Send
            + 'a,
    >,
> {
    if let (Some(validated), Some(gateway)) = (validated, gateway) {
        let gateway = gateway.clone();
        Box::pin(async move {
            gateway
                .start_streaming(validated, cancel)
                .await
                .map_err(provider_adapter_error_from_gateway)
        })
    } else {
        // 非 gateway 路径(传统/非逻辑 issue):直接 adapter start。逻辑代码库 feature
        // 在入口处由 `validated_input`/`logical_provider_gateway` 的存在与否分流,
        // 使旧 API 行为不被本工作包扩大。
        Box::pin(async move { provider.start(input, cancel).await })
    }
}

/// 把 gateway 错误映射为 `ProviderAdapterError`,使 provider stream 层把「政策门/
/// 复验拒绝」与「adapter 运行失败」统一为 transport 错误。fail-closed 维度保留在
/// `details` 中供上游诊断。
fn provider_adapter_error_from_gateway(
    error: crate::product::logical_codebase::ProviderGatewayError,
) -> ProviderAdapterError {
    use crate::protocol::contracts::TimeoutStatus;
    use crate::protocol::provider_errors::ProviderErrorCode;
    ProviderAdapterError {
        code: ProviderErrorCode::ProviderUnavailable,
        details: error.to_string(),
        stdout: String::new(),
        stderr: String::new(),
        exit_code: None,
        timeout_status: TimeoutStatus::NotTimedOut,
        duration_ms: 0,
    }
}

fn append_partial_output(observer: Option<&Arc<Mutex<String>>>, content: &str) {
    if let Some(observer) = observer
        && let Ok(mut output) = observer.lock()
    {
        output.push_str(content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::structured_output::StructuredOutputState;
    use serde_json::json;

    #[test]
    fn provider_stream_outcome_keeps_completion_structured_output() {
        let outcome = ProviderStreamOutcome {
            full_output: "可读审查回执".to_string(),
            structured_output: StructuredOutputState::Parsed(json!({
                "verdict": "approve",
                "findings": []
            })),
        };

        assert_eq!(outcome.full_output, "可读审查回执");
        assert_eq!(
            outcome.structured_output,
            StructuredOutputState::Parsed(json!({
                "verdict": "approve",
                "findings": []
            }))
        );
    }
}
