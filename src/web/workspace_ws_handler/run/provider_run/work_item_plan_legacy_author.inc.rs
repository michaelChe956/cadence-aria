// provider_run 拆分（large_file_guard 1200 行上限）：WorkItemPlanLegacyAuthor/OutlineRevision/OutlineRebuild 臂体经 include! 挂载于该臂表达式位置，函数与模块作用域不变，行为零变化（纯移动）。
{
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
                        PlanAuthorOutputContract::Structured,
                    )
                } else {
                    engine.build_work_item_plan_streaming_input(
                        invocation.provider_type.clone(),
                        invocation.prompt.clone(),
                        invocation.worktree_path.clone(),
                        invocation.author_provider.clone(),
                        PlanAuthorOutputContract::Structured,
                    )
                };
                // REQ-PIB-02（T2.3）：基线不可解析 → 终止本轮 run（可观测错误出站）。
                let provider_input = match provider_input {
                    Ok(provider_input) => provider_input,
                    Err(message) => {
                        engine.mark_active_run_finished(&run_label);
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                let provider_input = engine.attach_tool_policy_audit(provider_input);
                let provider_session = start_work_item_plan_author(
                    plan_launch,
                    provider_for_run.clone(),
                    provider_input,
                    run_cancel.clone(),
                    None,
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
                            drop(provider_drive_guard.take());
                            manager_for_task.finish_run(run_token).await;
                            return;
                        }
                        WorkItemPlanAuthorOutcome::HumanConfirm { reason: _ } => {
                            engine.mark_active_run_finished(&run_label);
                            drop(engine);
                            drop(provider_drive_guard.take());
                            manager_for_task.finish_run(run_token).await;
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
                                PlanAuthorOutputContract::Structured,
                            );
                            // REQ-PIB-02（T2.3）：基线不可解析 → 终止本轮 run。
                            let provider_input = match provider_input {
                                Ok(provider_input) => provider_input,
                                Err(message) => {
                                    engine.mark_active_run_finished(&run_label);
                                    drop(engine);
                                    let err = WsOutMessage::Error { message };
                                    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
                                    return;
                                }
                            };
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
