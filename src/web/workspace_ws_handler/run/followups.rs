use super::*;

pub(super) fn work_item_plan_findings_feedback(findings: &[WorkItemSplitFinding]) -> String {
    findings
        .iter()
        .map(|finding| {
            format!(
                "- [{}] {}: {} ({})",
                finding.severity.as_str(),
                finding.code,
                finding.message,
                finding.work_item_ids.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn combine_outline_auto_retry_feedback(
    base_feedback: Option<&str>,
    findings: &[WorkItemSplitFinding],
) -> String {
    let mut parts = Vec::new();
    if let Some(base) = base_feedback
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(base.to_string());
    }
    let findings_feedback = work_item_plan_findings_feedback(findings);
    if !findings_feedback.trim().is_empty() {
        parts.push(format!("[auto_retry_error]\n{findings_feedback}"));
    }
    parts.join("\n\n")
}

pub(super) fn work_item_plan_retry_error(
    findings: &[WorkItemSplitFinding],
) -> TimelineNodeRetryError {
    findings
        .first()
        .map(|finding| TimelineNodeRetryError {
            code: finding.code.clone(),
            message: finding.message.clone(),
        })
        .unwrap_or_else(|| TimelineNodeRetryError {
            code: "work_item_plan_outline_auto_retry".to_string(),
            message: "WorkItemPlan Outline 自动重跑".to_string(),
        })
}

macro_rules! workspace_ws_work_item_plan_revision_arm {
    (
        $engine:ident,
        $run_context_clone:ident,
        $provider_for_run:ident,
        $run_cancel:ident,
        $command_rx:ident,
        $run_label:ident,
        $outbound_tx_for_task:ident,
        $current_run_for_task:ident,
        $workspace_runs_for_task:ident,
        $session_id_for_task:ident,
        $run_token:ident,
        $feedback:ident
    ) => {{
                let lifecycle_for_run = LifecycleStore::new($run_context_clone.app_paths.clone());
                let app_paths_for_run = $run_context_clone.app_paths.clone();
                let session_record_for_run = $run_context_clone.session_record.clone();
                let mut $command_rx = $command_rx;

                let (retained, redo_specs, request) = match build_work_item_plan_revision_input(
                    &$engine,
                    &lifecycle_for_run,
                    $feedback.as_deref(),
                )
                .map_err(|e| format!("build revision input failed: {e}"))
                {
                    Ok(r) => r,
                    Err(message) => {
                        $engine.mark_active_run_finished(&$run_label);
                        drop($engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
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
                        $engine.mark_active_run_finished(&$run_label);
                        drop($engine);
                        let err = WsOutMessage::Error {
                            message: format!("load repository failed: {error}"),
                        };
                        let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                let issue = match IssueStore::new(app_paths_for_run.clone()).get(
                    &session_record_for_run.project_id,
                    &session_record_for_run.issue_id,
                ) {
                    Ok(i) => i,
                    Err(error) => {
                        $engine.mark_active_run_finished(&$run_label);
                        drop($engine);
                        let err = WsOutMessage::Error {
                            message: format!("load issue failed: {error}"),
                        };
                        let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                let author_provider = $engine.session().author_provider.clone();
                let invocation = match WorkItemSplitEngine::build_revision_invocation(
                    &request,
                    &lifecycle_for_run,
                    &issue,
                    &repository,
                    author_provider,
                    &retained,
                    &redo_specs,
                ) {
                    Ok(invocation) => invocation,
                    Err(error) => {
                        $engine.mark_active_run_finished(&$run_label);
                        drop($engine);
                        let err = WsOutMessage::Error {
                            message: format!("split generate_revision failed: {}", error.message),
                        };
                        let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                let Some(node_id) = $engine.active_timeline_node_id() else {
                    $engine.mark_active_run_finished(&$run_label);
                    drop($engine);
                    let err = WsOutMessage::Error {
                        message: "work item plan revision node unavailable".to_string(),
                    };
                    let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                    return;
                };
                let provider_input = $engine.build_work_item_plan_streaming_input(
                    invocation.provider_type.clone(),
                    invocation.prompt.clone(),
                    invocation.worktree_path.clone(),
                    invocation.author_provider.clone(),
                );
                let provider_session = $provider_for_run
                    .start(provider_input, $run_cancel.clone())
                    .await;
                let full_output = match $engine
                    .drive_work_item_plan_provider_session_to_output(
                        provider_session,
                        &mut $command_rx,
                        node_id,
                        invocation.author_provider.clone(),
                    )
                    .await
                {
                    Ok(output) => output,
                    Err(_) => {
                        $engine.mark_active_run_finished(&$run_label);
                        return;
                    }
                };
                let structured_output = match parse_work_item_split_structured_output(&full_output)
                {
                    Ok(output) => output,
                    Err(message) => {
                        $engine.mark_active_run_finished(&$run_label);
                        drop($engine);
                        let err = WsOutMessage::Error {
                            message: format!("split generate_revision failed: {message}"),
                        };
                        let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                let output = match WorkItemSplitEngine::complete_revision_from_structured_output(
                    &request,
                    &lifecycle_for_run,
                    &issue,
                    &repository,
                    &invocation.author_provider,
                    &invocation.prompt,
                    structured_output,
                    &retained,
                    &redo_specs,
                ) {
                    Ok(o) => o,
                    Err(error) => {
                        $engine.mark_active_run_finished(&$run_label);
                        drop($engine);
                        let err = WsOutMessage::Error {
                            message: format!("split generate_revision failed: {}", error.message),
                        };
                        let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                let mut outcome = match $engine.complete_work_item_plan_revision(output).await {
                    Ok(o) => o,
                    Err(message) => {
                        $engine.mark_active_run_finished(&$run_label);
                        drop($engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                        return;
                    }
                };

                // revision 也可能 validate 失败，使用与 author 相同的本地 AutoRevision 循环。
                // 由于 revision 的 retained/redo 是面向用户反馈的局部集合，AutoRevision 时
                // 退化为整组生成（retained/redo 均为空），让模型基于 validator findings 全局调整。
                let mut revision_iterations = 0;
                loop {
                    match outcome {
                        WorkItemPlanAuthorOutcome::AuthorConfirm => {
                            $engine.mark_active_run_finished(&$run_label);
                            drop($engine);
                            clear_active_run_if_token(
                                &$current_run_for_task,
                                &$workspace_runs_for_task,
                                &$session_id_for_task,
                                $run_token,
                            )
                            .await;
                            return;
                        }
                        WorkItemPlanAuthorOutcome::HumanConfirm { reason: _ } => {
                            $engine.mark_active_run_finished(&$run_label);
                            drop($engine);
                            clear_active_run_if_token(
                                &$current_run_for_task,
                                &$workspace_runs_for_task,
                                &$session_id_for_task,
                                $run_token,
                            )
                            .await;
                            return;
                        }
                        WorkItemPlanAuthorOutcome::AutoRevision { findings: _ } => {
                            revision_iterations += 1;
                            if revision_iterations > 5 {
                                $engine.mark_active_run_finished(&$run_label);
                                drop($engine);
                                let err = WsOutMessage::Error {
                                    message: "work item plan revision exceeded hard limit"
                                        .to_string(),
                                };
                                let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                                return;
                            }

                            // 每次重生前重新构建请求，把最新 persisted 的 validator_findings
                            // 作为 revision_feedback 注入 prompt。
                            let request = match build_work_item_plan_generate_request(
                                &$engine,
                                &lifecycle_for_run,
                            )
                            .map_err(|e| format!("build request failed: {e}"))
                            {
                                Ok(r) => r,
                                Err(message) => {
                                    $engine.mark_active_run_finished(&$run_label);
                                    drop($engine);
                                    let err = WsOutMessage::Error { message };
                                    let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                                    return;
                                }
                            };

                            // 整组 AutoRevision 时丢弃局部 retained/redo，使用完整 generate_revision。
                            let author_provider = $engine.session().author_provider.clone();
                            let invocation = match WorkItemSplitEngine::build_revision_invocation(
                                &request,
                                &lifecycle_for_run,
                                &issue,
                                &repository,
                                author_provider,
                                &[],
                                &[],
                            ) {
                                Ok(invocation) => invocation,
                                Err(error) => {
                                    $engine.mark_active_run_finished(&$run_label);
                                    drop($engine);
                                    let err = WsOutMessage::Error {
                                        message: format!(
                                            "split generate_revision failed: {}",
                                            error.message
                                        ),
                                    };
                                    let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                                    return;
                                }
                            };
                            let node_id = $engine
                                .begin_work_item_plan_auto_revision_run(revision_iterations)
                                .await;
                            let provider_input = $engine.build_work_item_plan_streaming_input(
                                invocation.provider_type.clone(),
                                invocation.prompt.clone(),
                                invocation.worktree_path.clone(),
                                invocation.author_provider.clone(),
                            );
                            let provider_session = $provider_for_run
                                .start(provider_input, $run_cancel.clone())
                                .await;
                            let full_output = match $engine
                                .drive_work_item_plan_provider_session_to_output(
                                    provider_session,
                                    &mut $command_rx,
                                    node_id,
                                    invocation.author_provider.clone(),
                                )
                                .await
                            {
                                Ok(output) => output,
                                Err(_) => {
                                    $engine.mark_active_run_finished(&$run_label);
                                    return;
                                }
                            };
                            let structured_output =
                                match parse_work_item_split_structured_output(&full_output) {
                                    Ok(output) => output,
                                    Err(message) => {
                                        $engine.mark_active_run_finished(&$run_label);
                                        drop($engine);
                                        let err = WsOutMessage::Error {
                                            message: format!(
                                                "split generate_revision failed: {message}"
                                            ),
                                        };
                                        let _ =
                                            send_json_outbound(&$outbound_tx_for_task, &err).await;
                                        return;
                                    }
                                };
                            let output =
                                match WorkItemSplitEngine::complete_revision_from_structured_output(
                                    &request,
                                    &lifecycle_for_run,
                                    &issue,
                                    &repository,
                                    &invocation.author_provider,
                                    &invocation.prompt,
                                    structured_output,
                                    &[],
                                    &[],
                                ) {
                                    Ok(o) => o,
                                    Err(error) => {
                                        $engine.mark_active_run_finished(&$run_label);
                                        drop($engine);
                                        let err = WsOutMessage::Error {
                                            message: format!(
                                                "split generate_revision failed: {}",
                                                error.message
                                            ),
                                        };
                                        let _ =
                                            send_json_outbound(&$outbound_tx_for_task, &err).await;
                                        return;
                                    }
                                };

                            outcome = match $engine.complete_work_item_plan_revision(output).await {
                                Ok(o) => o,
                                Err(message) => {
                                    $engine.mark_active_run_finished(&$run_label);
                                    drop($engine);
                                    let err = WsOutMessage::Error { message };
                                    let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                                    return;
                                }
                            };
                        }
                    }
                }
    }};
}

macro_rules! workspace_ws_provider_run_followups {
    (
        $engine:ident,
        $provider_registry_for_run:ident,
        $current_run_for_task:ident,
        $workspace_runs_for_task:ident,
        $session_id_for_task:ident,
        $run_token:ident,
        $run_label:ident,
        $outbound_tx_for_task:ident,
        $run_cancel:ident,
        $run_context_clone:ident
    ) => {{
        while $engine.session().stage == WorkspaceStage::CrossReview {
            let reviewer_name = $engine
                .session()
                .reviewer_provider
                .clone()
                .unwrap_or(ProviderName::Codex);
            let Some(provider_for_review) = $provider_registry_for_run.get(&reviewer_name) else {
                break;
            };
            let (review_command_tx, review_command_rx) = mpsc::channel(8);
            {
                let mut current = $current_run_for_task.lock().await;
                if let Some(active) = current.as_mut()
                    && active.token == $run_token
                {
                    active.command_tx = review_command_tx.clone();
                }
            }
            $workspace_runs_for_task
                .replace_command_tx_if_token(&$session_id_for_task, $run_token, review_command_tx)
                .await;
            $engine
                .drive_review_session(provider_for_review, review_command_rx)
                .await;
        }
        if $engine.session().workspace_type == WorkspaceType::WorkItemPlan
            && $engine.session().stage == WorkspaceStage::Running
            && $engine.active_node_type()
                == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemPlanOutlineRun)
        {
            let author_name = $engine.session().author_provider.clone();
            let Some(provider_for_outline) = $provider_registry_for_run.get(&author_name) else {
                $engine.mark_active_run_finished(&$run_label);
                drop($engine);
                let err = WsOutMessage::Error {
                    message: format!("provider unavailable: {author_name:?}"),
                };
                let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                clear_active_run_if_token(
                    &$current_run_for_task,
                    &$workspace_runs_for_task,
                    &$session_id_for_task,
                    $run_token,
                )
                .await;
                return;
            };
            let (outline_command_tx, mut outline_command_rx) = mpsc::channel(8);
            {
                let mut current = $current_run_for_task.lock().await;
                if let Some(active) = current.as_mut()
                    && active.token == $run_token
                {
                    active.command_tx = outline_command_tx.clone();
                }
            }
            $workspace_runs_for_task
                .replace_command_tx_if_token(&$session_id_for_task, $run_token, outline_command_tx)
                .await;
            match drive_current_work_item_plan_outline_run(
                &mut $engine,
                provider_for_outline,
                $run_cancel.clone(),
                &mut outline_command_rx,
                &$run_context_clone.app_paths,
                &$run_context_clone.session_record,
            )
            .await
            {
                Ok(WorkItemPlanAuthorOutcome::AuthorConfirm)
                | Ok(WorkItemPlanAuthorOutcome::HumanConfirm { .. }) => {}
                Ok(WorkItemPlanAuthorOutcome::AutoRevision { .. }) => {
                    $engine.mark_active_run_finished(&$run_label);
                    drop($engine);
                    let err = WsOutMessage::Error {
                        message: "work item plan outline auto revision after review is not supported in follow-up run".to_string(),
                    };
                    let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                    clear_active_run_if_token(
                        &$current_run_for_task,
                        &$workspace_runs_for_task,
                        &$session_id_for_task,
                        $run_token,
                    )
                    .await;
                    return;
                }
                Err(message) => {
                    $engine.mark_active_run_finished(&$run_label);
                    drop($engine);
                    let err = WsOutMessage::Error { message };
                    let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                    clear_active_run_if_token(
                        &$current_run_for_task,
                        &$workspace_runs_for_task,
                        &$session_id_for_task,
                        $run_token,
                    )
                    .await;
                    return;
                }
            }
        }
        if $engine.session().workspace_type == WorkspaceType::WorkItemPlan
            && $engine.session().stage == WorkspaceStage::Running
            && $engine.active_node_type()
                == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemDraftRun)
        {
            let author_name = $engine.session().author_provider.clone();
            let Some(provider_for_draft) = $provider_registry_for_run.get(&author_name) else {
                $engine.mark_active_run_finished(&$run_label);
                drop($engine);
                let err = WsOutMessage::Error {
                    message: format!("provider unavailable: {author_name:?}"),
                };
                let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                clear_active_run_if_token(
                    &$current_run_for_task,
                    &$workspace_runs_for_task,
                    &$session_id_for_task,
                    $run_token,
                )
                .await;
                return;
            };
            let (draft_command_tx, mut draft_command_rx) = mpsc::channel(8);
            {
                let mut current = $current_run_for_task.lock().await;
                if let Some(active) = current.as_mut()
                    && active.token == $run_token
                {
                    active.command_tx = draft_command_tx.clone();
                }
            }
            $workspace_runs_for_task
                .replace_command_tx_if_token(&$session_id_for_task, $run_token, draft_command_tx)
                .await;
            let Some(node_id) = $engine.active_timeline_node_id() else {
                $engine.mark_active_run_finished(&$run_label);
                drop($engine);
                let err = WsOutMessage::Error {
                    message: "work item draft run node unavailable".to_string(),
                };
                let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                clear_active_run_if_token(
                    &$current_run_for_task,
                    &$workspace_runs_for_task,
                    &$session_id_for_task,
                    $run_token,
                )
                .await;
                return;
            };
            let provider_input = match $engine.build_current_work_item_draft_streaming_input(None) {
                Ok(input) => input,
                Err(message) => {
                    $engine.mark_active_run_finished(&$run_label);
                    drop($engine);
                    let err = WsOutMessage::Error { message };
                    let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                    clear_active_run_if_token(
                        &$current_run_for_task,
                        &$workspace_runs_for_task,
                        &$session_id_for_task,
                        $run_token,
                    )
                    .await;
                    return;
                }
            };
            $engine
                .emit_provider_prompt_event(
                    &node_id,
                    provider_input.prompt.clone(),
                    "发送给 WorkItemDraft provider 的完整提示词",
                    Some(author_name.clone()),
                )
                .await;
            let provider_session = provider_for_draft
                .start(provider_input, $run_cancel.clone())
                .await;
            let full_output = match $engine
                .drive_work_item_plan_provider_session_to_output(
                    provider_session,
                    &mut draft_command_rx,
                    node_id,
                    author_name,
                )
                .await
            {
                Ok(output) => output,
                Err(_) => {
                    $engine.mark_active_run_finished(&$run_label);
                    return;
                }
            };
            let structured_output = match parse_work_item_split_structured_output(&full_output) {
                Ok(output) => output,
                Err(message) => {
                    $engine.mark_active_run_finished(&$run_label);
                    drop($engine);
                    let err = WsOutMessage::Error {
                        message: format!("work item draft generate failed: {message}"),
                    };
                    let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                    clear_active_run_if_token(
                        &$current_run_for_task,
                        &$workspace_runs_for_task,
                        &$session_id_for_task,
                        $run_token,
                    )
                    .await;
                    return;
                }
            };
            let candidate = match parse_work_item_draft_output(structured_output) {
                Ok(candidate) => candidate,
                Err(error) => {
                    $engine.mark_active_run_finished(&$run_label);
                    drop($engine);
                    let err = WsOutMessage::Error {
                        message: format!("work item draft parse failed: {}", error.message),
                    };
                    let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                    clear_active_run_if_token(
                        &$current_run_for_task,
                        &$workspace_runs_for_task,
                        &$session_id_for_task,
                        $run_token,
                    )
                    .await;
                    return;
                }
            };
            if let Err(message) = $engine.complete_work_item_draft_author(candidate).await {
                $engine.mark_active_run_finished(&$run_label);
                drop($engine);
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&$outbound_tx_for_task, &err).await;
                clear_active_run_if_token(
                    &$current_run_for_task,
                    &$workspace_runs_for_task,
                    &$session_id_for_task,
                    $run_token,
                )
                .await;
                return;
            }
        }
    }};
}

pub(crate) async fn drive_current_work_item_plan_outline_run(
    engine: &mut WorkspaceEngine,
    provider: Arc<dyn StreamingProviderAdapter>,
    run_cancel: CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    app_paths: &ProductAppPaths,
    session_record: &WorkspaceSessionRecord,
) -> Result<WorkItemPlanAuthorOutcome, String> {
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let mut request = build_work_item_plan_generate_request(engine, &lifecycle)
        .map_err(|error| format!("build request failed: {error}"))?;
    let repository = workspace_repository_for_session(app_paths, &lifecycle, session_record)
        .map_err(|error| format!("load repository failed: {error}"))?;
    let issue = IssueStore::new(app_paths.clone())
        .get(&session_record.project_id, &session_record.issue_id)
        .map_err(|error| format!("load issue failed: {error}"))?;

    let mut revision_iterations = 0;
    loop {
        let author_provider = engine.session().author_provider.clone();
        let context_resolutions = load_work_item_plan_outline_context_resolutions(
            app_paths,
            session_record,
            &request,
            &lifecycle,
            &issue,
        )?;
        let invocation = WorkItemSplitEngine::build_outline_invocation(
            &request,
            &lifecycle,
            &issue,
            &repository,
            author_provider,
            &context_resolutions,
        )
        .map_err(|error| format!("split generate failed: {}", error.message))?;
        let node_id = engine
            .active_timeline_node_id()
            .ok_or_else(|| "work item plan outline run node unavailable".to_string())?;
        let provider_input = engine.build_work_item_plan_streaming_input(
            invocation.provider_type.clone(),
            invocation.prompt.clone(),
            invocation.worktree_path.clone(),
            invocation.author_provider.clone(),
        );
        let provider_session = provider.start(provider_input, run_cancel.clone()).await;
        let full_output = engine
            .drive_work_item_plan_provider_session_to_output(
                provider_session,
                command_rx,
                node_id,
                invocation.author_provider.clone(),
            )
            .await?;
        let outcome =
            complete_work_item_plan_outline_author_from_output(engine, &full_output).await?;

        match outcome {
            WorkItemPlanAuthorOutcome::AuthorConfirm
            | WorkItemPlanAuthorOutcome::HumanConfirm { .. } => return Ok(outcome),
            WorkItemPlanAuthorOutcome::AutoRevision { findings } => {
                revision_iterations += 1;
                if revision_iterations > 5 {
                    return Err(
                        "work item plan outline author revision exceeded hard limit".to_string()
                    );
                }
                let retry_of_node_id = engine
                    .active_timeline_node_id()
                    .unwrap_or_else(|| "timeline_node_unknown".to_string());
                let retry_error = work_item_plan_retry_error(&findings);
                request.revision_feedback =
                    Some(combine_outline_auto_retry_feedback(None, &findings));
                engine
                    .begin_work_item_plan_outline_auto_retry_run(
                        retry_of_node_id,
                        revision_iterations + 1,
                        retry_error.code.clone(),
                        retry_error,
                    )
                    .await;
            }
        }
    }
}
