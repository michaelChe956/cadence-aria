impl WorkspaceEngine {
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
            ReviewProviderRunFailure::Start { message, code } => {
                // GAP-H（Task 0.3）：固定脱敏摘要在 message 移动前计算——匹配
                // 只用 code 与标记串，原始 stderr/凭据绝不进 durable 摘要。
                let diagnostic = reviewer_failure_diagnostic(Some(code), &message);
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                if let Some(node_id) = self.active_node_id.clone() {
                    self.update_timeline_node(
                        &node_id,
                        TimelineNodeStatus::Failed,
                        Some(diagnostic),
                    )
                    .await;
                }
                self.promote_single_candidate_review_failed();
                self.finish_failed_run().await;
            }
            ReviewProviderRunFailure::EmptyOutput => {
                self.promote_single_candidate_review_failed();
                self.finish_empty_assistant_output().await;
            }
            ReviewProviderRunFailure::Provider { message, code } => {
                let diagnostic = reviewer_failure_diagnostic(code, &message);
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                if let Some(node_id) = self.active_node_id.clone() {
                    self.update_timeline_node(
                        &node_id,
                        TimelineNodeStatus::Failed,
                        Some(diagnostic),
                    )
                    .await;
                }
                self.promote_single_candidate_review_failed();
                self.finish_failed_run().await;
            }
            ReviewProviderRunFailure::PermissionTimeout(permission_id) => {
                self.handle_permission_timeout(permission_id, self.active_node_id.clone())
                    .await;
            }
        }
    }

    /// P2 GAP-H（Task 0.3）：reviewer 失败的固定脱敏诊断摘要。仅当原 adapter
    /// 错误码为 `ProviderUnavailable` 且有界 details/stderr 同时含 `503` 与
    /// `No available accounts` 时落固定安全类；其余（含无 code 的 runtime
    /// 失败、动态网络波动）一律通用失败类。原始 stderr/Authorization 不落
    /// durable 摘要；故障不是自动重试信号，重驱只走 Task 0.1 人工入口。
    /// P2 GAP-E/G（Task 0.1）：SC Evaluate 相位的 reviewer 运行失败收敛为
    /// durable 终态 Failed（phase+status CAS），供人工显式重驱；仅作用于
    /// 当前 reviewer 节点已标 Failed 的现场，不进入其他 finish_failed_run
    /// 共用分支。非 Evaluate 相位（Generate/Approval 等）保持旧语义。
    fn promote_single_candidate_review_failed(&mut self) {
        if self.session.flow_kind
            == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
            && self.session.single_candidate_phase
                == Some(crate::product::models::SingleCandidatePhase::Evaluate)
        {
            self.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
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
                // 诊断直通（claude×轻 握手谜团第 2 轮）：provider 启动失败的 stderr
                // 尾部（有界）并入消息——claude D③ 快照未并入 details 的时刻不再
                // 被驱动层丢弃，随 Start 失败 → EngineEvent::Error → WS error 上浮。
                let mut message = error.details;
                let code = error.code;
                ProviderAdapterError::append_bounded_stderr_tail(
                    &mut message,
                    &error.stderr,
                    PROVIDER_ERROR_STDERR_TAIL_BYTES,
                );
                // GAP-H（Task 0.3）：错误码随失败透传，供节点摘要按原 code
                // 分类，不从拼接后的文案猜。
                return ReviewProviderRunResult::Failed(ReviewProviderRunFailure::Start {
                    message,
                    code,
                });
            }
        };

        let node_id = self.active_node_id.clone();
        let mut full_content = String::new();
        let cancel = self.cancel.clone();
        let mut events_open = true;
        let mut commands_open = true;
        let mut tool_call_titles = BTreeMap::new();
        let mut tool_call_commands = BTreeMap::new();
        // F-19b：review 面补零活动看门狗（对照 author 面 provider_drive 先例；
        // 此前 review 驱动循环完全无超时臂，reviewer 静默即永久悬置）。
        // 权限/choice 悬置等待人工期间按同法挂起（权限由 adapter 侧
        // PERMISSION_TIMEOUT 收口；choice 由下方等待界收口）。
        let idle_watchdog_timeout = PROVIDER_IDLE_WATCHDOG_TIMEOUT;
        let idle_watchdog =
            tokio::time::sleep_until(tokio::time::Instant::now() + idle_watchdog_timeout);
        tokio::pin!(idle_watchdog);
        let mut waiting_for_permission = false;
        // F-22/F-19b：choice 悬置等待界（语义同 author 面先例）。
        let choice_wait_timeout = PROVIDER_CHOICE_WAIT_TIMEOUT;
        let choice_wait_timer =
            tokio::time::sleep_until(tokio::time::Instant::now() + choice_wait_timeout);
        tokio::pin!(choice_wait_timer);
        // F-27R3：挂起集经 PendingChoiceRequests 镜像到进程级登记簿（对照主驱动
        // provider_drive 先例；review run 独立 guard——每次 once 调用即一个
        // provider 会话 run，Drop 只摘本 run 登记的 id）。session_state 全量投影
        // 据此补挂 review 驱动挂起的 choice 卡。
        let mut pending_choice_ids = PendingChoiceRequests::new(&self.session.session_id);

        while events_open {
            tokio::select! {
                _ = cancel.cancelled() => {
                    // 诊断打点（claude×轻 握手谜团第 2 轮，不改行为）：reviewer 驱动
                    // 循环观察到 engine/run token 被外部取消（workitem reviewer 启动
                    // 窗口被杀的观察点）。
                    eprintln!(
                        "[aria-cancellation] workspace review_drive select_cancelled trigger=engine_cancelled_observed session_id={} role=reviewer agent={reviewer:?}",
                        self.session.session_id
                    );
                    if let Some(node_id) = node_id.as_deref() {
                        let _ = self.flush_stream_buffer(node_id).await;
                    }
                    self.finish_aborted_run().await;
                    return ReviewProviderRunResult::Aborted;
                }
                _ = &mut idle_watchdog,
                if !waiting_for_permission && pending_choice_ids.is_empty() =>
                {
                    // F-19b：reviewer 零活动楔死——Abort kill 链 + cancel，
                    // 经 Failed(Provider) 让 finish_review_provider_run_failure
                    // 统一收口（Error 事件 + 节点失败 + finish_failed_run）。
                    eprintln!(
                        "[aria-cancellation] workspace review_drive idle_watchdog trigger=provider_idle_watchdog session_id={} role=reviewer agent={reviewer:?} timeout_secs={}",
                        self.session.session_id,
                        idle_watchdog_timeout.as_secs()
                    );
                    let message = format!(
                        "provider_idle_watchdog: reviewer 会话 {} 秒零活动（无事件/命令），疑似楔死，运行已由看门狗中止；可重新开始生成",
                        idle_watchdog_timeout.as_secs()
                    );
                    let _ = session.commands.send(ProviderCommand::Abort).await;
                    cancel.cancel();
                    if let Some(node_id) = node_id.as_deref() {
                        let _ = self.flush_stream_buffer(node_id).await;
                    }
                    return ReviewProviderRunResult::Failed(ReviewProviderRunFailure::Provider {
                        message,
                        code: None,
                    });
                }
                _ = &mut choice_wait_timer,
                if !pending_choice_ids.is_empty() =>
                {
                    // F-22/F-19b：reviewer choice 卡丢失/无人应答超界。
                    let pending_ids: Vec<&str> = pending_choice_ids.ids();
                    eprintln!(
                        "[aria-cancellation] workspace review_drive choice_wait_timeout trigger=provider_choice_wait_timeout session_id={} role=reviewer agent={reviewer:?} pending={:?} timeout_secs={}",
                        self.session.session_id,
                        pending_ids,
                        choice_wait_timeout.as_secs()
                    );
                    let message = format!(
                        "provider_choice_wait_timeout: 等待回答超时——{} 秒内未收到选择应答，运行已中止；choice 卡可能未送达（页面断线/连接降级期间丢失，刷新页面可补卡）或已送达但无人应答（pending={:?}）。请重新提交反馈重新发起本轮",
                        choice_wait_timeout.as_secs(),
                        pending_ids
                    );
                    let _ = session.commands.send(ProviderCommand::Abort).await;
                    cancel.cancel();
                    if let Some(node_id) = node_id.as_deref() {
                        let _ = self.flush_stream_buffer(node_id).await;
                    }
                    return ReviewProviderRunResult::Failed(ReviewProviderRunFailure::Provider {
                        message,
                        code: None,
                    });
                }
                command = command_rx.recv(), if commands_open => {
                    // F-19b：任何命令活动（含人工权限/选择应答）重置看门狗。
                    idle_watchdog
                        .as_mut()
                        .reset(tokio::time::Instant::now() + idle_watchdog_timeout);
                    match command {
                        Some(ProviderCommand::Abort) => {
                            // 诊断打点：Abort 命令到达 reviewer 驱动循环。
                            eprintln!(
                                "[aria-cancellation] workspace review_drive abort_command trigger=abort_command session_id={} role=reviewer agent={reviewer:?}",
                                self.session.session_id
                            );
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
                            // F-19b：人工权限应答到达，解除看门狗挂起。
                            waiting_for_permission = false;
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
                            receipt,
                        }) => {
                            // F-22/F-19b：choice 应答到达——解除 pending 等待界。
                            // F-27R3：应答命中挂起 choice（登记簿同步摘除）——通知
                            // Web runtime 广播全量 session_state 收敛已答卡（对照主
                            // 驱动先例）。
                            if pending_choice_ids.remove(&id).is_some() {
                                let _ = self
                                    .event_tx
                                    .send(EngineEvent::ChoicePendingChanged)
                                    .await;
                            }
                            tracing::info!(choice_id = %id, "engine forwarding choice response");
                            if session
                                .commands
                                .send(ProviderCommand::ChoiceResponse {
                                    id,
                                    selected_option_ids,
                                    free_text,
                                    answers,
                                    receipt,
                                })
                                .await
                            .is_err()
                            {
                                commands_open = false;
                            }
                        }
                        Some(ProviderCommand::ToolResult(_)) => {}
                        None => commands_open = false,
                    }
                }
                event = session.events.recv() => {
                    // F-19b：任何 reviewer 事件重置零活动看门狗。
                    idle_watchdog
                        .as_mut()
                        .reset(tokio::time::Instant::now() + idle_watchdog_timeout);
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
                            // F-19b：权限请求悬置期间等待人工输入，看门狗挂起
                            //（权限等待由 adapter 侧 PERMISSION_TIMEOUT 收口）。
                            waiting_for_permission = true;
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
                            // F-22/F-19b：pending 由空转非空时起算/重置等待界。
                            let choice_wait_started = pending_choice_ids.is_empty();
                            // F-27R3：登记进程级挂起集（三驱动统一登记簿，全量
                            // ChoiceRequestData）——session_state 投影经登记簿携带本卡。
                            pending_choice_ids.insert(request.clone(), "reviewer");
                            if choice_wait_started {
                                choice_wait_timer
                                    .as_mut()
                                    .reset(tokio::time::Instant::now() + choice_wait_timeout);
                            }
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
                        // token 用量（best-effort）：reviewer 侧同样落盘为 kind=usage。
                        ProviderEvent::UsageReport(report) => {
                            self.emit_execution_event(
                                execution_event_from_usage_report(report),
                                node_id.clone(),
                                Some(reviewer.clone()),
                            )
                            .await;
                        }
                        // 策略审计出口：观测性事件，review 驱动不消费（Task 3.2 接 sink）。
                        ProviderEvent::ToolPolicyDecision(_)
                        | ProviderEvent::ToolPolicyWarning(_)
                        | ProviderEvent::ToolPolicyTerminated(_) => {}
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
                                ReviewProviderRunFailure::Provider { message, code: None },
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
                                let _ = self
                                    .persist_permission_timeout(node_id, permission_id.clone())
                                    .await;
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
            // 诊断打点：reviewer 事件流自然结束后的取消后置检查（防竞态分支）。
            eprintln!(
                "[aria-cancellation] workspace review_drive post_loop_cancelled trigger=engine_cancelled_observed session_id={} role=reviewer agent={reviewer:?}",
                self.session.session_id
            );
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
