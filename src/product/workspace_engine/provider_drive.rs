use super::*;

mod aggregate_writeback;
// `pub(crate)`：REQ-ACS-02 诊断构造（有界 detail / 失败提示）由
// `workspace_engine::tests::artifact_selection` 直接断言。
pub(crate) mod artifact_retry;
mod choice_audit;
mod pending_choices;
// `pub(crate)`：REQ-NDR-05 usage 落盘失败诊断（降级档位 / 有界 detail）由
// `workspace_engine::tests::usage_persistence_diagnostic` 直接断言。
pub(crate) mod usage_diagnostic;
mod watchdog;

use crate::product::lifecycle_store::spec::ExistingSpecRecord;
use crate::product::lifecycle_store::{AggregateDesignSpecScope, AggregateStorySpecScope};
use crate::product::logical_codebase::PlanningContextSnapshotStore;
use crate::product::workspace_engine::aggregate_output_parser::{
    parse_design_aggregate_output, parse_story_aggregate_output,
};
use artifact_retry::artifact_failure_reasons_with_diagnostic;
use choice_audit::ChoiceResponseAuditInput;
pub(crate) use pending_choices::PendingChoiceRequests;
pub(crate) use pending_choices::pending_choice_requests_snapshot;
pub(crate) use watchdog::{PROVIDER_CHOICE_WAIT_TIMEOUT, PROVIDER_IDLE_WATCHDOG_TIMEOUT};

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

        // 第一阶段不实证 Kimi resume 稳定性，排除 artifact retry（同 Pi）
        let retry_context =
            provider_allows_artifact_retry(&self.session.author_provider).then(|| {
                ArtifactRetryContext {
                    provider: provider.clone(),
                    input: input.clone(),
                    attempted: false,
                }
            });
        // F-19：legacy story/design author run 拉起前登记 provider start
        //（SC 面有自己的 reserve_single_candidate_provider_start，不经此处）。
        self.register_provider_start_in_ledger(
            ProviderConversationRole::Author,
            self.session.author_provider.clone(),
        );
        let input = self.attach_tool_policy_audit(input);
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

    /// 策略会话审计接线（Task 3.2，REQ-ENV-09/D7）：policy present 时为 input 绑定
    /// run-bound durable sink（LifecycleStore `tool-policy-run-audit/` 分区）并按
    /// provider run 分配 `role_run_seq`（分配随该 run 的 provider_start 首行落盘
    /// 持久化）。每次 provider run 重新分配（重试 run 独立审计文件）。持久 store
    /// 缺失（内存态 engine）时不接线——真实 adapter 对 policy 会话缺 sink 自身
    /// fail-closed，fake provider 测试路径不受影响。
    pub(crate) fn attach_tool_policy_audit(
        &self,
        mut input: StreamingProviderInput,
    ) -> StreamingProviderInput {
        if input.tool_policy.is_none() || input.audit_sink.is_some() {
            return input;
        }
        let Some(store) = self.lifecycle_store.as_ref() else {
            tracing::warn!(
                "policy provider run without a persistent lifecycle store; durable tool-policy audit is not wired"
            );
            return input;
        };
        let workspace_session_id = input
            .workspace_session_id
            .clone()
            .unwrap_or_else(|| self.session.session_id.clone());
        let role_run_seq = match store.next_tool_policy_role_run_seq(&workspace_session_id) {
            Ok(seq) => seq,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    "tool-policy role_run_seq allocation failed; leaving audit sink unset"
                );
                return input;
            }
        };
        input.audit_sink = Some(
            crate::cross_cutting::tool_policy_audit::RoleRunBoundAuditSink::new(
                std::sync::Arc::new(store.clone()),
                workspace_session_id,
                role_run_seq,
            )
            .into_sink(),
        );
        input
    }

    /// 同 `attach_tool_policy_audit`，但作用于 gateway validated input（内部 input
    /// 重建后原样保留 launch policy）。
    fn attach_tool_policy_audit_to_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
    ) -> crate::cross_cutting::session_launch::ValidatedStreamingProviderInput {
        let (input, launch) = validated.into_parts();
        let input = self.attach_tool_policy_audit(input);
        crate::cross_cutting::session_launch::ValidatedStreamingProviderInput::new(input, launch)
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
        let validated_input = self.attach_tool_policy_audit_to_validated(validated_input);
        let session = gateway
            .start_streaming(validated_input, self.cancel.clone())
            .await
            .map_err(|error| {
                // 诊断直通（claude×轻 握手谜团第 2 轮）：不丢弃 adapter stderr——
                // 尾部（有界）并入 details，stderr 字段同源保留（其余 gateway 校验
                // 错误维持 Display 文案）。同 review/drive.rs 的映射约定。
                let mut mapped = crate::cross_cutting::provider_adapter::ProviderAdapterError {
                    code: crate::protocol::provider_errors::ProviderErrorCode::ProviderUnavailable,
                    details: error.to_string(),
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: None,
                    timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
                    duration_ms: 0,
                };
                if let crate::product::logical_codebase::ProviderGatewayError::Adapter(inner) =
                    &error
                {
                    crate::cross_cutting::provider_adapter::ProviderAdapterError::append_bounded_stderr_tail(
                        &mut mapped.details,
                        &inner.stderr,
                        crate::cross_cutting::provider_adapter::PROVIDER_ERROR_STDERR_TAIL_BYTES,
                    );
                    mapped.stderr = inner.stderr.clone();
                }
                mapped
            });
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

        !content_has_complete_workspace_artifact(full_content, &self.session.workspace_type)
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
                // 诊断直通（claude×轻 握手谜团第 2 轮）：provider 启动失败的 stderr
                // 尾部（有界）并入消息，随 EngineEvent::Error → WS error 消息上浮。
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
        // F-27：挂起集经 PendingChoiceRequests 镜像到进程级登记簿（见文件头
        // 注释），session_state 全量投影据此补挂丢失的 choice 卡。
        let mut pending_choice_requests = PendingChoiceRequests::new(&self.session.session_id);
        // F-19 零活动看门狗：事件/命令任一活动即重置；等待人工权限/选择应答
        // 期间挂起（人工等待由 ApprovalBridge PERMISSION_TIMEOUT 与其
        // PermissionTimeout 事件收口，不是 provider 楔死）。
        let idle_watchdog_timeout = PROVIDER_IDLE_WATCHDOG_TIMEOUT;
        let idle_watchdog =
            tokio::time::sleep_until(tokio::time::Instant::now() + idle_watchdog_timeout);
        tokio::pin!(idle_watchdog);
        let mut waiting_for_permission = false;
        // F-22/F-19b choice 悬置等待界：看门狗对 choice 挂起是「无人应答不是
        // provider 楔死」的设计豁免，但 choice 卡经 broadcast try_send 送达无
        // 重发/无恢复（丢失即永久无应答，v28 0482/0483 三连楔死实测）。本界
        // 补对称语义：pending choice 首次出现起计时，超界即按可诊断失败收口
        //（对照权限面 PERMISSION_TIMEOUT 先例）。清空后再次出现则重新计时。
        let choice_wait_timeout = PROVIDER_CHOICE_WAIT_TIMEOUT;
        let choice_wait_timer =
            tokio::time::sleep_until(tokio::time::Instant::now() + choice_wait_timeout);
        tokio::pin!(choice_wait_timer);

        while events_open {
            tokio::select! {
                _ = &mut idle_watchdog,
                if !waiting_for_permission && pending_choice_requests.is_empty() =>
                {
                    // F-19：provider 会话零活动楔死（触发依据见常量注释）。
                    // 处置对照 handle_permission_timeout 先例：Abort 命令入会话
                    // kill 链 + cancel 触发 adapter 侧 child kill；失败节点带
                    // 稳定原因码；finish_failed_run 把会话转回 Open/
                    // prepare_context——story/design 面的恢复语义即「可重新
                    // 开始生成」（对照 coding 面 AwaitingManualRecovery 的
                    // abort-only：workspace 会话是对话式可重跑面，无需 AMR）。
                    eprintln!(
                        "[aria-cancellation] workspace provider_drive idle_watchdog trigger=provider_idle_watchdog session_id={} role={role:?} timeout_secs={}",
                        self.session.session_id,
                        idle_watchdog_timeout.as_secs()
                    );
                    let message = format!(
                        "provider_idle_watchdog: provider 会话 {} 秒零活动（无事件/命令），疑似楔死，运行已由看门狗中止；可重新开始生成",
                        idle_watchdog_timeout.as_secs()
                    );
                    let _ = session.commands.send(ProviderCommand::Abort).await;
                    cancel.cancel();
                    if let Some(node_id) = node_id.as_deref() {
                        let _ = self.flush_stream_buffer(node_id).await;
                        self.update_timeline_node(
                            node_id,
                            TimelineNodeStatus::Failed,
                            Some(message.clone()),
                        )
                        .await;
                    }
                    let _ = self
                        .event_tx
                        .send(EngineEvent::Error { message })
                        .await;
                    self.finish_failed_run().await;
                    return;
                }
                _ = &mut choice_wait_timer,
                if !pending_choice_requests.is_empty() =>
                {
                    // F-22/F-19b：choice 卡丢失/无人应答超界——处置与看门狗
                    // 触发一致（Abort kill 链 + cancel + 失败节点原因码 +
                    // finish_failed_run 回 prepare_context 可重跑）。
                    let pending_ids: Vec<&str> = pending_choice_requests.ids();
                    eprintln!(
                        "[aria-cancellation] workspace provider_drive choice_wait_timeout trigger=provider_choice_wait_timeout session_id={} role={role:?} pending={:?} timeout_secs={}",
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
                        self.update_timeline_node(
                            node_id,
                            TimelineNodeStatus::Failed,
                            Some(message.clone()),
                        )
                        .await;
                    }
                    let _ = self
                        .event_tx
                        .send(EngineEvent::Error { message })
                        .await;
                    self.finish_failed_run().await;
                    return;
                }
                _ = cancel.cancelled() => {
                    // 诊断打点（claude×轻 握手谜团第 2 轮，不改行为）：驱动循环观察到
                    // engine/run token 被外部取消（谁取消见 ws 侧 trigger 打点）。
                    eprintln!(
                        "[aria-cancellation] workspace provider_drive select_cancelled trigger=engine_cancelled_observed session_id={} role={role:?}",
                        self.session.session_id
                    );
                    if let Some(node_id) = node_id.as_deref() {
                        let _ = self.flush_stream_buffer(node_id).await;
                    }
                    self.finish_aborted_run().await;
                    return;
                }
                command = command_rx.recv(), if commands_open => {
                    // F-19：任何命令活动（含人工权限/选择应答）重置零活动看门狗。
                    idle_watchdog
                        .as_mut()
                        .reset(tokio::time::Instant::now() + idle_watchdog_timeout);
                    match command {
                        Some(ProviderCommand::Abort) => {
                            // 诊断打点：Abort 命令到达驱动循环（来源：ws abort/断连
                            // 清理/新 run 接替的 command_tx.send(Abort)）。
                            eprintln!(
                                "[aria-cancellation] workspace provider_drive abort_command trigger=abort_command session_id={} role={role:?}",
                                self.session.session_id
                            );
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
                            // F-19：人工权限应答到达，解除看门狗挂起（permission
                            // 等待期由 adapter 侧 PERMISSION_TIMEOUT 收口）。
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
                            tracing::info!(choice_id = %id, "engine forwarding choice response");
                            let choice_id = id.clone();
                            let choice_request = pending_choice_requests.remove(&id);
                            if choice_request.is_some() {
                                // F-27R2：应答命中挂起 choice（进程级登记簿已同步
                                // 摘除）——通知 Web runtime 广播全量 session_state，
                                // 已连接 tab 的前端对账据此收敛已答卡。
                                let _ = self
                                    .event_tx
                                    .send(EngineEvent::ChoicePendingChanged)
                                    .await;
                            }
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
                            receipt,
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
                    // F-19：任何 provider 事件重置零活动看门狗。
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
                                    role: "assistant".to_string(),
                                    content,
                                    node_id: node_id.clone(),
                                })
                                .await;
                        }
                        ProviderEvent::PermissionRequest(request) => {
                            // F-19：权限请求悬置期间等待人工输入，看门狗挂起。
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
                            // F-22/F-19b：pending 由空转非空时起算/重置等待界。
                            let choice_wait_started = pending_choice_requests.is_empty();
                            pending_choice_requests.insert(request.clone(), role.wire_label());
                            if choice_wait_started {
                                choice_wait_timer
                                    .as_mut()
                                    .reset(tokio::time::Instant::now() + choice_wait_timeout);
                            }
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
                        // token 用量（best-effort）：映射为 kind=usage 的 execution event，
                        // 经既有管道写入 timeline node detail 与 WS 事件流。
                        ProviderEvent::UsageReport(report) => {
                            self.emit_execution_event(
                                execution_event_from_usage_report(report),
                                node_id.clone(),
                                agent.clone(),
                            )
                            .await;
                        }
                        // 策略审计出口（approval_decision/protocol_warning/
                        // session_terminated）：观测性事件，主循环不消费；durable
                        // 落盘在 Task 3.2 的 LifecycleStore sink 注入。
                        ProviderEvent::ToolPolicyDecision(_)
                        | ProviderEvent::ToolPolicyWarning(_)
                        | ProviderEvent::ToolPolicyTerminated(_) => {}
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
                                    &full_output,
                                    &self.session.workspace_type,
                                )
                                && content_has_complete_workspace_artifact(
                                    &full_content,
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
                                        // REQ-ACS-01/02：同一 raw 源只选一次——失败原因、
                                        // 诊断事件与续写 prompt 共用这一份 gate 结果。
                                        let selection = workspace_artifact_selection(
                                            &completed_output,
                                            &self.session.workspace_type,
                                        );
                                        let diagnostic_error = self
                                            .record_artifact_selection_diagnostic(
                                                node_id.as_deref(),
                                                &selection,
                                            )
                                            .await
                                            .err();
                                        let blocking_reasons =
                                            artifact_failure_reasons_with_diagnostic(
                                                &selection,
                                                diagnostic_error.as_deref(),
                                            );
                                        let retry_input = self.build_artifact_retry_input(
                                            &context.input,
                                            &completed_output,
                                            &blocking_reasons,
                                            completed_provider_session_id.clone(),
                                        );
                                        context.input = retry_input.clone();
                                        Some((context.provider.clone(), retry_input, blocking_reasons))
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            if let Some((provider, retry_input, blocking_reasons)) = retry_start {
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
                                let retry_input = self.attach_tool_policy_audit(retry_input);
                                match provider.start(retry_input, self.cancel.clone()).await {
                                    Ok(next_session) => {
                                        session = next_session;
                                        full_content.clear();
                                        tool_call_titles.clear();
                                        tool_call_commands.clear();
                                        continue;
                                    }
                                    Err(error) => {
                                        // 诊断直通：同入口臂——retry 启动失败的 stderr
                                        // 尾部（有界）并入消息后再上浮 WS error。
                                        let mut message = error.details.clone();
                                        crate::cross_cutting::provider_adapter::ProviderAdapterError::append_bounded_stderr_tail(
                                            &mut message,
                                            &error.stderr,
                                            crate::cross_cutting::provider_adapter::PROVIDER_ERROR_STDERR_TAIL_BYTES,
                                        );
                                        let _ = self
                                            .event_tx
                                            .send(EngineEvent::Error { message })
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
                                let retry_input = self.attach_tool_policy_audit(retry_input);
                                match provider.start(retry_input, self.cancel.clone()).await {
                                    Ok(next_session) => {
                                        session = next_session;
                                        full_content.clear();
                                        tool_call_titles.clear();
                                        tool_call_commands.clear();
                                        continue;
                                    }
                                    Err(error) => {
                                        // 诊断直通：同入口臂——resume 回退重启失败的
                                        // stderr 尾部（有界）并入消息后再上浮 WS error。
                                        let mut message = error.details.clone();
                                        crate::cross_cutting::provider_adapter::ProviderAdapterError::append_bounded_stderr_tail(
                                            &mut message,
                                            &error.stderr,
                                            crate::cross_cutting::provider_adapter::PROVIDER_ERROR_STDERR_TAIL_BYTES,
                                        );
                                        let _ = self
                                            .event_tx
                                            .send(EngineEvent::Error { message })
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
            // 诊断打点：事件流自然结束后的取消后置检查（防竞态分支）。
            eprintln!(
                "[aria-cancellation] workspace provider_drive post_loop_cancelled trigger=engine_cancelled_observed session_id={} role={role:?}",
                self.session.session_id
            );
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
            // 诊断打点：assistant 消息完成前的取消后置检查（complete_assistant_message，
            // 无角色上下文——由调用方 drive 循环的位点打点补充角色）。
            eprintln!(
                "[aria-cancellation] workspace provider_drive complete_cancelled trigger=engine_cancelled_observed session_id={} role=author",
                self.session.session_id
            );
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
        // REQ-ACS-01：唯一 gate-passing 候选才作为产物；零通过/歧义走既有失败分支
        // （retry 判断由调用方在同一 raw 源上先行完成，此处不重新选源）。
        let selection = workspace_artifact_selection(&full_content, &self.session.workspace_type);
        // REQ-ACS-02：候选选择发生/失败一律落一条有界诊断（durable-only，原文零复制）；
        // 诊断持久化失败只作为失败摘要里的有界提示，不放宽 gate、不额外启动 provider。
        let diagnostic_node_id = self.active_node_id.clone();
        let diagnostic_error = self
            .record_artifact_selection_diagnostic(diagnostic_node_id.as_deref(), &selection)
            .await
            .err();
        let Some(artifact_markdown) = selected_artifact_markdown(&selection) else {
            let blocking_reasons =
                artifact_failure_reasons_with_diagnostic(&selection, diagnostic_error.as_deref());
            if artifact_retry_attempted {
                self.finish_invalid_workspace_artifact_after_retry(&blocking_reasons)
                    .await;
            } else {
                self.finish_invalid_workspace_artifact(&blocking_reasons)
                    .await;
            }
            return;
        };
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
        // spec-design-dialog-revision T4：author 反馈修订完成后回 AuthorConfirm，summary 携带改动摘要
        // （brief 未覆盖提取机制，按现有 Revision 完成路径最小适配：从产物「## 改动摘要」小节提取）。
        // T5/M-1：谓词提取为共享 helper is_author_feedback_revision（decisions.rs）。
        let confirm_summary = if self.is_author_feedback_revision() {
            extract_changelog_summary(&artifact_markdown)
                .map(|changelog| format!("修订完成。\n\n## 改动摘要\n{changelog}"))
                .unwrap_or_else(|| "等待用户确认 author 结果".to_string())
        } else {
            "等待用户确认 author 结果".to_string()
        };
        self.complete_active_node(Some("生成完成".to_string()))
            .await;
        self.enter_author_confirm(Some(confirm_summary)).await;
    }
}

/// 从修订产物 markdown 提取「## 改动摘要」小节正文（到下一个二级标题或文末为止）。
/// spec-design-dialog-revision T4：author 反馈修订完成后，AuthorConfirm summary 携带改动摘要。
pub(crate) fn extract_changelog_summary(markdown: &str) -> Option<String> {
    const HEADER: &str = "## 改动摘要";
    let start = markdown.find(HEADER)?;
    let rest = &markdown[start + HEADER.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    let section = rest[..end].trim();
    if section.is_empty() {
        None
    } else {
        Some(section.to_string())
    }
}

pub(crate) fn provider_allows_artifact_retry(provider: &ProviderName) -> bool {
    !matches!(provider, ProviderName::Pi | ProviderName::KimiCode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kimi_is_excluded_from_artifact_retry() {
        assert!(!provider_allows_artifact_retry(&ProviderName::KimiCode));
        assert!(!provider_allows_artifact_retry(&ProviderName::Pi));
        assert!(provider_allows_artifact_retry(&ProviderName::Codex));
    }
}

mod work_item_plan;
