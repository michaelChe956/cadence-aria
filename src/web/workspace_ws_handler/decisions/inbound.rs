use super::*;

#[derive(Clone)]
pub(crate) struct WorkspaceInboundContext {
    pub(crate) engine: Arc<Mutex<WorkspaceEngine>>,
    pub(crate) run_context: ProviderRunContext,
    pub(crate) outbound_tx: mpsc::Sender<OutboundControl>,
    pub(crate) current_run: Arc<Mutex<Option<WorkspaceActiveRun>>>,
    pub(crate) workspace_runs: WorkspaceRunRegistry,
    pub(crate) session_id: String,
}

pub(crate) async fn handle_workspace_inbound_message(
    context: WorkspaceInboundContext,
    in_msg: WsInMessage,
) {
    let WorkspaceInboundContext {
        engine,
        run_context,
        outbound_tx,
        current_run,
        workspace_runs,
        session_id,
    } = context;

    match in_msg {
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
            abort_active_run(&current_run, &workspace_runs, &session_id).await;
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
            handle_human_confirm_from_handler(
                run_context.clone(),
                outbound_tx.clone(),
                HumanConfirmDecision::Confirm,
                None,
            )
            .await;
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
            let command_tx =
                active_run_command_tx(&current_run, &workspace_runs, &session_id).await;
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
            let active_run = active_run(&current_run, &workspace_runs, &session_id).await;
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
        WsInMessage::ReviewDecisionResponse {
            decision,
            extra_context,
        } => {
            handle_review_decision_from_handler(
                run_context.clone(),
                outbound_tx.clone(),
                decision,
                extra_context,
            )
            .await;
        }
        WsInMessage::AuthorDecision { decision } => {
            handle_author_decision_from_handler(run_context.clone(), outbound_tx.clone(), decision)
                .await;
        }
        WsInMessage::SelectWorkItemGenerationMode { mode } => {
            let selected_mode = mode.clone();
            let result = {
                let mut engine = engine.lock().await;
                engine.select_work_item_generation_mode(mode).await
            };
            if let Err(message) = result {
                let err = WsOutMessage::ProtocolError {
                    code: "WORK_ITEM_GENERATION_MODE_NODE_REQUIRED".to_string(),
                    message,
                    context: None,
                };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            } else {
                let run_kind = match selected_mode {
                    WorkItemGenerationModeDto::Serial => {
                        ProviderRunKind::WorkItemPlanDraft { feedback: None }
                    }
                    WorkItemGenerationModeDto::Batch => ProviderRunKind::WorkItemPlanBatch,
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
        }
        WsInMessage::RequestOutlineRevision { feedback } => {
            let result = {
                let mut engine = engine.lock().await;
                let revision_feedback =
                    engine.work_item_plan_outline_revision_feedback(feedback.as_deref());
                engine
                    .request_work_item_plan_outline_revision(feedback)
                    .await
                    .map(|_| revision_feedback)
            };
            match result {
                Ok(feedback) => {
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
                Err(message) => {
                    let err = WsOutMessage::ProtocolError {
                        code: "WORK_ITEM_GENERATION_MODE_NODE_REQUIRED".to_string(),
                        message,
                        context: None,
                    };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
            }
        }
        WsInMessage::WorkItemDraftDecision {
            outline_id,
            decision,
            feedback,
        } => {
            let result = {
                let mut engine = engine.lock().await;
                engine
                    .handle_work_item_draft_decision(outline_id, decision, feedback)
                    .await
            };
            match result {
                Ok(WorkItemDraftDecisionOutcome::StartDraftRun) => {
                    if let Err(message) = spawn_provider_run_from_handler(
                        run_context.clone(),
                        ProviderRunKind::WorkItemPlanDraft { feedback: None },
                        outbound_tx.clone(),
                    )
                    .await
                    {
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                }
                Ok(WorkItemDraftDecisionOutcome::StartReview) => {
                    if let Err(message) = spawn_provider_run_from_handler(
                        run_context.clone(),
                        ProviderRunKind::ReviewOnly,
                        outbound_tx.clone(),
                    )
                    .await
                    {
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                }
                Ok(WorkItemDraftDecisionOutcome::HumanConfirm) => {}
                Err(message) => {
                    let err = WsOutMessage::ProtocolError {
                        code: "WORK_ITEM_DRAFT_CONFIRM_REQUIRED".to_string(),
                        message,
                        context: None,
                    };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
            }
        }
        WsInMessage::WorkItemBatchDecision {
            decision,
            feedback,
            first_affected_outline_id,
        } => {
            let result = {
                let mut engine = engine.lock().await;
                engine
                    .handle_work_item_batch_decision(decision, feedback, first_affected_outline_id)
                    .await
            };
            match result {
                Ok(WorkItemBatchDecisionOutcome::StartBatchRun) => {
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
                Ok(WorkItemBatchDecisionOutcome::StartDraftRun) => {
                    if let Err(message) = spawn_provider_run_from_handler(
                        run_context.clone(),
                        ProviderRunKind::WorkItemPlanDraft { feedback: None },
                        outbound_tx.clone(),
                    )
                    .await
                    {
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                }
                Ok(WorkItemBatchDecisionOutcome::StartReview) => {
                    if let Err(message) = spawn_provider_run_from_handler(
                        run_context.clone(),
                        ProviderRunKind::ReviewOnly,
                        outbound_tx.clone(),
                    )
                    .await
                    {
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                }
                Ok(WorkItemBatchDecisionOutcome::HumanConfirm) => {}
                Err(message) => {
                    let err = WsOutMessage::ProtocolError {
                        code: "WORK_ITEM_BATCH_CONFIRM_REQUIRED".to_string(),
                        message,
                        context: None,
                    };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
            }
        }
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
        WsInMessage::Abort => {
            if abort_active_run(&current_run, &workspace_runs, &session_id).await {
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
        WsInMessage::Hello { .. } => {
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
                            ProviderRunKind::WorkItemPlanAuthor
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
        WsInMessage::SelectRevisionPath {
            path,
            extra_context,
        } => {
            let (decision, extra_context) = map_revision_path(path, extra_context);
            handle_review_decision_from_handler(
                run_context.clone(),
                outbound_tx.clone(),
                decision,
                extra_context,
            )
            .await;
        }
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
                        if let Err(message) = spawn_provider_run_from_handler(
                            run_context.clone(),
                            ProviderRunKind::WorkItemPlanAuthor,
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
                let payload = serde_json::to_value(feedback).ok();
                handle_human_confirm_from_handler(
                    run_context.clone(),
                    outbound_tx.clone(),
                    HumanConfirmDecision::RequestChange,
                    payload,
                )
                .await;
            }
        }
        WsInMessage::HumanConfirm { decision, payload } => {
            handle_human_confirm_from_handler(
                run_context.clone(),
                outbound_tx.clone(),
                decision,
                payload,
            )
            .await;
        }
        WsInMessage::RevertWorkItem {
            work_item_id,
            feedback,
            clear,
        } => {
            let result = {
                let mut engine = engine.lock().await;
                engine
                    .apply_revert_mark(&work_item_id, feedback, clear)
                    .await
            };
            if let Err(message) = result {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
            // 成功时 apply_revert_mark 已发 EngineEvent::ArtifactUpdate，event forwarder 会推前端
        }
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
