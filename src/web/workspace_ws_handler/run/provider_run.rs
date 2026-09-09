use super::*;

pub(crate) async fn spawn_provider_run_from_event(
    run_context: ProviderRunContext,
    run_kind: ProviderRunKind,
    requested_node_id: Option<String>,
    outbound_tx: mpsc::Sender<OutboundControl>,
) -> Result<(), String> {
    if let Some(active_run) = run_context
        .workspace_runs
        .run(&run_context.session_id)
        .await
    {
        if active_run.node_id == requested_node_id {
            tracing::debug!(
                session_id = %run_context.session_id,
                node_id = ?requested_node_id,
                "drained duplicate provider run request"
            );
            return Ok(());
        }
        // ReviewOnly 接力让位（claude×轻 rep1v11 session_0255 双 spawn 误杀终局）：
        // 两处 ReviewOnly 发射点（product/workspace_engine/single_candidate.rs 的
        // SC compile 完成路径与 conversational_gate.rs 的 SC 人工修订 turn 路径）
        // 均由仍在途的活跃 run 发出，且该 run 的任务体会经 followups 宏内联驱动
        // CrossReview 评审（`while stage == CrossReview`）——接力事件与在途 run 的
        // 自续驱动是同一逻辑 review 启动的两条执行路径。接力请求的 node_id
        // （ReviewerRun 节点）与在途 run 注册时的 node_id（author 节点）必然不同，
        // 同节点去重不命中；若放行到 from_handler，其无条件 supersede 会取消在途
        // run 的 token，杀死正在握手的 reviewer 会话（handshake failed: cancelled）
        // 并双驱动 review。故会话有活跃 run 在途时 drain 接力，让位自续。
        // 返修接力（WorkItemPlan* kind，由已退场/让位的 run 经策略路由委托）与
        // 用户显式路径（直调 from_handler）不在此收窄范围内，supersede 语义不变。
        if matches!(run_kind, ProviderRunKind::ReviewOnly) {
            tracing::debug!(
                session_id = %run_context.session_id,
                node_id = ?requested_node_id,
                "drained review relay; in-flight run owns review continuation"
            );
            return Ok(());
        }
    }

    spawn_provider_run_from_handler(run_context, run_kind, outbound_tx).await
}

pub(crate) async fn spawn_provider_run_from_handler(
    run_context: ProviderRunContext,
    run_kind: ProviderRunKind,
    outbound_tx: mpsc::Sender<OutboundControl>,
) -> Result<(), String> {
    let run_context_clone = run_context.clone();
    let ProviderRunContext {
        provider_registry,
        engine,
        current_run,
        workspace_runs,
        session_id,
        next_run_id,
        app_paths: _,
        session_record: _,
    } = run_context;

    // Handler-originated starts always supersede the active provider run. In
    // particular, a user message must cancel a streaming run before waiting
    // for the engine mutex, which the stream owner holds while driving its
    // provider session. Engine-originated relays are filtered by
    // `spawn_provider_run_from_event` before reaching this handler: same-node
    // duplicates are drained, and ReviewOnly relays yield to an in-flight run
    // whose followups own the review continuation. Repair relays (WorkItemPlan
    // kinds, emitted by a run that delegated via policy routing) intentionally
    // keep the supersede hand-off below.
    //
    // 诊断打点（claude×轻 握手谜团第 2 轮，不改行为）：新 run 接替取消旧 run
    // （workitem 重试/新阶段启动时若旧 runner 仍在注册表，其 token 在此被取消——
    // workspace 版 H1 候选：接替误杀在途握手）。
    eprintln!(
        "[aria-cancellation] workspace handler_run_supersede trigger=handler_run_supersede session_id={}",
        session_id
    );
    abort_active_run(&current_run, &workspace_runs, &session_id).await;

    let target_node_id = {
        let engine = engine.lock().await;
        engine.active_timeline_node_id()
    };

    let provider_name = {
        let engine = engine.lock().await;
        match &run_kind {
            ProviderRunKind::Author { .. }
            | ProviderRunKind::AuthorChoiceFollowup { .. }
            | ProviderRunKind::Revision => engine.session().author_provider.clone(),
            ProviderRunKind::ReviewOnly => engine
                .session()
                .reviewer_provider
                .clone()
                .unwrap_or(ProviderName::Codex),
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

    let run_id = {
        let mut next = next_run_id.lock().await;
        *next += 1;
        *next
    };
    let run_label = format!("run-{run_id}");
    let run_token = NEXT_ACTIVE_RUN_TOKEN.fetch_add(1, Ordering::Relaxed);
    let run_cancel = CancellationToken::new();
    let (command_tx, command_rx) = mpsc::channel(8);
    let active_run = WorkspaceActiveRun {
        id: run_id,
        token: run_token,
        node_id: target_node_id,
        cancel: run_cancel.clone(),
        command_tx: command_tx.clone(),
        pending_choice_ids: Arc::new(Mutex::new(std::collections::HashSet::new())),
    };
    *current_run.lock().await = Some(active_run.clone());
    workspace_runs.insert(session_id.clone(), active_run).await;
    // provider drive 期标记（idle 关闭守卫扩展）：从 run 任务启动到结束，该 session
    // 的 idle 守卫都不主动关连接；断连清理不再取消 run 后，该标记覆盖「run 跨
    // socket 存活」窗口（此时 registry 已摘除、新 socket 无 current_run）。
    // 守卫 drop（含 panic/abort 展开）即结束标记，不会泄漏压制 idle 回收。
    let provider_drive_guard = workspace_runs.begin_provider_drive(&session_id);

    {
        let mut engine = engine.lock().await;
        engine.mark_active_run_started(run_label.clone());
    }

    let engine_for_run = engine.clone();
    let current_run_for_task = current_run.clone();
    let workspace_runs_for_task = workspace_runs.clone();
    let session_id_for_task = session_id.clone();
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
        // 声明在首位 → 任务体结束时最后 drop：drive 标记覆盖整个 provider 驱动期。
        let _provider_drive_guard = provider_drive_guard;
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
                    .handle_user_message(content, provider_for_run.clone(), command_rx)
                    .await;
            }
            ProviderRunKind::AuthorChoiceFollowup { content } => {
                engine
                    .handle_author_choice_followup_message(
                        content,
                        provider_for_run.clone(),
                        command_rx,
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
                let rebuilt = rebuild_context.as_ref();
                let lifecycle_for_run = LifecycleStore::new(run_context_clone.app_paths.clone());
                let app_paths_for_run = run_context_clone.app_paths.clone();
                let session_record_for_run = run_context_clone.session_record.clone();
                let mut command_rx = command_rx;

                let request =
                    match build_work_item_plan_generate_request(&engine, &lifecycle_for_run)
                        .map_err(|e| format!("build request failed: {e}"))
                    {
                        Ok(r) => r,
                        Err(message) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    };

                let repository = match workspace_repository_for_session(
                    &app_paths_for_run,
                    &lifecycle_for_run,
                    &session_record_for_run,
                ) {
                    Ok(r) => r,
                    Err(error) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("load repository failed: {error}"),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                let issue = match IssueStore::new(app_paths_for_run.clone()).get(
                    &session_record_for_run.project_id,
                    &session_record_for_run.issue_id,
                ) {
                    Ok(i) => i,
                    Err(error) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("load issue failed: {error}"),
                        };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                let author_provider = engine.session().author_provider.clone();
                let plan_launch = match resolve_plan_author_launch(
                    &engine,
                    repository
                        .logical_repository_id
                        .as_ref()
                        .map(|id| id.0.to_string()),
                    repository
                        .primary_checkout_id
                        .as_ref()
                        .map(|id| id.0.to_string()),
                ) {
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

                // 首次生成：使用完整 prompt + context_resolutions；
                // review/AutoRevision：使用同一会话增量返修 prompt，不重复完整上下文。
                let mut invocation = if is_outline_revision_run {
                    let feedback = outline_revision_feedback
                        .as_deref()
                        .unwrap_or("用户在 Author Confirm 阶段要求返修当前 Outline");
                    match WorkItemSplitEngine::build_outline_revision_invocation(
                        &request,
                        &issue,
                        &repository,
                        author_provider,
                        feedback,
                        &routing_context,
                    ) {
                        Ok(invocation) => invocation,
                        Err(error) => {
                            engine.mark_active_run_finished(&run_label);
                            let err = WsOutMessage::Error {
                                message: format!(
                                    "outline revision invocation failed: {}",
                                    error.message
                                ),
                            };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    }
                } else {
                    let context_resolutions = match load_work_item_plan_outline_context_resolutions(
                        &app_paths_for_run,
                        &session_record_for_run,
                        &request,
                        &lifecycle_for_run,
                        &issue,
                    ) {
                        Ok(resolutions) => resolutions,
                        Err(message) => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    };
                    match WorkItemSplitEngine::build_outline_invocation(
                        &request,
                        &lifecycle_for_run,
                        &issue,
                        &repository,
                        author_provider,
                        &context_resolutions,
                        &routing_context,
                    ) {
                        Ok(invocation) => invocation,
                        Err(error) => {
                            engine.mark_active_run_finished(&run_label);
                            let err = WsOutMessage::Error {
                                message: format!("split generate failed: {}", error.message),
                            };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    }
                };

                // B3：StaleContext 重建 —— 用 rebuilt planning context 构造新 run：
                // 聚合根 cwd（provider_context_root）作为 worktree，注入
                // inventory/effective members 到 prompt（不沿用旧会话内容）。
                if let Some(rebuilt) = rebuilt {
                    invocation.worktree_path = rebuilt.cwd.to_string_lossy().to_string();
                    invocation.prompt.push_str(
                        &crate::product::workspace_engine::aggregate_work_item_target_scope_prompt(
                            &rebuilt.inventory_injection.rendered,
                            &rebuilt.snapshot.effective_member_ids,
                        ),
                    );
                }

                let node_id = if rebuilt.is_some() {
                    // B3：重建 run 不沿用中断会话的 OutlineRun 节点，新建节点。
                    engine.begin_work_item_plan_outline_run().await
                } else if engine.active_node_type()
                    == Some(
                        crate::web::workspace_ws_types::TimelineNodeType::WorkItemPlanOutlineRun,
                    )
                {
                    match engine.active_timeline_node_id() {
                        Some(node_id) => node_id,
                        None => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            let err = WsOutMessage::Error {
                                message: "work item plan outline run node unavailable".to_string(),
                            };
                            let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                            return;
                        }
                    }
                } else {
                    engine.begin_work_item_plan_outline_run().await
                };
                engine
                    .emit_provider_prompt_event(
                        &node_id,
                        invocation.prompt.clone(),
                        if is_outline_revision_run {
                            "发送给 WorkItemPlan provider 的增量返修提示词"
                        } else {
                            "发送给 WorkItemPlan provider 的完整提示词"
                        },
                        Some(invocation.author_provider.clone()),
                    )
                    .await;
                // B3 round3：StaleContext 重建 run 的 provider **全新启动** —— 不携带
                // provider_resume_session_id（不读取旧 Author conversation，避免以 --resume
                // 复用旧 provider 原生会话）；legacy 正常 resume（SameContext/传统单仓）
                // 保持现有行为（携带会话 id）。
                let provider_input = if rebuilt.is_some() {
                    engine.build_work_item_plan_streaming_input_fresh(
                        invocation.provider_type.clone(),
                        invocation.prompt.clone(),
                        invocation.worktree_path.clone(),
                        invocation.author_provider.clone(),
                    )
                } else {
                    engine.build_work_item_plan_streaming_input(
                        invocation.provider_type.clone(),
                        invocation.prompt.clone(),
                        invocation.worktree_path.clone(),
                        invocation.author_provider.clone(),
                    )
                };
                let provider_input = engine.attach_tool_policy_audit(provider_input);
                let provider_session = start_work_item_plan_author(
                    plan_launch,
                    provider_for_run.clone(),
                    provider_input,
                    run_cancel.clone(),
                )
                .await;
                // 新 BLOCKER 修复：rebuilt snapshot 仅在 provider 成功启动后 commit。
                // provider 启动失败不落盘 —— 重连仍判 StaleContext（避免再次 TOCTOU）。
                if let Some(rebuilt) = rebuilt {
                    commit_rebuilt_snapshot_after_provider_start(
                        &app_paths_for_run,
                        rebuilt,
                        &provider_session,
                    );
                }
                let full_output = match engine
                    .drive_work_item_plan_provider_session_to_output(
                        provider_session,
                        &mut command_rx,
                        node_id,
                        invocation.author_provider.clone(),
                    )
                    .await
                {
                    Ok(output) => output,
                    Err(_) => {
                        engine.mark_active_run_finished(&run_label);
                        return;
                    }
                };
                let mut outcome = match complete_work_item_plan_outline_author_from_output(
                    &mut engine,
                    &full_output,
                )
                .await
                {
                    Ok(o) => o,
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

                // 本地循环处理 AutoRevision：基于同一会话增量返修，不重复完整上下文。
                // 循环上限由 `complete_work_item_plan_author` 内部的
                // `work_item_plan_author_retry_count` 控制（达到 3 次后返回 HumanConfirm）；
                // 下方的 `revision_iterations` 作为硬兜底。
                let mut revision_iterations = 0;
                loop {
                    match outcome {
                        WorkItemPlanAuthorOutcome::AuthorConfirm => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            clear_active_run_if_token(
                                &current_run_for_task,
                                &workspace_runs_for_task,
                                &session_id_for_task,
                                run_token,
                            )
                            .await;
                            return;
                        }
                        WorkItemPlanAuthorOutcome::HumanConfirm { reason: _ } => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            clear_active_run_if_token(
                                &current_run_for_task,
                                &workspace_runs_for_task,
                                &session_id_for_task,
                                run_token,
                            )
                            .await;
                            return;
                        }
                        WorkItemPlanAuthorOutcome::AutoRevision { findings } => {
                            revision_iterations += 1;
                            if revision_iterations > 5 {
                                engine.mark_active_run_finished(&run_label);
                                drop(engine);
                                let err = WsOutMessage::Error {
                                    message: "work item plan author revision exceeded hard limit"
                                        .to_string(),
                                };
                                let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                return;
                            }

                            let feedback = combine_outline_auto_retry_feedback(
                                outline_revision_feedback.as_deref(),
                                &findings,
                            );
                            let retry_of_node_id = engine
                                .active_timeline_node_id()
                                .unwrap_or_else(|| "timeline_node_unknown".to_string());
                            let retry_error = work_item_plan_retry_error(&findings);
                            let author_provider = engine.session().author_provider.clone();
                            let plan_launch = match resolve_plan_author_launch(
                                &engine,
                                repository
                                    .logical_repository_id
                                    .as_ref()
                                    .map(|id| id.0.to_string()),
                                repository
                                    .primary_checkout_id
                                    .as_ref()
                                    .map(|id| id.0.to_string()),
                            ) {
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
                            invocation =
                                match WorkItemSplitEngine::build_outline_revision_invocation(
                                    &request,
                                    &issue,
                                    &repository,
                                    author_provider,
                                    &feedback,
                                    &routing_context,
                                ) {
                                    Ok(invocation) => invocation,
                                    Err(error) => {
                                        engine.mark_active_run_finished(&run_label);
                                        drop(engine);
                                        let err = WsOutMessage::Error {
                                            message: format!(
                                                "outline revision invocation failed: {}",
                                                error.message
                                            ),
                                        };
                                        let _ =
                                            send_json_outbound(&outbound_tx_for_task, &err).await;
                                        return;
                                    }
                                };
                            let node_id = engine
                                .begin_work_item_plan_outline_auto_retry_run(
                                    retry_of_node_id,
                                    revision_iterations + 1,
                                    retry_error.code.clone(),
                                    retry_error,
                                )
                                .await;
                            engine
                                .emit_provider_prompt_event(
                                    &node_id,
                                    invocation.prompt.clone(),
                                    "发送给 WorkItemPlan provider 的增量返修提示词",
                                    Some(invocation.author_provider.clone()),
                                )
                                .await;
                            let provider_input = engine.build_work_item_plan_streaming_input(
                                invocation.provider_type.clone(),
                                invocation.prompt.clone(),
                                invocation.worktree_path.clone(),
                                invocation.author_provider.clone(),
                            );
                            let provider_input = engine.attach_tool_policy_audit(provider_input);
                            let provider_session = start_work_item_plan_author(
                                plan_launch,
                                provider_for_run.clone(),
                                provider_input,
                                run_cancel.clone(),
                            )
                            .await;
                            let full_output = match engine
                                .drive_work_item_plan_provider_session_to_output(
                                    provider_session,
                                    &mut command_rx,
                                    node_id,
                                    invocation.author_provider.clone(),
                                )
                                .await
                            {
                                Ok(output) => output,
                                Err(_) => {
                                    engine.mark_active_run_finished(&run_label);
                                    return;
                                }
                            };
                            outcome = match complete_work_item_plan_outline_author_from_output(
                                &mut engine,
                                &full_output,
                            )
                            .await
                            {
                                Ok(o) => o,
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
                        }
                    }
                }
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
                    Ok(single_candidate::SingleCandidateProviderRunOutcome::AlreadyReserved) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        clear_active_run_if_token(
                            &current_run_for_task,
                            &workspace_runs_for_task,
                            &session_id_for_task,
                            run_token,
                        )
                        .await;
                        return;
                    }
                    Err(single_candidate::SingleCandidateProviderRunError::AlreadyFinished) => {
                        engine.mark_active_run_finished(&run_label);
                        return;
                    }
                    Err(single_candidate::SingleCandidateProviderRunError::Message(message)) => {
                        engine
                            .finish_active_run_with_failed_node(message.clone())
                            .await;
                        drop(engine);
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
                    current_run_for_task,
                    workspace_runs_for_task,
                    session_id_for_task,
                    run_token,
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

                let node_id = if let Some(node_id) = engine.active_timeline_node_id() {
                    node_id
                } else {
                    engine.begin_work_item_plan_author_run().await
                };
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
                let provider_input = engine.build_work_item_plan_streaming_input(
                    crate::product::work_item_split_engine::types::provider_name_to_type(
                        &author_provider,
                    ),
                    prompt.clone(),
                    worktree_path.to_string_lossy().to_string(),
                    author_provider.clone(),
                );
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
            current_run_for_task,
            workspace_runs_for_task,
            session_id_for_task,
            run_token,
            run_label,
            outbound_tx_for_task,
            run_cancel,
            run_context_clone
        );
        engine.mark_active_run_finished(&run_label);
        drop(engine);

        clear_active_run_if_token(
            &current_run_for_task,
            &workspace_runs_for_task,
            &session_id_for_task,
            run_token,
        )
        .await;
    });

    Ok(())
}
