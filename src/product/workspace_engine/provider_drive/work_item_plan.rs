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
                // 诊断直通（claude×轻 握手谜团第 2 轮）：workitem author 启动失败的
                // stderr 尾部（有界）并入消息——随 EngineEvent::Error → WS error
                // 消息上浮，并在驱动 result（Err 返回值）中同源携带。
                let mut message = error.details.clone();
                crate::cross_cutting::provider_adapter::ProviderAdapterError::append_bounded_stderr_tail(
                    &mut message,
                    &error.stderr,
                    crate::cross_cutting::provider_adapter::PROVIDER_ERROR_STDERR_TAIL_BYTES,
                );
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
        // F-27R3：挂起集经 PendingChoiceRequests 镜像到进程级登记簿（对照主驱动
        // provider_drive 先例）——session_state 全量投影据此补挂本子驱动挂起的
        // choice 卡；guard Drop（本函数任意出口）只摘本 run 登记的 id，防泄漏。
        let mut pending_choice_requests = PendingChoiceRequests::new(&self.session.session_id);
        // r29 问题2 根修(r28 现场):本循环此前缺 F-19 零活动看门狗(主驱动
        // drive_provider_session 有)——SC 门修订 turn 的 provider 流式 1min 后
        // 静默挂死,驱动永不收口,harness 侧 90min 空等零信号。同构补齐:
        // 事件/命令任一活动重置;等待人工权限/选择应答期间挂起(与主驱动
        // 同语义——人工等待由 ApprovalBridge PERMISSION_TIMEOUT 与
        // choice_wait 界收口,不是 provider 楔死)。
        // r32 校准:plan 文档生成的合法静默可超 10min(r28 实测 648s),本循环
        // 用 1800s 专用界(主驱动保持 600s——story/design 逐 token 流式形态
        // 不同);r32 现场 600s 界曾误杀合法慢生成。
        let idle_watchdog_timeout = PROVIDER_WORK_ITEM_PLAN_IDLE_WATCHDOG_TIMEOUT;
        let idle_watchdog =
            tokio::time::sleep_until(tokio::time::Instant::now() + idle_watchdog_timeout);
        tokio::pin!(idle_watchdog);
        let choice_wait_timeout = PROVIDER_CHOICE_WAIT_TIMEOUT;
        let choice_wait_timer =
            tokio::time::sleep_until(tokio::time::Instant::now() + choice_wait_timeout);
        tokio::pin!(choice_wait_timer);
        let mut waiting_for_permission = false;
        while events_open {
            tokio::select! {
                _ = &mut idle_watchdog,
                if !waiting_for_permission && pending_choice_requests.is_empty() =>
                {
                    // F-19 同构:provider 会话零活动楔死——Abort kill 链 + cancel +
                    // 失败节点带稳定原因码 + Error 事件 + finish_failed_run
                    //(workspace 会话回 prepare_context 可重跑)。
                    eprintln!(
                        "[aria-cancellation] workspace work_item_plan_drive idle_watchdog trigger=provider_idle_watchdog session_id={} role=author timeout_secs={}",
                        self.session.session_id,
                        idle_watchdog_timeout.as_secs()
                    );
                    let message = format!(
                        "provider_idle_watchdog: provider 会话 {} 秒零活动（无事件/命令），疑似楔死，运行已由看门狗中止；可重新开始生成",
                        idle_watchdog_timeout.as_secs()
                    );
                    let _ = session.commands.send(ProviderCommand::Abort).await;
                    cancel.cancel();
                    let display_content = display_filter.finish();
                    self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                    self.update_timeline_node(
                        &node_id,
                        TimelineNodeStatus::Failed,
                        Some(message.clone()),
                    )
                    .await;
                    let _ = self
                        .event_tx
                        .send(EngineEvent::Error { message })
                        .await;
                    self.finish_failed_run().await;
                    return Err("provider_idle_watchdog: work item plan provider 会话零活动楔死，运行已中止".to_string());
                }
                _ = &mut choice_wait_timer,
                if !pending_choice_requests.is_empty() =>
                {
                    // F-22/F-19b 同构:choice 卡丢失/无人应答超界。
                    let pending_ids: Vec<&str> = pending_choice_requests.ids();
                    eprintln!(
                        "[aria-cancellation] workspace work_item_plan_drive choice_wait_timeout trigger=provider_choice_wait_timeout session_id={} role=author pending={:?} timeout_secs={}",
                        self.session.session_id,
                        pending_ids,
                        choice_wait_timeout.as_secs()
                    );
                    let message = format!(
                        "provider_choice_wait_timeout: 等待回答超时——{} 秒内未收到选择应答，运行已中止；choice 卡可能未送达或已送达但无人应答（pending={:?}）。请重新提交反馈重新发起本轮",
                        choice_wait_timeout.as_secs(),
                        pending_ids
                    );
                    let _ = session.commands.send(ProviderCommand::Abort).await;
                    cancel.cancel();
                    let display_content = display_filter.finish();
                    self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                    self.update_timeline_node(
                        &node_id,
                        TimelineNodeStatus::Failed,
                        Some(message.clone()),
                    )
                    .await;
                    let _ = self
                        .event_tx
                        .send(EngineEvent::Error { message })
                        .await;
                    self.finish_failed_run().await;
                    return Err("provider_choice_wait_timeout: work item plan provider 等待选择应答超时，运行已中止".to_string());
                }
                _ = cancel.cancelled() => {
                    // 诊断打点（claude×轻 握手谜团第 2 轮，不改行为）：workitem author
                    // 驱动循环观察到 engine/run token 被外部取消。
                    eprintln!(
                        "[aria-cancellation] workspace work_item_plan_drive select_cancelled trigger=engine_cancelled_observed session_id={} role=author agent={agent:?}",
                        self.session.session_id
                    );
                    let display_content = display_filter.finish();
                    self.emit_work_item_plan_display_chunk(&node_id, display_content).await;
                    let _ = self.flush_stream_buffer(&node_id).await;
                    self.finish_aborted_run().await;
                    return Err("provider run aborted".to_string());
                }
                command = command_rx.recv(), if commands_open => {
                    // F-19 同构:任何命令活动(含人工权限/选择应答)重置看门狗。
                    idle_watchdog
                        .as_mut()
                        .reset(tokio::time::Instant::now() + idle_watchdog_timeout);
                    match command {
                        Some(ProviderCommand::Abort) => {
                            // 诊断打点：Abort 命令到达 workitem author 驱动循环。
                            eprintln!(
                                "[aria-cancellation] workspace work_item_plan_drive abort_command trigger=abort_command session_id={} role=author agent={agent:?}",
                                self.session.session_id
                            );
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
                            // 人工权限应答到达:解除看门狗挂起(与主驱动同语义)。
                            waiting_for_permission = false;
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
                            receipt,
                        }) => {
                            // F-27R3：应答命中挂起 choice（登记簿同步摘除）——
                            // 通知 Web runtime 广播全量 session_state 收敛已答卡
                            //（对照主驱动先例）。
                            if pending_choice_requests.remove(&id).is_some() {
                                let _ = self
                                    .event_tx
                                    .send(EngineEvent::ChoicePendingChanged)
                                    .await;
                            }
                            if session.commands.send(ProviderCommand::ChoiceResponse {
                                id,
                                selected_option_ids,
                                free_text,
                                answers,
                            receipt,
                            }).await.is_err() {
                                commands_open = false;
                            }
                        }
                        Some(ProviderCommand::ToolResult(_)) => {}
                        None => commands_open = false,
                    }
                }
                event = session.events.recv() => {
                    // F-19 同构:事件活动重置看门狗。
                    idle_watchdog
                        .as_mut()
                        .reset(tokio::time::Instant::now() + idle_watchdog_timeout);
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
                            // F-19 同构:等待人工权限应答期间看门狗不计时。
                            waiting_for_permission = true;
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
                            // F-27R3：登记进程级挂起集（三驱动统一登记簿）——
                            // session_state 投影经登记簿携带本卡。
                            pending_choice_requests.insert(request.clone(), "author");
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
                        ProviderEvent::Completed(completion) => {
                            let full_output = completion.full_output;
                            let provider_session_id = completion.provider_session_id;
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
                        // token 用量（F-40）：与主循环 / reviewer 驱动 / coding 引擎同构，
                        // 映射为 kind=usage 的 execution event——同时落 node detail 与 WS
                        // 事件流（此前本链路是空分支：计划拆分 author 的 token 在数据面
                        // 整体缺失，author 阶段卡/气泡因此无读数，仅 Review 卡有）。
                        ProviderEvent::UsageReport(report) => {
                            self.emit_execution_event(
                                execution_event_from_usage_report(report),
                                Some(node_id.clone()),
                                Some(agent.clone()),
                            )
                            .await;
                        }
                        // 策略审计出口（approval_decision/protocol_warning/
                        // session_terminated）：观测性事件，engine 主循环不消费；
                        // durable 落盘在 Task 3.2 的 LifecycleStore sink 注入。
                        ProviderEvent::ToolPolicyDecision(_)
                        | ProviderEvent::ToolPolicyWarning(_)
                        | ProviderEvent::ToolPolicyTerminated(_) => {}
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
            let persisted = self
                .update_node_detail(node_id, |detail| {
                    upsert_execution_event_json(&mut detail.execution_events, event_json);
                })
                .await;
            // REQ-NDR-05：usage 事件的 durable 落盘失败此前被 `let _ =` 静默吞掉，导致
            // 「节点无 usage」无法区分「本来没有」与「落盘失败」。这里按档位留痕
            // （节点 detail 诊断事件 → 会话级 usage-diagnostics.jsonl → 结构化 warn），
            // 不改变事件广播与 gate/provider 行为。
            if let Err(error) = persisted
                && event.kind == ProviderExecutionEventKind::Usage
            {
                self.record_usage_persist_diagnostic(node_id, &event.event_id, &error)
                    .await;
            }
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
