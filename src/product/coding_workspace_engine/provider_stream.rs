use super::cross_target_check::capture_cross_target_baseline;
use super::*;
use crate::cross_cutting::structured_output::StructuredOutputState;
use std::sync::{Arc, Mutex};

mod persistence;

mod cancellation;
mod launch;
mod legacy_stream;
#[cfg(test)]
mod outcome_tests;

use cancellation::warn_cancellation_site;
use launch::launch_provider_session;

/// P0 1.3：coding gate 只在回执 Delivered 后 resolve；等待 provider 等待者
/// 接收的上限（超时不 resolve、不假 Delivered，由 run 生命周期收口）。
const CODING_CHOICE_RECEIPT_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// 等待回执进入终态（Delivered/Rejected/Expired）；watch 关闭（信号被全部
/// 丢弃）同样返回当前状态，由调用方按非 Delivered 处理。
async fn wait_for_choice_receipt_terminal(
    status: &mut tokio::sync::watch::Receiver<
        crate::cross_cutting::choice_delivery::ChoiceReplyState,
    >,
) -> crate::cross_cutting::choice_delivery::ChoiceReplyState {
    loop {
        let current = *status.borrow();
        if !matches!(
            current,
            crate::cross_cutting::choice_delivery::ChoiceReplyState::Submitting
                | crate::cross_cutting::choice_delivery::ChoiceReplyState::Resolving
        ) {
            return current;
        }
        if status.changed().await.is_err() {
            return *status.borrow();
        }
    }
}

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

    /// 策略会话审计接线（Task 3.2 → Task 7 统一）：为 input 绑定 run-bound
    /// durable sink 并分配新的 `role_run_seq`；gateway validated input 同步重建。
    /// 每次 provider run 重新分配（重试 run 独立审计文件）。Task 7 起不因
    /// `tool_policy=None` 早退——LC run（Coder/Kimi 无通用策略）同样分配
    /// run-bound sink（prepared launch 已绑定 sink 时不重复分配）；非 LC
    /// legacy 直连保持零变化（不分配）。所有分配错误直接返回，不删 policy
    /// 再尝试。
    fn attach_tool_policy_audit(
        &self,
        attempt: &CodingExecutionAttempt,
        mut input: StreamingProviderInput,
        mut validated: Option<
            crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        >,
    ) -> Result<
        (
            StreamingProviderInput,
            Option<crate::cross_cutting::session_launch::ValidatedStreamingProviderInput>,
        ),
        String,
    > {
        if input.tool_policy.is_none() {
            let is_lc_run = attempt.target_snapshot.is_some();
            if !is_lc_run || input.audit_sink.is_some() {
                // 非 LC legacy 直连零变化;LC prepared launch 的 sink 已在
                // gateway prepare 阶段绑定,不重复分配。
                return Ok((input, validated));
            }
        }
        let workspace_session_id = input
            .workspace_session_id
            .clone()
            .unwrap_or_else(|| format!("coding-{}", attempt.id));
        let store = crate::product::lifecycle_store::LifecycleStore::new(self.store.paths());
        let role_run_seq = store
            .next_tool_policy_role_run_seq(&workspace_session_id)
            .map_err(|error| error.to_string())?;
        let sink = crate::cross_cutting::tool_policy_audit::RoleRunBoundAuditSink::new(
            std::sync::Arc::new(store),
            workspace_session_id,
            role_run_seq,
        )
        .into_sink();
        input.audit_sink = Some(sink.clone());
        if let Some(validated_input) = validated.take() {
            let (mut inner, launch) = validated_input.into_parts();
            if inner.audit_sink.is_none() {
                inner.audit_sink = Some(sink.clone());
            }
            validated = Some(
                crate::cross_cutting::session_launch::ValidatedStreamingProviderInput::new(
                    inner, launch,
                ),
            );
        }
        Ok((input, validated))
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
        self.admit_provider_run(attempt, &attempt.stage, "provider_stream")?;
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
        // Task 3.2（REQ-ENV-09/D7）：策略会话由 engine 构造 durable 审计 sink
        // （LifecycleStore `tool-policy-run-audit/` 分区）并按 provider run 分配
        // `role_run_seq`（分配随该 run 的 provider_start 首行落盘持久化）；三
        // adapter 在 start 返回前完成有界握手并写 provider_start，append 失败沿
        // 既有 kill 链终止会话、run 判失败。非策略路径零变化。
        let (active_input, validated_input) =
            match self.attach_tool_policy_audit(attempt, active_input, validated_input) {
                Ok(attached) => attached,
                Err(error) => {
                    return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                        "tool_policy_audit_sink_attach_failed: {error}"
                    )));
                }
            };

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
                        warn_cancellation_site(
                            attempt,
                            role_run,
                            timeout_reason_code.unwrap_or("provider_stream_timeout"),
                            "provider_start",
                        );
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
                        warn_cancellation_site(
                            attempt,
                            role_run,
                            "engine_cancellation",
                            "provider_start",
                        );
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
                        warn_cancellation_site(
                            attempt,
                            role_run,
                            "engine_cancellation",
                            "provider_start",
                        );
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
                        warn_cancellation_site(
                            attempt,
                            role_run,
                            "provider_start_persistence_failure",
                            "provider_start",
                        );
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
                        warn_cancellation_site(
                            attempt,
                            role_run,
                            "engine_cancellation",
                            "provider_stream",
                        );
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
                        warn_cancellation_site(
                            attempt,
                            role_run,
                            reason_code,
                            "provider_stream",
                        );
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
                                warn_cancellation_site(
                                    attempt,
                                    role_run,
                                    "runner_abort_attempt_command",
                                    "provider_stream",
                                );
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
                                answers,
                                receipt,
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
                                    if let Some(receipt) = receipt.as_ref() {
                                        receipt.reject();
                                    }
                                    continue;
                                }
                                // P0 1.3（REQ-WIGA-05）：完整 answers + 回执逐字段
                                // 透传 provider；仅当回执 Delivered（provider 等待者
                                // 真正接收）才落盘 resolve_choice_gate。无回执的旧
                                // WS 单题路径保持原立即 resolve 行为。
                                let mut receipt_status =
                                    receipt.as_ref().map(|signal| signal.subscribe());
                                if send_provider_command_with_cancellation(
                                    &session.commands,
                                    ProviderCommand::ChoiceResponse {
                                        id: id.clone(),
                                        selected_option_ids: selected_option_ids.clone(),
                                        free_text: free_text.clone(),
                                        answers: answers.clone(),
                                        receipt: receipt.clone(),
                                    },
                                    &self.cancellation,
                                )
                                .await
                                {
                                    let delivered = match receipt_status.as_mut() {
                                        None => true,
                                        Some(status) => match tokio::time::timeout(
                                            CODING_CHOICE_RECEIPT_WAIT,
                                            wait_for_choice_receipt_terminal(status),
                                        )
                                        .await
                                        {
                                            Ok(
                                                crate::cross_cutting::choice_delivery::ChoiceReplyState::Delivered,
                                            ) => true,
                                            Ok(_) | Err(_) => false,
                                        },
                                    };
                                    if !delivered {
                                        let _ = self
                                            .event_tx
                                            .send(CodingWsOutMessage::CodingProtocolError {
                                                code: "coding_choice_not_delivered".to_string(),
                                                message: format!(
                                                    "ChoiceResponse id={id} was not delivered to the provider waiter"
                                                ),
                                            })
                                            .await;
                                        continue;
                                    }
                                    let ack_selected_option_ids = selected_option_ids.clone();
                                    let ack_free_text = free_text.clone();
                                    let _ = self.store.resolve_choice_gate(
                                        &attempt.project_id,
                                        &attempt.issue_id,
                                        &attempt.id,
                                        &id,
                                        ResolveChoiceGateInput {
                                            selected_option_ids,
                                            free_text,
                                            answers,
                                        },
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
                                    if let Some(receipt) = receipt.as_ref() {
                                        receipt.reject();
                                    }
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
                            // token 用量（best-effort）：与 workspace_engine 主循环
                            // 同构，映射为 kind=usage 的 execution event
                            // （event_id=usage_{role}，同 role upsert 覆盖最新快照），
                            // 经 CodingExecutionEvent 送达 WS 并落 role-run 审计。
                            ProviderEvent::UsageReport(report) => {
                                let event = ws_execution_event_from_usage_report(report);
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
                            // 策略审计出口：观测性事件，主循环不消费。
                            ProviderEvent::ToolPolicyDecision(_)
                            | ProviderEvent::ToolPolicyWarning(_)
                            | ProviderEvent::ToolPolicyTerminated(_) => {}
                        }
                    }
                }
            }
        }
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

fn append_partial_output(observer: Option<&Arc<Mutex<String>>>, content: &str) {
    if let Some(observer) = observer
        && let Ok(mut output) = observer.lock()
    {
        output.push_str(content);
    }
}

/// Task 7(lcg_t07):LC validated 分流收口与 run-bound sink 统一分配的
/// engine 侧观测。计数 probe adapter 镜像真实 LC adapter 的 fail-closed
/// 契约(`start_validated` 缺 run-bound sink 即拒),使 sink 缺失与裸 start
/// 回退都成为可观测红点。
#[cfg(test)]
mod lcg_t07_validated_dispatch_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
    use crate::cross_cutting::provider_availability_gate::{
        ProviderAvailabilityGate, ProviderHealthSource,
    };
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::cross_cutting::session_launch::{
        ValidatedAdapterInput, ValidatedStreamingProviderInput,
    };
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
    use crate::product::logical_codebase::store::LogicalCodebaseManifest;
    use crate::product::logical_codebase::{
        AggregatePolicyArtifactStore, GatewayRunAudit, LogicalCodebaseProviderGateway,
        PolicyTarget, PolicyTargetResolver, ProviderCapability, ProviderCapabilitySource,
        ProviderGatewayError, ProviderRef, ProviderRefType, SessionLaunchRequest,
        SessionPolicyAction,
    };
    use crate::product::models::ProviderName;
    use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    /// 计数 probe streaming adapter:raw `start` 计数并成功返回;`start_validated`
    /// 镜像真实 LC adapter 契约——缺 run-bound sink 即 fail-closed,成功启动
    /// 才计入 validated 计数。
    struct T07DispatchProbeAdapter {
        raw_starts: AtomicUsize,
        validated_starts: AtomicUsize,
    }

    impl T07DispatchProbeAdapter {
        fn new() -> Self {
            Self {
                raw_starts: AtomicUsize::new(0),
                validated_starts: AtomicUsize::new(0),
            }
        }

        fn raw_start_count(&self) -> usize {
            self.raw_starts.load(Ordering::SeqCst)
        }

        fn validated_start_count(&self) -> usize {
            self.validated_starts.load(Ordering::SeqCst)
        }
    }

    fn probe_session() -> crate::cross_cutting::streaming_provider::ProviderSession {
        use crate::cross_cutting::streaming_provider::{
            ProviderCompletion, ProviderEvent, ProviderSession,
        };
        let (event_tx, events) = tokio::sync::mpsc::channel(4);
        let (commands, _command_rx) = tokio::sync::mpsc::channel(4);
        let _ = event_tx.try_send(ProviderEvent::Completed(ProviderCompletion::plain(
            "lcg t07 probe done",
            None,
        )));
        ProviderSession {
            native_session_id: None,
            events,
            commands,
        }
    }

    #[async_trait::async_trait]
    impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
        for T07DispatchProbeAdapter
    {
        async fn start(
            &self,
            _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<crate::cross_cutting::streaming_provider::ProviderSession, ProviderAdapterError>
        {
            self.raw_starts.fetch_add(1, Ordering::SeqCst);
            Ok(probe_session())
        }

        async fn start_validated(
            &self,
            launch: ValidatedStreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<crate::cross_cutting::streaming_provider::ProviderSession, ProviderAdapterError>
        {
            let (input, launch_policy) = launch.into_parts();
            if input.audit_sink.is_none() {
                return Err(ProviderAdapterError::parse_error(
                    "lcg t07 probe: audit sink is required for LC launches",
                    String::new(),
                    String::new(),
                ));
            }
            let _ = launch_policy;
            self.validated_starts.fetch_add(1, Ordering::SeqCst);
            Ok(probe_session())
        }
    }

    /// 计数 probe sync adapter:raw `run` 与 `run_validated` 分别计数。
    struct T07DispatchProbeSyncAdapter {
        raw_runs: AtomicUsize,
        validated_runs: AtomicUsize,
    }

    impl T07DispatchProbeSyncAdapter {
        fn new() -> Self {
            Self {
                raw_runs: AtomicUsize::new(0),
                validated_runs: AtomicUsize::new(0),
            }
        }

        fn raw_run_count(&self) -> usize {
            self.raw_runs.load(Ordering::SeqCst)
        }

        fn validated_run_count(&self) -> usize {
            self.validated_runs.load(Ordering::SeqCst)
        }
    }

    fn probe_output() -> AdapterOutput {
        AdapterOutput {
            exit_code: Some(0),
            stdout: "lcg t07 probe ok".to_string(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        }
    }

    impl ProviderAdapter for T07DispatchProbeSyncAdapter {
        fn run(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
        ) -> Result<AdapterOutput, ProviderAdapterError> {
            self.raw_runs.fetch_add(1, Ordering::SeqCst);
            Ok(probe_output())
        }

        fn run_validated(
            &self,
            _launch: ValidatedAdapterInput,
        ) -> Result<AdapterOutput, ProviderAdapterError> {
            self.validated_runs.fetch_add(1, Ordering::SeqCst);
            Ok(probe_output())
        }
    }

    struct T07StaticCapabilitySource;

    impl T07StaticCapabilitySource {
        fn capability() -> ProviderCapability {
            use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
            use crate::product::logical_codebase::policy::{ProviderDialect, ProviderWireDialect};
            use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
            ProviderCapability {
                provider_type: ProviderRefType::ClaudeCode,
                version: "1.0.0".to_string(),
                adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
                wire_dialect: ProviderWireDialect::ClaudeCodeStreamJson,
                capability_snapshot_ref: "cap-lcg-t07".to_string(),
                action_capability: ProviderActionCapability {
                    action: SessionPolicyAction::CodingTargetWrite,
                    launch: ProviderCapabilityEvidence::Confirmed,
                    resume: ProviderCapabilityEvidence::Confirmed,
                    write_boundary: ProviderCapabilityEvidence::Confirmed,
                    projection_digest: "t07-projection-digest".to_string(),
                    evidence_ref: "t07-evidence".to_string(),
                },
                trust: ProviderCapabilityEvidence::Confirmed,
            }
        }
    }

    impl ProviderCapabilitySource for T07StaticCapabilitySource {
        fn require_supported(
            &self,
            _provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Ok(Self::capability())
        }

        fn require_resume_supported(
            &self,
            provider: &ProviderRef,
            action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            self.require_supported(provider, action)
        }

        fn require_write_boundary(
            &self,
            provider: &ProviderRef,
            action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            self.require_supported(provider, action)
        }

        fn require_root_recipe_supported(
            &self,
            _provider: &ProviderRef,
            _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Err(ProviderGatewayError::UnsupportedCapability(
                "t07 fixture has no root recipe facts".to_string(),
            ))
        }
    }

    struct T07TargetResolver;

    impl PolicyTargetResolver for T07TargetResolver {
        fn resolve_and_revalidate(
            &self,
            request: &SessionLaunchRequest,
        ) -> Result<PolicyTarget, ProviderGatewayError> {
            Ok(request.target.clone())
        }
    }

    fn t07_available_gate() -> std::sync::Arc<ProviderAvailabilityGate> {
        struct AlwaysHealthy(std::sync::Arc<ProviderHealthSnapshot>);
        impl ProviderHealthSource for AlwaysHealthy {
            fn snapshot(&self) -> std::sync::Arc<ProviderHealthSnapshot> {
                self.0.clone()
            }
            fn degraded(&self) -> bool {
                false
            }
        }
        let checked_at = chrono::Utc::now();
        let snapshot = std::sync::Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at,
            providers: [ProviderName::ClaudeCode]
                .into_iter()
                .map(|provider| ProviderHealthEntry {
                    provider,
                    command: "stub".to_string(),
                    available: true,
                    version: Some("1.0.0".to_string()),
                    reason_code: None,
                    reason: None,
                    checked_at,
                })
                .collect(),
        });
        std::sync::Arc::new(ProviderAvailabilityGate::new(std::sync::Arc::new(
            AlwaysHealthy(snapshot),
        )))
    }

    fn t07_gateway(
        paths: &ProductAppPaths,
        registry: ProviderRegistry,
        sync_adapter: std::sync::Arc<T07DispatchProbeSyncAdapter>,
        authority_root: std::path::PathBuf,
    ) -> LogicalCodebaseProviderGateway {
        LogicalCodebaseProviderGateway::with_audit(
            AggregatePolicyArtifactStore::new(paths.clone()),
            std::sync::Arc::new(T07StaticCapabilitySource),
            std::sync::Arc::new(T07TargetResolver),
            std::sync::Arc::new(registry),
            sync_adapter,
            t07_available_gate(),
            std::sync::Arc::new(GatewayRunAudit::new()),
            authority_root,
        )
    }

    fn t07_ensure_bootstrap(paths: &ProductAppPaths) {
        let manifest =
            LogicalCodebaseManifest::new("project_0001", paths.root().to_path_buf(), vec![]);
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .expect("install lc bootstrap policy");
    }

    fn t07_logical_running_attempt(store: &CodingAttemptStore) -> CodingExecutionAttempt {
        let worktree = store.paths().root().join("member-worktree");
        std::fs::create_dir_all(&worktree).expect("member worktree");
        let created = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                worktree_path: Some(worktree.clone()),
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::ClaudeCode,
                    reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 2,
            })
            .expect("create attempt");
        let running = store
            .seed_running_attempt_for_test(&created.project_id, &created.issue_id, &created.id)
            .expect("seed running attempt");
        let mut logical = running.clone();
        logical.target_snapshot = Some(crate::product::coding_models::AttemptTargetSnapshot {
            logical_repository_id: crate::product::logical_codebase::LogicalRepositoryId(
                uuid::Uuid::new_v4(),
            ),
            checkout_id: crate::product::logical_codebase::RepositoryCheckoutId(
                uuid::Uuid::new_v4(),
            ),
            physical_repository_id: "repository_0001".to_string(),
            canonical_path: worktree,
            git_dir_identity: "git-dir-identity".to_string(),
            revision: None,
            policy_digest: String::new(),
            membership_revision: 1,
            captured_at: "2026-10-04T00:00:00Z".to_string(),
            capture_source: "lcg-t07".to_string(),
        });
        crate::product::json_store::write_json(
            &store
                .paths()
                .issue_root(&logical.project_id, &logical.issue_id)
                .join("coding-attempts")
                .join(format!("{}.json", logical.id)),
            &logical,
        )
        .expect("seed logical attempt record");
        logical
    }

    fn t07_engine(
        store: &CodingAttemptStore,
        gateway: Option<std::sync::Arc<LogicalCodebaseProviderGateway>>,
    ) -> CodingWorkspaceEngine {
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        let engine = CodingWorkspaceEngine::new(
            store.clone(),
            crate::product::git_workspace_service::GitWorkspaceService::new(),
            tx,
        );
        match gateway {
            Some(gateway) => engine.with_logical_provider_gateway(gateway),
            None => engine,
        }
    }

    fn t07_coder_input(
        working_dir: std::path::PathBuf,
        tool_policy: Option<crate::cross_cutting::streaming_provider::ProviderToolPolicy>,
    ) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
        crate::cross_cutting::streaming_provider::StreamingProviderInput {
            working_directory: None,
            baseline_tree: None,
            tool_policy,
            audit_sink: None,
            provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
            role: crate::protocol::contracts::AdapterRole::Executor,
            prompt: "lcg t07 validated dispatch probe".to_string(),
            working_dir,
            workspace_session_id: Some("ws-lcg-t07".to_string()),
            resume_provider_session_id: None,
            permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: std::collections::BTreeMap::new(),
            timeout_secs: 30,
        }
    }

    /// 同步栈 fixture input:与生产 LC sync 调用同形——独立 cwd = authority
    /// root(envelope 冻结的 canonical root),`worktree_path` 仍是 target。
    fn t07_legacy_input(
        authority_root: std::path::PathBuf,
        worktree: std::path::PathBuf,
    ) -> crate::protocol::contracts::AdapterInput {
        crate::protocol::contracts::AdapterInput {
            working_directory: Some(authority_root),
            prompt: "lcg t07 legacy probe".to_string(),
            provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
            role: crate::protocol::contracts::AdapterRole::Executor,
            timeout: 30,
            max_retries: 0,
            context_files: Vec::new(),
            output_schema: String::new(),
            provider_stream_log_dir: None,
            worktree_path: Some(worktree.to_string_lossy().to_string()),
        }
    }

    fn t07_coding_request(
        authority_root: std::path::PathBuf,
        worktree: std::path::PathBuf,
    ) -> SessionLaunchRequest {
        SessionLaunchRequest {
            project_id: "project_0001".to_string(),
            provider: ProviderRef::claude_code("cap-lcg-t07"),
            action: SessionPolicyAction::CodingTargetWrite,
            target: PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
            working_directory: authority_root.clone(),
            readable_roots: vec![authority_root],
            writable_roots: vec![worktree],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    async fn t07_run_provider_stream(
        engine: &CodingWorkspaceEngine,
        attempt: &CodingExecutionAttempt,
        provider: &dyn crate::cross_cutting::streaming_provider::StreamingProviderAdapter,
        input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        validated_input: Option<ValidatedStreamingProviderInput>,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let (command_tx, command_rx) = tokio::sync::mpsc::channel::<
            crate::product::coding_workspace_runner::CodingRunnerCommand,
        >(32);
        std::mem::forget(command_tx);
        let legacy_input = crate::protocol::contracts::AdapterInput {
            working_directory: None,
            prompt: "lcg t07 legacy probe".to_string(),
            provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
            role: crate::protocol::contracts::AdapterRole::Executor,
            timeout: 30,
            max_retries: 0,
            context_files: Vec::new(),
            output_schema: String::new(),
            provider_stream_log_dir: None,
            worktree_path: None,
        };
        let run = CodingProviderStreamRun {
            attempt,
            node_id: "lcg-t07-node",
            role_run: None,
            provider,
            legacy_input: &legacy_input,
            input,
            provider_name: &ProviderName::ClaudeCode,
            provider_role: CodingProviderRole::Coder,
            command_rx: &mut { command_rx },
            allow_legacy_stream_fallback: false,
            timeout: None,
            timeout_reason_code: None,
            suppress_failure_side_effects: true,
            validated_input,
        };
        engine.run_provider_stream_to_completion(run).await
    }

    /// Task 7 Step 1(断言组 377-380 逐字):LC 会话启动永远不触达裸
    /// `start`/`run`——engine 流式路径经 gateway 只调 `start_validated`
    /// (BASE:Coder 无通用策略时 `attach_tool_policy_audit` 早退、validated
    /// 启动缺 run-bound sink 失败,计数为 0,红);gateway 缺席时 LC
    /// validated 输入返回稳定错误、不回退裸 `start`(BASE:分派裸 start,
    /// raw 计数 1,红);同步栈 prepared launch 只经 `run_validated`,
    /// 非 prepared 存量构造同样不得落入裸 `run`(BASE:gateway 过渡分叉
    /// 走裸 run,raw 计数 1,红)。
    #[tokio::test]
    async fn lcg_t07_validated_launch_never_invokes_raw_start_or_run() {
        let root = tempfile::tempdir().expect("lcg t07 root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = t07_logical_running_attempt(&store);
        let authority_root = std::fs::canonicalize(root.path()).expect("canonical authority root");
        let worktree = attempt.worktree_path.clone().expect("worktree");
        t07_ensure_bootstrap(&paths);

        // 场景 A(流式,gateway 在场,prepared Coder launch):只允许
        // `start_validated` 成功一次;probe 镜像真实 adapter 的 sink 契约。
        let probe_stream = std::sync::Arc::new(T07DispatchProbeAdapter::new());
        let mut registry_stream = ProviderRegistry::new();
        registry_stream.register(ProviderName::ClaudeCode, probe_stream.clone());
        let gateway_stream = std::sync::Arc::new(t07_gateway(
            &paths,
            registry_stream,
            std::sync::Arc::new(T07DispatchProbeSyncAdapter::new()),
            authority_root.clone(),
        ));
        let engine_stream = t07_engine(&store, Some(gateway_stream.clone()));
        // 与生产 Coder 调用同源:input 显式携带 envelope 冻结的 canonical
        // cwd(authority root),spawn 前复验以 effective cwd 消费。
        let mut coder_input = t07_coder_input(worktree.clone(), None);
        coder_input.working_directory = Some(authority_root.clone());
        let prepared = engine_stream
            .prepare_streaming_launch_for_role(
                &attempt,
                CodingProviderRole::Coder,
                &worktree,
                coder_input.clone(),
            )
            .expect("prepare lc coder launch")
            .expect("logical attempt with gateway must prepare");
        if let Err(error) = t07_run_provider_stream(
            &engine_stream,
            &attempt,
            probe_stream.as_ref(),
            coder_input,
            Some(prepared),
        )
        .await
        {
            panic!("scenario A lc stream run failed: {error}");
        }

        // 场景 B(gateway 缺席):LC validated 输入不得回退裸 `start`。
        let probe_fallback = std::sync::Arc::new(T07DispatchProbeAdapter::new());
        let mut registry_probe = ProviderRegistry::new();
        registry_probe.register(ProviderName::ClaudeCode, probe_fallback.clone());
        let gateway_probe = t07_gateway(
            &paths,
            registry_probe,
            std::sync::Arc::new(T07DispatchProbeSyncAdapter::new()),
            authority_root.clone(),
        );
        let policy_for_fallback = gateway_probe
            .validate(t07_coding_request(authority_root.clone(), worktree.clone()))
            .expect("validate fallback probe policy");
        let engine_no_gateway = t07_engine(&store, None);
        let fallback_input = t07_coder_input(worktree.clone(), None);
        let fallback_validated =
            ValidatedStreamingProviderInput::new(fallback_input.clone(), policy_for_fallback);
        let fallback_result = t07_run_provider_stream(
            &engine_no_gateway,
            &attempt,
            probe_fallback.as_ref(),
            fallback_input,
            Some(fallback_validated),
        )
        .await;
        assert!(fallback_result.is_err(), "gateway 缺失必须稳定报错");
        assert!(
            fallback_result
                .expect_err("fallback result")
                .to_string()
                .contains("lc_validated_launch_requires_gateway"),
            "LC validated 输入缺 gateway 必须返回稳定错误码,不回退裸 start"
        );

        // 场景 C(同步,prepared):只经 `run_validated`,恰好一次。
        let probe_sync_prepared = std::sync::Arc::new(T07DispatchProbeSyncAdapter::new());
        let gateway_sync_prepared = t07_gateway(
            &paths,
            ProviderRegistry::new(),
            probe_sync_prepared.clone(),
            authority_root.clone(),
        );
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(paths.clone());
        let context =
            crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext {
                workspace_session_id: "ws-lcg-t07".to_string(),
                role_run_seq: lifecycle
                    .next_tool_policy_role_run_seq("ws-lcg-t07")
                    .expect("allocate role run seq"),
                audit_sink: std::sync::Arc::new(lifecycle),
            };
        let sync_input = t07_legacy_input(authority_root.clone(), worktree.clone());
        let prepared_sync = gateway_sync_prepared
            .prepare_sync_launch(
                sync_input.clone(),
                t07_coding_request(authority_root.clone(), worktree.clone()),
                context,
            )
            .expect("prepare sync launch");
        gateway_sync_prepared
            .run_sync(prepared_sync)
            .expect("prepared sync launch runs via run_validated");

        // 场景 D(同步,非 prepared 存量构造):不得落入裸 `run`。
        let probe_sync_legacy = std::sync::Arc::new(T07DispatchProbeSyncAdapter::new());
        let gateway_sync_legacy = t07_gateway(
            &paths,
            ProviderRegistry::new(),
            probe_sync_legacy.clone(),
            authority_root.clone(),
        );
        let legacy_policy = gateway_sync_legacy
            .validate(t07_coding_request(authority_root.clone(), worktree.clone()))
            .expect("validate legacy sync policy");
        let legacy_sync = ValidatedAdapterInput::new(sync_input, legacy_policy);
        let _ = gateway_sync_legacy.run_sync(legacy_sync);

        let raw_start_count_for_lc =
            probe_stream.raw_start_count() + probe_fallback.raw_start_count();
        let raw_run_count_for_lc =
            probe_sync_prepared.raw_run_count() + probe_sync_legacy.raw_run_count();
        let validated_start_count_for_lc = probe_stream.validated_start_count();
        let validated_run_count_for_lc = probe_sync_prepared.validated_run_count();
        assert_eq!(raw_start_count_for_lc, 0);
        assert_eq!(raw_run_count_for_lc, 0);
        assert_eq!(validated_start_count_for_lc, 1);
        assert_eq!(validated_run_count_for_lc, 1);
    }

    /// Task 7 Step 1(断言组 381 逐字):`attach_tool_policy_audit` 不因
    /// `tool_policy=None` 早退——LC run(Coder/Kimi 无通用策略)同样分配
    /// run-bound durable sink 与独立 `role_run_seq`;非 LC legacy 直连保持
    /// 零变化(不分配)。
    #[test]
    fn lcg_t07_lc_tool_policy_none_still_allocates_audit_sink() {
        let root = tempfile::tempdir().expect("lcg t07 audit root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = t07_logical_running_attempt(&store);
        let engine = t07_engine(&store, None);

        // LC run(tool_policy=None):必须分配 run-bound sink。
        let lc_input = t07_coder_input(attempt.worktree_path.clone().expect("worktree"), None);
        let (attached_input, attached_validated) = engine
            .attach_tool_policy_audit(&attempt, lc_input, None)
            .expect("attach audit for lc run");
        let audit_sink_allocated_when_tool_policy_is_none =
            attached_input.audit_sink.is_some() && attached_validated.is_none();

        // 非 LC legacy 直连(tool_policy=None):保持零变化,不分配。
        let mut legacy_attempt = attempt.clone();
        legacy_attempt.target_snapshot = None;
        let legacy_input = t07_coder_input(
            legacy_attempt.worktree_path.clone().expect("worktree"),
            None,
        );
        let (legacy_attached, _) = engine
            .attach_tool_policy_audit(&legacy_attempt, legacy_input, None)
            .expect("attach audit for legacy run");
        assert!(
            legacy_attached.audit_sink.is_none(),
            "非 LC legacy 直连不分配审计 sink(零变化)"
        );

        assert!(audit_sink_allocated_when_tool_policy_is_none);
    }
}
