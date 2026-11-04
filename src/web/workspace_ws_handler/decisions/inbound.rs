use super::*;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::product::workspace_engine::{exclude_only_feedback_requested, normalized_finding_keys};
use std::pin::Pin;

#[derive(Clone)]
pub(crate) struct WorkspaceInboundContext {
    pub(crate) app_state: WebAppState,
    pub(crate) engine: Arc<Mutex<WorkspaceEngine>>,
    pub(crate) run_context: ProviderRunContext,
    pub(crate) outbound_tx: mpsc::Sender<OutboundControl>,
    pub(crate) session_id: String,
}

pub(crate) fn provider_run_kind_for_interrupted_recovery(
    outcome: InterruptedRunRecoveryOutcome,
) -> ProviderRunKind {
    match outcome {
        InterruptedRunRecoveryOutcome::Review => ProviderRunKind::ReviewOnly,
        InterruptedRunRecoveryOutcome::WorkItemPlanAuthorGeneration { flow_kind } => {
            ProviderRunKind::work_item_plan_author_for_durable_flow(flow_kind)
        }
        InterruptedRunRecoveryOutcome::WorkItemDraftGeneration => {
            ProviderRunKind::WorkItemPlanDraft { feedback: None }
        }
        // spec-design-dialog-revision T7：Revision 恢复臂 → 修订 run。
        InterruptedRunRecoveryOutcome::Revision => ProviderRunKind::Revision,
    }
}

pub(crate) async fn finish_interrupted_recovery_spawn_error(
    engine: &Arc<Mutex<WorkspaceEngine>>,
    message: &str,
) {
    engine
        .lock()
        .await
        .finish_active_run_with_failed_node(message.to_string())
        .await;
}

pub(crate) fn handle_workspace_inbound_message<E>(
    context: WorkspaceInboundContext,
    envelope: E,
) -> Pin<Box<dyn Future<Output = ()> + Send>>
where
    E: Into<WorkspaceInboundEnvelope>,
{
    // 巨型 match 分发是库内最大 async 状态机之一；socket select 嵌套 + 深链
    // 测试（campaign_stage3 族）在线程栈上贴边（曾因加臂溢出）。整体装箱：
    // 语义零变化（WS 入站消息频率低，一次堆分配可忽略），后续加臂不再
    // 影响调用方栈深。
    Box::pin(handle_workspace_inbound_message_inner(
        context,
        envelope.into(),
    ))
}

async fn handle_workspace_inbound_message_inner(
    context: WorkspaceInboundContext,
    envelope: WorkspaceInboundEnvelope,
) {
    let WorkspaceInboundContext {
        app_state,
        engine,
        run_context,
        outbound_tx,
        session_id,
    } = context;

    // `ProviderRunContext.session_record` is the immutable session snapshot captured
    // when the websocket was established; reading flow_kind here must not wait for
    // the engine mutex held by a provider run.
    let scope_submission_error = single_candidate_scope_submission_error(
        run_context.session_record.flow_kind,
        &envelope.submitted_fields,
    );
    if let Some(err) = scope_submission_error {
        let _ = send_json_outbound(&outbound_tx, &err).await;
        return;
    }

    if let WsInMessage::HumanGateFeedback { command_id, .. } = &envelope.message
        && let Err(err) = validate_command_id(command_id)
    {
        let _ = send_json_outbound(&outbound_tx, &err).await;
        return;
    }

    if run_context.session_record.flow_kind == WorkItemPlanFlowKind::SingleCandidate
        && ((requires_stage_validation(&envelope.message)
            // C3/REQ-HTR-01：Abort 的门态拒收已在 socket 层非阻塞判定；此处
            // 不得再等 engine 锁——run 持锁期 Abort 必须即刻到达取消臂。
            && !matches!(envelope.message, WsInMessage::Abort))
            || matches!(envelope.message, WsInMessage::UserMessage { .. }))
    {
        let stage = engine.lock().await.current_stage();
        if let Some(err) = human_gate_message_boundary_error(
            run_context.session_record.flow_kind,
            stage,
            &envelope.message,
        ) {
            let _ = send_json_outbound(&outbound_tx, &err).await;
            return;
        }
    }

    match envelope.message {
        WsInMessage::HumanGateFeedback {
            command_id,
            feedback,
            excluded_finding_ids,
        } => {
            if let Err(err) = validate_command_id(&command_id) {
                let _ = send_json_outbound(&outbound_tx, &err).await;
                return;
            }
            // REQ-WSC-13：owner 的逐条「排除此项」先落 durable 已处置事实
            // 清单（下一轮评审 prompt 携带它）；空＝旧客户端，零副作用。
            //
            // k3 P0-2A（修法 A）：`feedback` 为空且带合法排除 key 的帧是
            // **只表达处置事实**的排除-only 命令——落 durable 后即受理，不建
            // 修订 turn、不跑 `validate_feedback`。否则它必然撞上
            // `INVALID_HUMAN_GATE_FEEDBACK · feedback must not be blank`，而
            // 门卡已经显示「已排除」——正是本 change 要消灭的「不知道如何
            // 继续」。反馈非空时行为逐字不变。
            let exclude_only = exclude_only_feedback_requested(&feedback, &excluded_finding_ids);
            let mut exclusion_error = None;
            if !excluded_finding_ids.is_empty() {
                let recorded = engine
                    .lock()
                    .await
                    .record_owner_finding_exclusions(&excluded_finding_ids)
                    .await;
                match recorded {
                    Ok(0) => {}
                    Ok(count) => eprintln!(
                        "[aria-gate-disposition] owner excluded findings session_id={} count={count}",
                        run_context.session_record.id
                    ),
                    Err(error) => {
                        eprintln!(
                            "[aria-gate-disposition] record exclusions failed session_id={} error={error}",
                            run_context.session_record.id
                        );
                        exclusion_error = Some(error);
                    }
                }
            }
            if exclude_only {
                match exclusion_error {
                    // 受理回执：界面据此确认「已排除」成立（不是失败的乐观态）。
                    None => {
                        let _ = send_json_outbound(
                            &outbound_tx,
                            &WsOutMessage::HumanGateExclusionsAccepted {
                                command_id,
                                excluded_finding_ids: normalized_finding_keys(
                                    &excluded_finding_ids,
                                ),
                            },
                        )
                        .await;
                    }
                    // 落盘失败不得冒充已受理（REQ-WSC-15「发起失败不显示为
                    // 已排除」）。
                    Some(error) => {
                        let _ = send_json_outbound(
                            &outbound_tx,
                            &WsOutMessage::Error {
                                message: format!("排除项没有落盘，请重试：{error}"),
                            },
                        )
                        .await;
                    }
                }
                return;
            }
            handle_human_gate_feedback_from_handler(
                run_context.clone(),
                outbound_tx.clone(),
                HumanGateFeedbackInput {
                    command_id,
                    feedback,
                },
            )
            .await;
        }
        WsInMessage::Advance { command_id } => {
            if let Err(err) = validate_command_id(&command_id) {
                let _ = send_json_outbound(&outbound_tx, &err).await;
                return;
            }
            let stage = engine.lock().await.current_stage();
            let message = WsInMessage::Advance {
                command_id: command_id.clone(),
            };
            if !is_message_valid_for_stage_with_flow(
                run_context.session_record.flow_kind,
                &message,
                &stage,
            ) {
                let err =
                    advance_stage_error(command_id, &stage, run_context.session_record.flow_kind);
                let _ = send_json_outbound(&outbound_tx, &err).await;
                return;
            }
            handle_advance_from_handler(
                app_state.clone(),
                run_context.clone(),
                outbound_tx.clone(),
                command_id,
            )
            .await;
        }
        WsInMessage::AbandonHumanGate { command_id } => {
            if let Err(err) = validate_command_id(&command_id) {
                let _ = send_json_outbound(&outbound_tx, &err).await;
                return;
            }
            // L0 typed 重承载（REQ-RET-02/REQ-CG-04）：abandon 显式 typed 门命令，
            // command_id 为幂等/审计键（与 HumanGateFeedback/Advance 同族；
            // 关门幂等由 durable 会话单飞语义承载——先到者赢）。调用点
            // Box::pin：本函数 match 已是巨型 async 状态机，低频人机命令
            // 堆上执行，避免测试线程栈贴边溢出（campaign_stage3_advance 族）。
            Box::pin(handle_human_gate_termination_from_handler(
                run_context.clone(),
                outbound_tx.clone(),
                HumanGateCloseDecision::Abandon,
            ))
            .await;
        }
        WsInMessage::UserMessage { content } => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context.clone(),
                ProviderRunKind::Author { content },
                outbound_tx.clone(),
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        WsInMessage::Rollback { checkpoint_id } => {
            // 诊断打点：Rollback 入口先取消 active run（engine/run token）。
            eprintln!(
                "[aria-cancellation] workspace ws_rollback_message trigger=ws_rollback_message session_id={} checkpoint_id={checkpoint_id}",
                session_id
            );
            run_context.manager.abort_active_run().await;
            let mut engine = engine.lock().await;
            if let Err(e) = engine.handle_rollback(&checkpoint_id).await {
                let err = WsOutMessage::Error { message: e };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            } else {
                let mut state_msg = engine.build_session_state();
                // 对外出口统一以 durable enrollment 覆盖 automation 归属
                //（真实链 2026-10-24：裸帧携带引擎缺省 client/off 会让
                // 前端 automationManaged 误判、重现接管死路）。
                run_context
                    .manager
                    .project_automation_ownership(&mut state_msg);
                // REQ-HGC-04：入站消息后的补帧同样携带「继续自动化」入口事实。
                run_context
                    .manager
                    .project_plan_continue_available(&mut state_msg);
                let _ = send_json_outbound(&outbound_tx, &state_msg).await;
            }
        }
        WsInMessage::Confirm => {
            if run_context.session_record.flow_kind == WorkItemPlanFlowKind::SingleCandidate {
                handle_human_gate_termination_from_handler(
                    run_context.clone(),
                    outbound_tx.clone(),
                    HumanGateCloseDecision::Approve,
                )
                .await;
            } else {
                // L2 退役收口：非 SC 流的 approve 帧直连引擎确认（原
                // human-confirm 决策桥接随消息族删除）。
                handle_confirm_from_handler(run_context.clone(), outbound_tx.clone()).await;
            }
        }
        WsInMessage::ProviderSelect { role, provider } => {
            let result = {
                let mut engine = engine.lock().await;
                engine.set_provider(&role, provider)
            };
            if let Err(e) = result {
                let err = WsOutMessage::Error { message: e };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            } else {
                if let Err(message) =
                    refresh_workspace_context_for_session(&run_context, engine.clone()).await
                {
                    let err = WsOutMessage::Error { message };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                    return;
                }
                let mut state_msg = {
                    let engine = engine.lock().await;
                    engine.build_session_state()
                };
                // 对外出口统一以 durable enrollment 覆盖 automation 归属
                //（裸帧携带引擎缺省 client/off 会让前端 automationManaged
                // 误判、重现接管死路）。
                run_context
                    .manager
                    .project_automation_ownership(&mut state_msg);
                // REQ-HGC-04：入站消息后的补帧同样携带「继续自动化」入口事实。
                run_context
                    .manager
                    .project_plan_continue_available(&mut state_msg);
                let _ = send_json_outbound(&outbound_tx, &state_msg).await;
            }
        }
        WsInMessage::PermissionResponse {
            id,
            approved,
            reason,
        } => {
            tracing::info!(permission_id = %id, approved, "ws inbound permission response");
            let command_tx = run_context.manager.active_run_command_tx().await;
            if let Some(command_tx) = command_tx {
                let _ = command_tx
                    .send(ProviderCommand::PermissionResponse {
                        id,
                        approved,
                        reason,
                    })
                    .await;
            } else {
                let _ = send_json_outbound(
                    &outbound_tx,
                    &missing_active_run_error("permission_response", &id),
                )
                .await;
            }
        }
        WsInMessage::ChoiceResponse {
            id,
            selected_option_ids,
            free_text,
            answers,
            command_id,
            expected_run_id,
        } => {
            tracing::info!(choice_id = %id, "ws inbound choice response");
            let active_run = run_context.manager.active_run().await;
            if let Some(run) = active_run {
                // P0 1.3（REQ-WIGA-05）：WS 与 REST 共用 claim 门面——缺省
                // command_id/expected_run_id 只绑当前唯一 run；pending 帧在
                // 回执 Delivered 后才由 manager 摘除（不再发送前移除）。
                let answer_data: Vec<crate::cross_cutting::streaming_provider::ChoiceAnswerData> =
                    answers
                        .clone()
                        .into_iter()
                        .map(
                            |answer| crate::cross_cutting::streaming_provider::ChoiceAnswerData {
                                question_id: answer.question_id,
                                selected_option_ids: answer.selected_option_ids,
                                free_text: answer.free_text,
                            },
                        )
                        .collect();
                let request = match (command_id.clone(), expected_run_id.clone()) {
                    (Some(command_id), Some(expected_run_id)) => {
                        crate::web::choice_reply::ChoiceResponseRequest {
                            command_id,
                            expected_run_id,
                            answers: answer_data,
                        }
                    }
                    _ => match run_context
                        .manager
                        .bind_current_run_request(&id, answer_data)
                    {
                        Some(request) => request,
                        None => {
                            let _ =
                                send_json_outbound(&outbound_tx, &choice_id_unmatched_error(&id))
                                    .await;
                            return;
                        }
                    },
                };
                if request.expected_run_id != run.run_incarnation {
                    let err = WsOutMessage::ProtocolError {
                        code: "CHOICE_RUN_EXPIRED".to_string(),
                        message: format!(
                            "ChoiceResponse id={id} targets a finished run; the pending choice must be re-answered on the current run"
                        ),
                        context: Some(serde_json::json!({ "id": id })),
                    };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                    return;
                }
                match run_context.manager.claim_choice(&id, &request) {
                    Ok((_, true)) => match run_context
                        .manager
                        .submit_claimed_choice(&id, &request)
                        .await
                    {
                        Ok(_) => {
                            return;
                        }
                        Err(error) => {
                            let _ =
                                send_json_outbound(&outbound_tx, &choice_claim_error(&id, &error))
                                    .await;
                            return;
                        }
                    },
                    // 同 command 幂等重发：已认领不二发，等待 Delivered 收敛。
                    Ok((_, false)) => return,
                    Err(error) => {
                        let _ = send_json_outbound(&outbound_tx, &choice_claim_error(&id, &error))
                            .await;
                        return;
                    }
                }
            }

            let prompt = {
                let mut engine = engine.lock().await;
                engine
                    .take_pending_author_choice_prompt(&id, selected_option_ids, free_text)
                    .await
            };
            match prompt {
                Ok(content) => {
                    if let Err(message) = spawn_provider_run_from_handler(
                        run_context.clone(),
                        ProviderRunKind::AuthorChoiceFollowup { content },
                        outbound_tx.clone(),
                    )
                    .await
                    {
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                }
                Err(PendingAuthorChoiceError::NotFound { .. }) => {
                    let _ = send_json_outbound(
                        &outbound_tx,
                        &missing_active_run_error("choice_response", &id),
                    )
                    .await;
                }
                Err(error) => {
                    let err = WsOutMessage::ProtocolError {
                        code: error.code().to_string(),
                        message: error.message(),
                        context: Some(serde_json::json!({ "id": id })),
                    };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
            }
        }
        // 退役留档（T5/REQ-RET-02）：legacy 决策路由分支已删除——review 决策
        // 应答/作者决策/生成模式选择/outline 修订请求/草稿决策/批量决策/
        // presentation 修订保存消息族的 wire 名在 parse 面即被拒收
        // （LEGACY_MESSAGE_RETIRED），消费链与处置见 wp5-attribution-table.md。
        WsInMessage::WorkItemPlanCompileRecoveryAction { action, reason } => {
            let result = {
                let mut engine = engine.lock().await;
                engine
                    .handle_work_item_plan_compile_recovery_action(action, reason)
                    .await
            };
            match result {
                Ok(WorkItemPlanCompileRecoveryOutcome::Continue)
                | Ok(WorkItemPlanCompileRecoveryOutcome::HumanConfirm) => {}
                Err(message) => {
                    let err = WsOutMessage::ProtocolError {
                        code: "INVALID_COMPILE_RECOVERY_ACTION".to_string(),
                        message,
                        context: None,
                    };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
            }
        }
        // 退役留档（T5/REQ-RET-02）：SaveHumanPresentationRevision 消息族已
        // 删除（SC 无消费，wp5-attribution-table.md §2）——wire 名 parse 面
        // 拒收，路由分支随之退役。
        WsInMessage::Abort => {
            // 诊断打点：显式 Abort 消息（driver/前端）取消 active run。
            eprintln!(
                "[aria-cancellation] workspace ws_abort_message trigger=ws_abort_message session_id={}",
                session_id
            );
            if run_context.manager.abort_active_run().await {
                let _ = send_json_outbound(
                    &outbound_tx,
                    &WsOutMessage::ProviderStatus {
                        status: WsProviderStatus::Aborted,
                    },
                )
                .await;
            } else {
                // C3/REQ-HTR-01：无 active run 的 Abort 不再静默零回执——返回
                // 稳定码 abort_no_active_run + 当前阶段（门开态附门级动作指路）。
                // stage 读取放独立任务：错误回执不在取消关键路径上，等锁不阻塞
                // socket 读循环。
                let engine_for_error = engine.clone();
                let outbound_for_error = outbound_tx.clone();
                tokio::spawn(async move {
                    let stage = engine_for_error.lock().await.current_stage();
                    let err = crate::web::workspace_ws_handler::abort_no_active_run_error(&stage);
                    let _ = send_json_outbound(&outbound_for_error, &err).await;
                });
            }
        }
        // trust 指引卡「登记并重试」按钮（owner 2026-10-12 直批）：用户
        // 显式授权的按需登记——root 服务端解析、复用 ensure 登记链、typed
        // 回执/稳定等待码；不自动重新生成。阶段矩阵豁免（协议面
        // requires_stage_validation）：带外修复动作，不触碰 run 状态，
        // 门开态（确认链 trust 等待）同样可用。
        WsInMessage::RegisterTrustEntry { provider } => {
            handle_register_trust_entry_from_handler(
                app_state.clone(),
                engine.clone(),
                outbound_tx.clone(),
                provider,
            )
            .await;
        }
        WsInMessage::Ping => {
            let _ = send_json_outbound(&outbound_tx, &WsOutMessage::Pong).await;
        }
        WsInMessage::Hello {
            role,
            after_event_seq,
            ..
        } => {
            run_context.manager.bind_role(
                &outbound_tx,
                crate::web::workspace_session::ConnectionRole::normalize(role),
                after_event_seq,
            );
            if let Some(after_event_seq) = after_event_seq {
                let Some(connection_id) = run_context.connection_id.as_deref() else {
                    return;
                };
                run_context
                    .manager
                    .resubscribe(&outbound_tx, connection_id, after_event_seq)
                    .await;
            } else {
                let engine_for_hello = engine.clone();
                let outbound_for_hello = outbound_tx.clone();
                // 无 cursor Hello 的全量基线帧同样过 durable 归属投影——
                //（真实链 2026-10-24，workspace_session_auto_261ab5db 现场）
                // 此前裸发 `engine.build_session_state()`（automation 为引擎
                // 缺省 client/off）且晚于 socket 初帧到达，观察/驾驶连接的
                // 前端 automationManaged 被“伪 client”覆盖 → 精确绑定
                // enabled enrollment 的 stopped_needs_human 收件箱误现
                // 「接管」按钮，服务端 takeover 拒 409 形成死路。
                let manager_for_hello = run_context.manager.clone();
                tokio::spawn(async move {
                    let mut state_msg = {
                        let engine = engine_for_hello.lock().await;
                        engine.build_session_state()
                    };
                    manager_for_hello.project_automation_ownership(&mut state_msg);
                    // REQ-HGC-04：Hello 基线帧（重连路径）同样携带「继续自动化」
                    // 入口事实——否则重连后门卡变假指引。
                    manager_for_hello.project_plan_continue_available(&mut state_msg);
                    let _ = send_json_outbound(&outbound_for_hello, &state_msg).await;
                });
            }
        }
        WsInMessage::ContextNote { content } => {
            let result = {
                let mut engine = engine.lock().await;
                engine.append_context_note(content).await
            };
            if let Err(message) = result {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        WsInMessage::StartGeneration {
            provider_config,
            reviewer_enabled,
            resume_recovery,
        } => {
            if let Some(intent) = resume_recovery {
                // B1b（Task 4.2，设计 §5.1）：显式 fresh——socket 入站门已做
                // 身份/同 ID Replay/配置/source 前置核验，此处为 2.5 合同的
                // 权威 CAS claim（`claim_lc_fresh`）+ typed OperationResult
                // sender（A2.2 移交锚：本 decisions 消费面）。claim 后的
                // provider 启动/tag dispatch 归 B2（5.1+），本包零 spawn。
                handle_lc_resume_recovery_fresh(run_context.clone(), outbound_tx.clone(), intent)
                    .await;
                return;
            }
            let result = {
                let mut engine = engine.lock().await;
                engine
                    .start_generation(provider_config, reviewer_enabled)
                    .await
            };
            match result {
                Ok((_node, locked)) => {
                    let run_context =
                        match refresh_workspace_context_for_session(&run_context, engine.clone())
                            .await
                        {
                            Ok(run_context) => run_context,
                            Err(message) => {
                                let err = WsOutMessage::Error { message };
                                let _ = send_json_outbound(&outbound_tx, &err).await;
                                return;
                            }
                        };
                    let _ = send_json_outbound(&outbound_tx, &locked).await;
                    let run_kind = {
                        let engine = engine.lock().await;
                        if engine.session().workspace_type == WorkspaceType::WorkItemPlan {
                            ProviderRunKind::work_item_plan_author_for_durable_flow(
                                run_context.session_record.flow_kind,
                            )
                        } else {
                            ProviderRunKind::Author {
                                content: String::new(),
                            }
                        }
                    };
                    if let Err(message) = spawn_provider_run_from_handler(
                        run_context.clone(),
                        run_kind,
                        outbound_tx.clone(),
                    )
                    .await
                    {
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                }
                Err(message) => {
                    let err = WsOutMessage::Error { message };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
            }
        }
        WsInMessage::RetryInterruptedRun { failed_node_id } => {
            let result = {
                let mut engine = engine.lock().await;
                engine.retry_interrupted_run(&failed_node_id).await
            };
            match result {
                Ok(outcome) => {
                    let run_kind = provider_run_kind_for_interrupted_recovery(outcome);
                    if let Err(message) = spawn_provider_run_from_handler(
                        run_context.clone(),
                        run_kind,
                        outbound_tx.clone(),
                    )
                    .await
                    {
                        finish_interrupted_recovery_spawn_error(&engine, &message).await;
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                }
                Err(error) => {
                    let err = WsOutMessage::ProtocolError {
                        code: error.code().to_string(),
                        message: error.to_string(),
                        context: Some(serde_json::json!({
                            "failed_node_id": failed_node_id,
                        })),
                    };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
            }
        }
        // 退役留档（T5/REQ-RET-02）：SelectRevisionPath 消息族已删除（必删集，
        // wp5-attribution-table.md §1）——wire 名 parse 面拒收，路由分支退役。
        WsInMessage::RequestRevision { feedback } => {
            let is_work_item_plan = {
                let engine = engine.lock().await;
                engine.session().workspace_type == WorkspaceType::WorkItemPlan
            };
            let feedback_text = {
                let description = feedback.description.trim().to_string();
                if description.is_empty() {
                    None
                } else {
                    Some(description)
                }
            };
            if is_work_item_plan {
                let result = {
                    let mut engine = engine.lock().await;
                    engine
                        .request_work_item_plan_revision(feedback_text.clone())
                        .await
                };
                match result {
                    Ok(ReviewDecisionOutcome::StartWorkItemPlanOutline) => {
                        let run_kind = ProviderRunKind::work_item_plan_author_for_durable_flow(
                            run_context.session_record.flow_kind,
                        );
                        if let Err(message) = spawn_provider_run_from_handler(
                            run_context.clone(),
                            run_kind,
                            outbound_tx.clone(),
                        )
                        .await
                        {
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx, &err).await;
                        }
                    }
                    Ok(ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback }) => {
                        if let Err(message) = spawn_provider_run_from_handler(
                            run_context.clone(),
                            ProviderRunKind::WorkItemPlanOutlineRevision { feedback },
                            outbound_tx.clone(),
                        )
                        .await
                        {
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx, &err).await;
                        }
                    }
                    Ok(ReviewDecisionOutcome::StartWorkItemDraft { feedback }) => {
                        if let Err(message) = spawn_provider_run_from_handler(
                            run_context.clone(),
                            ProviderRunKind::WorkItemPlanDraft { feedback },
                            outbound_tx.clone(),
                        )
                        .await
                        {
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx, &err).await;
                        }
                    }
                    Ok(ReviewDecisionOutcome::StartWorkItemBatch) => {
                        if let Err(message) = spawn_provider_run_from_handler(
                            run_context.clone(),
                            ProviderRunKind::WorkItemPlanBatch,
                            outbound_tx.clone(),
                        )
                        .await
                        {
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx, &err).await;
                        }
                    }
                    Ok(ReviewDecisionOutcome::StartRevision) => {
                        if let Err(message) = spawn_provider_run_from_handler(
                            run_context.clone(),
                            ProviderRunKind::WorkItemPlanRevision {
                                feedback: feedback_text,
                            },
                            outbound_tx.clone(),
                        )
                        .await
                        {
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx, &err).await;
                        }
                    }
                    Ok(ReviewDecisionOutcome::HumanConfirm)
                    | Ok(ReviewDecisionOutcome::ConfirmedWithChildSessions { .. }) => {}
                    Err(message) => {
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                }
            } else {
                // v38 复验 #2/#3（恢复 C3 前原意）：story/design 门上的反馈修订
                // 通道——L2 退役时随 HumanConfirm 消息族一并收口为协议错误，
                // 现按 workspace_type 分流恢复：story/design → 引擎进入 Revision
                // 阶段并发起修订 run（作者按反馈重写，完成后回门）；WorkItemPlan
                // plan-repair 链不受影响。非 story/design 的门外语义由引擎守卫
                // 拒绝（阶段矩阵已限定 author_confirm + 非 SC 流）。
                let result = {
                    let mut engine = engine.lock().await;
                    engine
                        .request_story_design_revision(feedback_text.clone())
                        .await
                };
                match result {
                    Ok(()) => {
                        if let Err(message) = spawn_provider_run_from_handler(
                            run_context.clone(),
                            ProviderRunKind::Revision,
                            outbound_tx.clone(),
                        )
                        .await
                        {
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx, &err).await;
                        }
                    }
                    Err(message) => {
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                }
            }
        }
        // 退役留档（T5/REQ-RET-02 L2）：HumanConfirm 消息族已删除（必删集）——
        // SC 门关门决策由 typed 三命令承载（Confirm/HumanGateFeedback/
        // AbandonHumanGate，T2 重承载），双轨期 legacy 桥接臂随之退役；wire 名
        // parse 面拒收（LEGACY_MESSAGE_RETIRED，REQ-RET-03 在途限制）。
        WsInMessage::ConfirmPlanAmendment { amendment_id } => {
            handle_plan_amendment_confirmation_from_handler(
                app_state.clone(),
                engine.clone(),
                run_context.manager.clone(),
                outbound_tx.clone(),
                amendment_id,
            )
            .await;
        }
        WsInMessage::CancelPlanAmendment {
            amendment_id,
            reason,
        } => {
            handle_plan_amendment_cancel_from_handler(
                engine.clone(),
                run_context.manager.clone(),
                outbound_tx.clone(),
                amendment_id,
                reason,
            )
            .await;
        }
        WsInMessage::StartLinkedWorkspaceAmendment { target } => {
            let target_for_context = target.clone();
            let result = {
                let engine = engine.lock().await;
                engine.start_linked_workspace_amendment(target)
            };
            let message = match result {
                Ok(snapshot) => WsOutMessage::LinkedWorkspaceAmendmentCreated { snapshot },
                Err(error) => WsOutMessage::ProtocolError {
                    code: "LINKED_WORKSPACE_AMENDMENT_INVALID".to_string(),
                    message: format!("{error:?}"),
                    context: serde_json::to_value(target_for_context).ok(),
                },
            };
            let _ = send_json_outbound(&outbound_tx, &message).await;
        } // 退役留档（T5/REQ-RET-02）：RevertWorkItem 消息族已删除（SC 无 revert
          // 消费，grep 实测裁定见 wp5-attribution-table.md §2）——wire 名 parse
          // 面拒收，路由分支退役。
    }
}

async fn refresh_workspace_context_for_session(
    run_context: &ProviderRunContext,
    engine: Arc<Mutex<WorkspaceEngine>>,
) -> Result<ProviderRunContext, String> {
    let lifecycle = LifecycleStore::new(run_context.app_paths.clone());
    let session_record = lifecycle
        .get_workspace_session(&run_context.session_id)
        .map_err(|error| format!("reload workspace session after provider lock failed: {error}"))?;
    let session_record =
        ensure_workspace_context_message(&run_context.app_paths, &lifecycle, session_record)
            .await
            .map_err(|error| {
                format!("refresh workspace context after provider lock failed: {error}")
            })?;

    {
        let mut engine = engine.lock().await;
        engine
            .session
            .replace_messages_from_records(session_record.messages.clone());
    }

    let mut refreshed = run_context.clone();
    refreshed.session_record = session_record;
    Ok(refreshed)
}

/// P0 1.3：claim 门面错误 → 既有 ProtocolError 码表（REQ-WIGA-05：
/// Conflict/Expired 显式映射，不静默吞）。
fn choice_claim_error(
    id: &str,
    error: &crate::web::workspace_session::ChoiceReplyError,
) -> WsOutMessage {
    use crate::web::workspace_session::ChoiceReplyError;
    let (code, message) = match error {
        ChoiceReplyError::Unknown => (
            "CHOICE_ID_UNMATCHED",
            "choice response has no matching pending choice on the current run",
        ),
        ChoiceReplyError::Conflict => (
            "CHOICE_CLAIM_CONFLICT",
            "choice response conflicts with an in-flight command for this choice",
        ),
        ChoiceReplyError::Expired => (
            "CHOICE_RUN_EXPIRED",
            "choice response targets a finished run and cannot be replayed",
        ),
    };
    WsOutMessage::ProtocolError {
        code: code.to_string(),
        message: format!("{message} (id={id})"),
        context: Some(serde_json::json!({ "id": id })),
    }
}

/// B1b（Task 4.2，设计 §5.1）：显式 fresh 的权威 CAS claim（A2a
/// `claim_lc_fresh` 固定判定顺序，Task 2.5 合同）与 typed 结果发送。
/// `Claimed` → Accepted/Reserved 帧；同 ID `Replay`（socket 门与 claim 间
/// 的竞态窗口同形收敛）→ Replayed 帧；`Busy`/`Stale` → typed
/// ProtocolError（码表与 socket 门共用）；store 写失败整体 Err（2.5：不残
/// 留半受理状态）→ durable 不可用稳定码。全程零 provider 启动——claim 后
/// 的 fresh native run 归 B2 tag dispatch。
async fn handle_lc_resume_recovery_fresh(
    run_context: ProviderRunContext,
    outbound_tx: mpsc::Sender<OutboundControl>,
    intent: crate::web::workspace_ws_types::LcResumeRecoveryIntent,
) {
    use crate::product::lifecycle_store::LcFreshClaimOutcome;
    use crate::web::workspace_ws_handler::socket::{
        LC_RESUME_RECOVERY_BUSY_CODE, LC_RESUME_STORE_UNAVAILABLE_CODE,
    };
    use crate::web::workspace_ws_types::LcResumeRecoveryDelivery;

    let store = LifecycleStore::new(run_context.app_paths.clone());
    let frame = match store.claim_lc_fresh(
        &run_context.session_id,
        &intent.recovery_id,
        &intent.operation_id,
    ) {
        Ok(LcFreshClaimOutcome::Claimed { operation, .. }) => {
            WsOutMessage::LcResumeRecoveryOperationResult {
                recovery_id: intent.recovery_id.clone(),
                operation_id: operation.operation_id.clone(),
                delivery: LcResumeRecoveryDelivery::Accepted,
                operation_state: operation.state,
                fresh_node_id: operation.fresh_node_id.clone(),
                reason_code: operation.reason_code.clone(),
            }
        }
        Ok(LcFreshClaimOutcome::Replay { operation, .. }) => {
            WsOutMessage::LcResumeRecoveryOperationResult {
                recovery_id: intent.recovery_id.clone(),
                operation_id: operation.operation_id.clone(),
                delivery: LcResumeRecoveryDelivery::Replayed,
                operation_state: operation.state,
                fresh_node_id: operation.fresh_node_id.clone(),
                reason_code: operation.reason_code.clone(),
            }
        }
        Ok(LcFreshClaimOutcome::Busy {
            active_operation_id,
        }) => WsOutMessage::ProtocolError {
            code: LC_RESUME_RECOVERY_BUSY_CODE.to_string(),
            message: format!(
                "lc resume recovery {} already holds an accepted fresh operation; a new operation id is rejected and running/unknown operations are never auto-resent",
                intent.recovery_id
            ),
            context: Some(serde_json::json!({ "active_operation_id": active_operation_id })),
        },
        Ok(LcFreshClaimOutcome::Stale { reason_code }) => WsOutMessage::ProtocolError {
            code: reason_code.clone(),
            message: format!(
                "lc resume fresh claim for operation {} rejected: {reason_code}",
                intent.operation_id
            ),
            context: Some(serde_json::json!({
                "recovery_id": intent.recovery_id,
                "operation_id": intent.operation_id,
            })),
        },
        Err(error) => WsOutMessage::ProtocolError {
            code: LC_RESUME_STORE_UNAVAILABLE_CODE.to_string(),
            message: format!(
                "durable projection unavailable for session {}: lc fresh claim store failure: {error}",
                run_context.session_id
            ),
            context: None,
        },
    };
    let _ = send_json_outbound(&outbound_tx, &frame).await;
}
