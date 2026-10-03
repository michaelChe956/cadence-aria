use super::*;
use crate::product::workspace_engine::PlanAuthorOutputContract;

#[path = "provider_run/entry.rs"]
mod entry;
pub(crate) use entry::{
    ProviderRunStartMode, spawn_provider_run_claiming_idle, spawn_provider_run_from_event,
    spawn_provider_run_from_handler,
};

pub(super) async fn spawn_provider_run_with_start_mode(
    run_context: ProviderRunContext,
    run_kind: ProviderRunKind,
    outbound_tx: mpsc::Sender<OutboundControl>,
    start_mode: ProviderRunStartMode,
) -> Result<bool, String> {
    let run_context_clone = run_context.clone();
    let ProviderRunContext {
        provider_registry,
        manager,
        engine,
        workspace_runs,
        session_id,
        connection_id,
        app_paths: _,
        session_record: _,
    } = run_context;

    // Handler-originated starts always supersede the active provider run: a user
    // message must cancel a streaming run before waiting for the engine mutex
    // (held by the stream owner while driving its provider session). Engine-
    // originated relays are filtered by `spawn_provider_run_from_event`: same-
    // node duplicates drained; ReviewOnly relays yield to an in-flight run whose
    // followups own the review continuation; repair relays (WorkItemPlan kinds,
    // emitted by a run that delegated via policy routing) keep the supersede
    // hand-off. P1 WIGA Task 5：auto 认领（ClaimIfIdle）不走本段——supersede
    // 只属于显式 human/driver 重跑。
    if matches!(start_mode, ProviderRunStartMode::SupersedeFromAttachment) {
        // 诊断打点（claude×轻 握手谜团第 2 轮，不改行为）：新 run 接替取消旧 run
        // 的唯一裁决点迁入 manager，令所有连接共享同一临界区。
        eprintln!(
            "[aria-cancellation] workspace handler_run_supersede trigger=handler_run_supersede session_id={} kind={:?}",
            session_id, run_kind
        );

        // socket 发起的 supersede 在进入 engine 锁前原子核验 attachment epoch；已被
        // 接管的 stale driver 会在触碰 active run 前返回 STALE_DRIVER_LEASE。
        manager
            .abort_active_run_from_attachment(connection_id.as_deref())
            .await?;
    }
    let provider_name = {
        let engine = engine.lock().await;
        match &run_kind {
            ProviderRunKind::Author { .. }
            | ProviderRunKind::AuthorChoiceFollowup { .. }
            | ProviderRunKind::Revision => engine.session().author_provider.clone(),
            ProviderRunKind::ReviewOnly => {
                // C2 Task 5（REQ-CRO-05）：reviewer 缺失（空 effective）不启动
                // ReviewOnly run——绝不以 Codex 顶替。
                let reviewer = engine.session().reviewer_provider.clone();
                match reviewer {
                    Some(reviewer) => reviewer,
                    None => {
                        return Err(
                            "reviewer_configuration_missing: review run not started".to_string()
                        );
                    }
                }
            }
            ProviderRunKind::WorkItemPlanLegacyAuthor
            | ProviderRunKind::WorkItemPlanSingleCandidateAuthor
            | ProviderRunKind::WorkItemPlanOutlineRevision { .. }
            | ProviderRunKind::WorkItemPlanOutlineRebuild { .. }
            | ProviderRunKind::WorkItemPlanDraft { .. }
            | ProviderRunKind::WorkItemPlanBatch
            | ProviderRunKind::WorkItemPlanRevision { .. }
            | ProviderRunKind::HumanGateScManualRevision { .. } => {
                engine.session().author_provider.clone()
            }
        }
    };
    let provider_for_run = {
        let Some(provider) = provider_registry.get(&provider_name) else {
            return Err(format!("provider unavailable: {provider_name:?}"));
        };
        provider
    };

    let target_node_id = {
        let engine = engine.lock().await;
        engine.active_timeline_node_id()
    };
    let registered = match start_mode {
        ProviderRunStartMode::SupersedeFromAttachment => manager
            .start_run_from_attachment(connection_id.as_deref(), run_kind.clone(), target_node_id)
            .await
            .map(Some)?,
        ProviderRunStartMode::ClaimIfIdle => {
            manager
                .try_start_run_if_idle(run_kind.clone(), target_node_id)
                .await?
        }
    };
    let Some((run_id, run_token, run_cancel, command_rx, _node_id)) = registered else {
        // 认领窗口内活 run 已被他人启动：不 supersede，交由其持有者驱动。
        return Ok(false);
    };
    let run_label = format!("run-{run_id}");
    // provider drive 期标记（idle 关闭守卫扩展）：从 run 任务启动到结束，该 session
    // 的 idle 守卫都不主动关连接；run 所有权已由 manager 跨 socket 保存。
    let provider_drive_guard = workspace_runs.begin_provider_drive(&session_id);

    {
        let mut engine = engine.lock().await;
        engine.mark_active_run_started(run_label.clone());
    }

    let engine_for_run = engine.clone();
    let manager_for_task = manager.clone();
    let provider_registry_for_run = provider_registry.clone();
    let outbound_tx_for_task = outbound_tx.clone();
    let outline_revision_feedback = match &run_kind {
        ProviderRunKind::WorkItemPlanOutlineRevision { feedback } => feedback.clone(),
        _ => None,
    };
    let is_outline_revision_run = matches!(
        &run_kind,
        ProviderRunKind::WorkItemPlanOutlineRevision { .. }
    );
    tokio::spawn(async move {
        let mut provider_drive_guard = Some(provider_drive_guard);
        let mut engine = engine_for_run.lock().await;
        engine.use_run_token(run_cancel.clone());
        // B3：StaleContext 重建携带的 rebuilt planning context（provider run 构造新会话
        // 使用：cwd/inventory/policy digest）。在 `match run_kind` 前提取，供共享 arm 使用。
        let rebuild_context = match &run_kind {
            ProviderRunKind::WorkItemPlanOutlineRebuild { rebuilt } => Some((**rebuilt).clone()),
            _ => None,
        };
        match run_kind {
            ProviderRunKind::Author { content } => {
                engine
                    .handle_user_message_from_run(
                        content,
                        provider_for_run.clone(),
                        command_rx,
                        &run_context_clone.session_record,
                    )
                    .await;
            }
            ProviderRunKind::AuthorChoiceFollowup { content } => {
                engine
                    .handle_author_choice_followup_from_run(
                        content,
                        provider_for_run.clone(),
                        command_rx,
                        &run_context_clone.session_record,
                    )
                    .await;
            }
            ProviderRunKind::Revision => {
                engine
                    .drive_revision_session(provider_for_run.clone(), command_rx)
                    .await;
            }
            ProviderRunKind::ReviewOnly => {
                if engine.logical_provider_gateway().is_some() {
                    engine.drive_review_session_via_gateway(command_rx).await;
                } else {
                    engine
                        .drive_review_session(provider_for_run.clone(), command_rx)
                        .await;
                }
            }
            ProviderRunKind::WorkItemPlanLegacyAuthor
            | ProviderRunKind::WorkItemPlanOutlineRevision { .. }
            | ProviderRunKind::WorkItemPlanOutlineRebuild { .. } => {
                include!("provider_run/work_item_plan_legacy_author.inc.rs")
            }
            ProviderRunKind::WorkItemPlanSingleCandidateAuthor => {
                let mut command_rx = command_rx;
                match single_candidate::run_single_candidate_author(
                    &mut engine,
                    provider_for_run.clone(),
                    run_cancel.clone(),
                    &mut command_rx,
                    &run_context_clone,
                )
                .await
                {
                    Ok(single_candidate::SingleCandidateProviderRunOutcome::Completed) => {}
                    Err(single_candidate::SingleCandidateProviderRunError::AlreadyFinished) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        drop(provider_drive_guard.take());
                        manager_for_task.finish_run(run_token).await;
                        return;
                    }
                    Err(single_candidate::SingleCandidateProviderRunError::Superseded) => {
                        // 败者让位：键的持有者（健康在途 run 或已完结会话）继续
                        // 拥有本次启动。迟到 run 静默退场——不落 failed 节点、
                        // 不广播 Error、不改写 durable phase（k3 P2）。
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        drop(provider_drive_guard.take());
                        manager_for_task.finish_run(run_token).await;
                        return;
                    }
                    Err(single_candidate::SingleCandidateProviderRunError::Message(message)) => {
                        engine
                            .finish_active_run_with_failed_node(message.clone())
                            .await;
                        drop(engine);
                        // 缺陷 #10：终态失败路径必须释放 run 注册——对齐同族
                        // AlreadyFinished/Superseded/AdmissionWaiting 分支的既有
                        // finish_run 惯例。修复前提前 return 跳过释放——provider
                        // 已死但 active_run 永驻，is_active_run() 误报
                        // sc_recovery_busy，显式恢复面被僵死注册阻塞
                        //（E2E v1.1 §3.3，曾需人工 WS Abort 清除）。
                        drop(provider_drive_guard.take());
                        manager_for_task.finish_run(run_token).await;
                        let _ = send_json_outbound(
                            &outbound_tx_for_task,
                            &WsOutMessage::Error { message },
                        )
                        .await;
                        return;
                    }
                    Err(single_candidate::SingleCandidateProviderRunError::AdmissionWaiting {
                        reason_code,
                        detail,
                        missing_materials,
                        allowed_actions,
                    }) => {
                        // C-1：LC admission waiting——durable phase 已由运行内
                        // 回落 Prepare 面，此处以无 failed 节点收尾（waiting 不是
                        // 失败），并把缺失材料与允许动作作为可操作指引上浮。
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        drop(provider_drive_guard.take());
                        manager_for_task.finish_run(run_token).await;
                        let message = format_single_candidate_admission_waiting(
                            &reason_code,
                            &detail,
                            &missing_materials,
                            &allowed_actions,
                        );
                        let _ = send_json_outbound(
                            &outbound_tx_for_task,
                            &WsOutMessage::Error { message },
                        )
                        .await;
                        return;
                    }
                }
            }
            ProviderRunKind::WorkItemPlanDraft { feedback } => {
                let mut command_rx = command_rx;
                let mut feedback = feedback.or_else(|| engine.pending_revision_context.clone());
                while engine.active_node_type()
                    == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemDraftRun)
                {
                    let Some(node_id) = engine.active_timeline_node_id() else {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: "work item draft run node unavailable".to_string(),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    };
                    let plan_launch = match resolve_plan_author_launch(&engine, None, None) {
                        Ok(launch) => launch,
                        Err(error) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error {
                                message: format!("logical plan launch failed: {error}"),
                            };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    };
                    let routing_context = plan_launch.routing_context();
                    let provider_input = match engine.build_current_work_item_draft_streaming_input(
                        feedback.as_deref(),
                        &routing_context,
                    ) {
                        Ok(input) => input,
                        Err(message) => {
                            engine
                                .finish_active_run_with_failed_node(message.clone())
                                .await;
                            drop(engine);
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    };
                    let author_provider = engine.session().author_provider.clone();
                    engine
                        .emit_provider_prompt_event(
                            &node_id,
                            provider_input.prompt.clone(),
                            if feedback.is_some() {
                                "发送给 WorkItemDraft provider 的增量返修提示词"
                            } else {
                                "发送给 WorkItemDraft provider 的完整提示词"
                            },
                            Some(author_provider.clone()),
                        )
                        .await;
                    let provider_input = engine.attach_tool_policy_audit(provider_input);
                    let provider_session = start_work_item_plan_author(
                        plan_launch,
                        provider_for_run.clone(),
                        provider_input,
                        run_cancel.clone(),
                        None,
                    )
                    .await;
                    let full_output = match engine
                        .drive_work_item_plan_provider_session_to_output(
                            provider_session,
                            &mut command_rx,
                            node_id,
                            author_provider,
                        )
                        .await
                    {
                        Ok(output) => output,
                        Err(_) => {
                            engine.mark_active_run_finished(&run_label);
                            return;
                        }
                    };
                    let structured_output =
                        match parse_work_item_split_structured_output(&full_output) {
                            Ok(output) => output,
                            Err(message) => {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error {
                                    message: format!("work item draft generate failed: {message}"),
                                };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }
                        };
                    let candidate = match parse_work_item_draft_output(structured_output) {
                        Ok(candidate) => candidate,
                        Err(error) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error {
                                message: format!("work item draft parse failed: {}", error.message),
                            };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    };
                    match engine
                        .complete_work_item_draft_author(candidate, feedback.as_deref())
                        .await
                    {
                        Ok(WorkItemDraftAuthorOutcome::RetryOnce {
                            feedback: repair_feedback,
                            ..
                        }) => feedback = Some(repair_feedback),
                        Ok(WorkItemDraftAuthorOutcome::AwaitConfirmation) => break,
                        Err(message) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    }
                }
            }
            ProviderRunKind::WorkItemPlanBatch => {
                let mut command_rx = command_rx;
                let mut feedback = engine.pending_revision_context.clone();
                while engine.active_node_type()
                    == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemBatchRun)
                {
                    let Some(node_id) = engine.active_timeline_node_id() else {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: "work item batch run node unavailable".to_string(),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    };
                    let plan_launch = match resolve_plan_author_launch(&engine, None, None) {
                        Ok(launch) => launch,
                        Err(error) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error {
                                message: format!("logical plan launch failed: {error}"),
                            };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    };
                    let routing_context = plan_launch.routing_context();
                    let provider_input = match engine
                        .build_current_work_item_batch_draft_streaming_input(
                            feedback.as_deref(),
                            &routing_context,
                        ) {
                        Ok(input) => input,
                        Err(message) => {
                            engine
                                .finish_active_run_with_failed_node(message.clone())
                                .await;
                            drop(engine);
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    };
                    let author_provider = engine.session().author_provider.clone();
                    engine
                        .emit_provider_prompt_event(
                            &node_id,
                            provider_input.prompt.clone(),
                            if feedback.is_some() {
                                "发送给 WorkItemBatch provider 的本地校验修复提示词"
                            } else {
                                "发送给 WorkItemBatch provider 的完整提示词"
                            },
                            Some(author_provider.clone()),
                        )
                        .await;
                    let provider_input = engine.attach_tool_policy_audit(provider_input);
                    let provider_session = start_work_item_plan_author(
                        plan_launch,
                        provider_for_run.clone(),
                        provider_input,
                        run_cancel.clone(),
                        None,
                    )
                    .await;
                    let full_output = match engine
                        .drive_work_item_plan_provider_session_to_output(
                            provider_session,
                            &mut command_rx,
                            node_id,
                            author_provider,
                        )
                        .await
                    {
                        Ok(output) => output,
                        Err(_) => {
                            engine.mark_active_run_finished(&run_label);
                            return;
                        }
                    };
                    let structured_output =
                        match parse_work_item_split_structured_output(&full_output) {
                            Ok(output) => output,
                            Err(message) => {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error {
                                    message: format!(
                                        "work item batch draft generate failed: {message}"
                                    ),
                                };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }
                        };
                    let candidate = match parse_work_item_draft_output(structured_output) {
                        Ok(candidate) => candidate,
                        Err(error) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error {
                                message: format!(
                                    "work item batch draft parse failed: {}",
                                    error.message
                                ),
                            };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    };
                    match engine
                        .complete_work_item_batch_draft_author(candidate, feedback.as_deref())
                        .await
                    {
                        Ok(WorkItemDraftAuthorOutcome::RetryOnce {
                            feedback: repair_feedback,
                            ..
                        }) => feedback = Some(repair_feedback),
                        Ok(WorkItemDraftAuthorOutcome::AwaitConfirmation) => feedback = None,
                        Err(message) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    }
                }
            }
            ProviderRunKind::WorkItemPlanRevision { feedback } => {
                workspace_ws_work_item_plan_revision_arm!(
                    engine,
                    run_context_clone,
                    provider_for_run,
                    run_cancel,
                    command_rx,
                    run_label,
                    outbound_tx_for_task,
                    manager_for_task,
                    run_token,
                    provider_drive_guard,
                    feedback
                );
            }
            ProviderRunKind::HumanGateScManualRevision { turn_id, prompt } => {
                use crate::product::models::HumanGateTurnFailureClass;
                let mut command_rx = command_rx;
                let prompt = if prompt.is_empty() {
                    let turn = match LifecycleStore::new(run_context_clone.app_paths.clone())
                        .get_human_gate_turn(&engine.session().session_id, &turn_id)
                    {
                        Ok(turn) => turn,
                        Err(error) => {
                            let message = format!("load human gate turn failed: {error}");
                            let _ = engine
                                .fail_human_gate_turn(
                                    &turn_id,
                                    HumanGateTurnFailureClass::ProviderErr,
                                )
                                .await;
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let _ = send_json_outbound(
                                &outbound_tx_for_task,
                                &WsOutMessage::HumanGateTurnFailed {
                                    turn_id,
                                    failure_class: "provider_err".to_string(),
                                    message,
                                },
                            )
                            .await;
                            return;
                        }
                    };
                    match engine.build_sc_manual_revision_prompt_for_turn(&turn.feedback_text) {
                        Ok(prompt) => prompt,
                        Err(message) => {
                            let _ = engine
                                .fail_human_gate_turn(
                                    &turn_id,
                                    HumanGateTurnFailureClass::ProviderErr,
                                )
                                .await;
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let _ = send_json_outbound(
                                &outbound_tx_for_task,
                                &WsOutMessage::HumanGateTurnFailed {
                                    turn_id,
                                    failure_class: "provider_err".to_string(),
                                    message,
                                },
                            )
                            .await;
                            return;
                        }
                    }
                } else {
                    prompt
                };

                // Durable state must prove Running before the provider is started. This is
                // deliberately inside the spawned task, after the handler has cancelled any
                // previous run, so a crash cannot leave Reserved while a provider is active.
                if let Err(message) = engine.mark_human_gate_turn_running(&turn_id) {
                    let _ = engine
                        .fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
                        .await;
                    engine.mark_active_run_finished(&run_label);
                    drop(engine);
                    let _ = send_json_outbound(
                        &outbound_tx_for_task,
                        &WsOutMessage::HumanGateTurnFailed {
                            turn_id,
                            failure_class: "provider_err".to_string(),
                            message,
                        },
                    )
                    .await;
                    return;
                }

                // F-49/A6：门修订轮必须与普通 SC 修订同构地落在 author 节点上。
                // 此前取 `active_timeline_node_id()`——门内活动节点就是 HumanConfirm
                // 门节点，修订 prompt/输出流/artifact_ref 与产物版本 source_node_id
                // 全部写进门节点 detail；门节点在对话流 rebuild 判 role=null，整节点
                // 零条目，用户侧「author 修订步不可见」。选节点规则（复用活动
                // AuthorRun / 新建）见 `begin_work_item_plan_human_gate_revision_run`。
                let node_id = engine.begin_work_item_plan_human_gate_revision_run().await;
                let author_provider = engine.session().author_provider.clone();
                engine
                    .emit_provider_prompt_event(
                        &node_id,
                        prompt.clone(),
                        "发送给 SC human-gate revision provider 的完整修订提示词",
                        Some(author_provider.clone()),
                    )
                    .await;
                let worktree_path = engine
                    .session()
                    .repository_path
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                let provider_input = match engine.build_work_item_plan_streaming_input(
                    crate::product::work_item_split_engine::types::provider_name_to_type(
                        &author_provider,
                    ),
                    prompt.clone(),
                    worktree_path.to_string_lossy().to_string(),
                    author_provider.clone(),
                    PlanAuthorOutputContract::Structured,
                ) {
                    Ok(provider_input) => provider_input,
                    Err(message) => {
                        // REQ-PIB-02（T2.3）：基线不可解析 → 门内轮次失败（可观测）。
                        let _ = engine
                            .fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
                            .await;
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let _ = send_json_outbound(
                            &outbound_tx_for_task,
                            &WsOutMessage::HumanGateTurnFailed {
                                turn_id,
                                failure_class: "provider_err".to_string(),
                                message,
                            },
                        )
                        .await;
                        return;
                    }
                };
                let launch = match resolve_plan_author_launch(&engine, None, None) {
                    Ok(launch) => launch,
                    Err(error) => {
                        let message = error.details.clone();
                        let _ = engine
                            .fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
                            .await;
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let _ = send_json_outbound(
                            &outbound_tx_for_task,
                            &WsOutMessage::HumanGateTurnFailed {
                                turn_id,
                                failure_class: "provider_err".to_string(),
                                message,
                            },
                        )
                        .await;
                        return;
                    }
                };
                let provider_input = engine.attach_tool_policy_audit(provider_input);
                let provider_session = start_work_item_plan_author(
                    launch,
                    provider_for_run.clone(),
                    provider_input,
                    run_cancel.clone(),
                    None,
                )
                .await;
                let full_output = match engine
                    .drive_work_item_plan_provider_session_to_output(
                        provider_session,
                        &mut command_rx,
                        node_id,
                        author_provider,
                    )
                    .await
                {
                    Ok(output) => output,
                    Err(message) => {
                        let _ = engine
                            .fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
                            .await;
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let _ = send_json_outbound(
                            &outbound_tx_for_task,
                            &WsOutMessage::HumanGateTurnFailed {
                                turn_id,
                                failure_class: "provider_err".to_string(),
                                message,
                            },
                        )
                        .await;
                        return;
                    }
                };
                match engine
                    .run_sc_manual_revision_turn(&turn_id, full_output)
                    .await
                {
                    Ok(crate::product::workspace_engine::ScManualRevisionResult::Accepted { artifact_ref }) => {
                        let _ = send_json_outbound(
                            &outbound_tx_for_task,
                            &WsOutMessage::HumanGateTurnCompleted { turn_id, artifact_ref },
                        ).await;
                    }
                    Ok(crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected { diagnostics }) => {
                        let message = diagnostics.join("; ");
                        let _ = send_json_outbound(
                            &outbound_tx_for_task,
                            &WsOutMessage::HumanGateTurnFailed {
                                turn_id,
                                failure_class: "validation_reject".to_string(),
                                message,
                            },
                        ).await;
                    }
                    Err(message) => {
                        let _ = engine.fail_human_gate_turn(
                            &turn_id,
                            HumanGateTurnFailureClass::ProviderErr,
                        ).await;
                        let _ = send_json_outbound(
                            &outbound_tx_for_task,
                            &WsOutMessage::HumanGateTurnFailed {
                                turn_id,
                                failure_class: "provider_err".to_string(),
                                message,
                            },
                        ).await;
                    }
                }
            }
        }
        workspace_ws_provider_run_followups!(
            engine,
            provider_registry_for_run,
            manager_for_task,
            run_token,
            run_label,
            outbound_tx_for_task,
            run_cancel,
            run_context_clone,
            provider_drive_guard
        );
        engine.mark_active_run_finished(&run_label);
        drop(engine);
        drop(provider_drive_guard.take());
        manager_for_task.finish_run(run_token).await;
    });

    Ok(true)
}

include!("provider_run/single_candidate_admission_waiting.inc.rs");
