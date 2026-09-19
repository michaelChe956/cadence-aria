use super::*;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
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
        && (requires_stage_validation(&envelope.message)
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
        } => {
            if let Err(err) = validate_command_id(&command_id) {
                let _ = send_json_outbound(&outbound_tx, &err).await;
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
            handle_advance_from_handler(run_context.clone(), outbound_tx.clone(), command_id).await;
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
                let state_msg = engine.build_session_state();
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
                // HumanConfirmDecision::Confirm 桥随消息族删除）。
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
                let state_msg = {
                    let engine = engine.lock().await;
                    engine.build_session_state()
                };
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
        } => {
            tracing::info!(choice_id = %id, "ws inbound choice response");
            eprintln!(
                "[aria-choice-diag] ws inbound choice_response session={} id={} selected={:?} free_text_present={}",
                session_id,
                id,
                selected_option_ids,
                free_text
                    .as_ref()
                    .is_some_and(|text| !text.trim().is_empty())
            );
            let active_run = run_context.manager.active_run().await;
            if let Some(run) = active_run {
                let mut pending_choice_ids = run.pending_choice_ids.lock().await;
                if !pending_choice_ids.remove(&id) {
                    let _ = send_json_outbound(&outbound_tx, &choice_id_unmatched_error(&id)).await;
                    return;
                }
                drop(pending_choice_ids);

                eprintln!(
                    "[aria-choice-diag] ws forwarding choice_response to active run session={} id={}",
                    session_id, id
                );
                if run
                    .command_tx
                    .send(ProviderCommand::ChoiceResponse {
                        id: id.clone(),
                        selected_option_ids: selected_option_ids.clone(),
                        free_text: free_text.clone(),
                        answers: answers
                            .clone()
                            .into_iter()
                            .map(|answer| {
                                crate::cross_cutting::streaming_provider::ChoiceAnswerData {
                                    question_id: answer.question_id,
                                    selected_option_ids: answer.selected_option_ids,
                                    free_text: answer.free_text,
                                }
                            })
                            .collect(),
                    })
                    .await
                    .is_ok()
                {
                    eprintln!(
                        "[aria-choice-diag] ws forwarded choice_response to active run session={} id={}",
                        session_id, id
                    );
                    return;
                }
                eprintln!(
                    "[aria-choice-diag] ws failed to forward choice_response to active run session={} id={}; falling back",
                    session_id, id
                );
            } else {
                eprintln!(
                    "[aria-choice-diag] ws has no active run for choice_response session={} id={}; trying text fallback follow-up",
                    session_id, id
                );
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
        // 退役留档（T5/REQ-RET-02）：legacy 决策路由分支已删除——ReviewDecision
        // Response/AuthorDecision/SelectWorkItemGenerationMode/RequestOutline
        // Revision/WorkItemDraftDecision/WorkItemBatchDecision/SaveHumanPresentation
        // Revision 消息族 wire 名在 parse 面即被拒收（LEGACY_MESSAGE_RETIRED），
        // 消费链与处置见 wp5-attribution-table.md。
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
            }
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
                tokio::spawn(async move {
                    let state_msg = {
                        let engine = engine_for_hello.lock().await;
                        engine.build_session_state()
                    };
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
        } => {
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
                // L2 退役收口：非 WorkItemPlan 会话的 RequestChange 桥接随
                // HumanConfirm 消息族删除（wp5-attribution-table.md §2
                // RequestRevision 行）——保留通道仅服务 WorkItemPlan plan-repair。
                let err = WsOutMessage::ProtocolError {
                    code: "REQUEST_REVISION_WORKSPACE_INVALID".to_string(),
                    message: "request_revision is only supported for work item plan workspaces"
                        .to_string(),
                    context: None,
                };
                let _ = send_json_outbound(&outbound_tx, &err).await;
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
        }
        // 退役留档（T5/REQ-RET-02）：RevertWorkItem 消息族已删除（SC 无 revert
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
