use super::*;

pub(crate) enum SingleCandidateProviderRunOutcome {
    Completed,
    AlreadyReserved,
}

pub(crate) enum SingleCandidateProviderRunError {
    AlreadyFinished,
    Message(String),
}

/// SingleCandidate 的内部两阶段 author 链路。
///
/// 轻量 markdown outline 只用于 compiler parser 的机械候选计数；计数成功后才选择
/// 内部 mode、记录 diagnostic 并启动完整 markdown author。此模块刻意不触碰 legacy
/// outline/draft/batch 链路。
pub(crate) async fn run_single_candidate_author(
    engine: &mut WorkspaceEngine,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    run_cancel: CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    run_context: &ProviderRunContext,
) -> Result<SingleCandidateProviderRunOutcome, SingleCandidateProviderRunError> {
    let should_start = match engine.reserve_single_candidate_author_start() {
        Ok(should_start) => should_start,
        Err(message) => {
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::Message(message));
        }
    };
    if !should_start {
        return Ok(SingleCandidateProviderRunOutcome::AlreadyReserved);
    }

    let lifecycle = LifecycleStore::new(run_context.app_paths.clone());
    let request = build_work_item_plan_generate_request(engine, &lifecycle).map_err(|error| {
        SingleCandidateProviderRunError::Message(format!(
            "build single candidate request failed: {error}"
        ))
    })?;
    let repository = workspace_repository_for_session(
        &run_context.app_paths,
        &lifecycle,
        &run_context.session_record,
    )
    .map_err(|error| {
        SingleCandidateProviderRunError::Message(format!("load repository failed: {error}"))
    })?;
    let issue = IssueStore::new(run_context.app_paths.clone())
        .get(
            &run_context.session_record.project_id,
            &run_context.session_record.issue_id,
        )
        .map_err(|error| {
            SingleCandidateProviderRunError::Message(format!("load issue failed: {error}"))
        })?;
    let story_context = crate::product::work_item_split_engine::context::collect_story_context(
        &lifecycle, &request, &issue,
    )
    .map_err(|error| {
        SingleCandidateProviderRunError::Message(format!(
            "load story context failed: {}",
            error.message
        ))
    })?
    .join("\n\n");
    let design_context = crate::product::work_item_split_engine::context::collect_design_context(
        &lifecycle, &request, &issue,
    )
    .map_err(|error| {
        SingleCandidateProviderRunError::Message(format!(
            "load design context failed: {}",
            error.message
        ))
    })?
    .join("\n\n");
    let repository_structure =
        crate::product::work_item_split_engine::context::summarize_repository_structure(
            &repository.path,
        );
    let author_provider = engine.session().author_provider.clone();

    let outline_launch = resolve_plan_author_launch(
        engine,
        repository
            .logical_repository_id
            .as_ref()
            .map(|id| id.0.to_string()),
        repository
            .primary_checkout_id
            .as_ref()
            .map(|id| id.0.to_string()),
    )
    .map_err(|error| {
        SingleCandidateProviderRunError::Message(format!("logical plan launch failed: {error}"))
    })?;
    let outline_prompt = crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_outline_prompt(
        &request,
        &issue,
        &repository,
        &story_context,
        &design_context,
        &repository_structure,
        &outline_launch.routing_context(),
    )
    .map_err(SingleCandidateProviderRunError::Message)?;
    let node_id = if engine.active_node_type()
        == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemPlanOutlineRun)
    {
        engine.active_timeline_node_id().ok_or_else(|| {
            SingleCandidateProviderRunError::Message(
                "single candidate author run node unavailable".to_string(),
            )
        })?
    } else {
        engine.begin_work_item_plan_outline_run().await
    };

    #[cfg(test)]
    super::record_single_candidate_generation_step(&engine.session().session_id, "outline");
    engine
        .emit_provider_prompt_event(
            &node_id,
            outline_prompt.clone(),
            "发送给 SingleCandidate markdown outline 的轻量提示词",
            Some(author_provider.clone()),
        )
        .await;
    let outline_input = engine.build_work_item_plan_streaming_input(
        crate::product::work_item_split_engine::types::provider_name_to_type(&author_provider),
        outline_prompt,
        repository.path.to_string_lossy().to_string(),
        author_provider.clone(),
    );
    let outline_session = start_work_item_plan_author(
        outline_launch,
        provider_for_run.clone(),
        outline_input,
        run_cancel.clone(),
    )
    .await;
    let outline_output = match engine
        .drive_work_item_plan_provider_session_to_output(
            outline_session,
            command_rx,
            node_id.clone(),
            author_provider.clone(),
        )
        .await
    {
        Ok(output) => output,
        Err(_) => {
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::AlreadyFinished);
        }
    };
    let outline_ast =
        match crate::product::work_item_plan_compiler::parse_work_item_plan(&outline_output) {
            Ok(ast) => ast,
            Err(diagnostics) => {
                let diagnostics = diagnostics
                    .iter()
                    .map(|diagnostic| {
                        format!(
                            "{}:{}:{}",
                            diagnostic.code, diagnostic.line, diagnostic.message
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                engine.persist_single_candidate_terminal_phase(
                    crate::product::models::SingleCandidatePhase::Failed,
                );
                return Err(SingleCandidateProviderRunError::Message(format!(
                    "single candidate markdown outline parse failed: {diagnostics}"
                )));
            }
        };
    let candidate_item_count = outline_ast.items.len();
    let trusted_command_catalog =
        crate::product::work_item_plan_compiler::trusted_command_catalog_from_ast(
            &outline_ast,
            ".",
        );
    #[cfg(test)]
    {
        super::record_work_item_plan_parser_path(
            &engine.session().session_id,
            "single_candidate_outline",
        );
        super::record_single_candidate_generation_step(&engine.session().session_id, "parse_count");
    }

    let full_launch = resolve_plan_author_launch(
        engine,
        repository
            .logical_repository_id
            .as_ref()
            .map(|id| id.0.to_string()),
        repository
            .primary_checkout_id
            .as_ref()
            .map(|id| id.0.to_string()),
    )
    .map_err(|error| {
        SingleCandidateProviderRunError::Message(format!("logical plan launch failed: {error}"))
    })?;
    let full_prompt =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: &story_context,
                design_context: &design_context,
                repository_structure: &repository_structure,
                routing_context: &full_launch.routing_context(),
                trusted_command_catalog: &trusted_command_catalog,
            },
        )
        .map_err(SingleCandidateProviderRunError::Message)?;
    let decision_input = crate::product::workspace_engine::SingleCandidateGenerationDecisionInput {
        provider: author_provider.clone(),
        candidate_item_count,
        prompt_bytes: full_prompt.len(),
        provider_input_budget_bytes:
            crate::product::workspace_engine::single_candidate_provider_input_budget_bytes(
                &author_provider,
            ),
    };
    let generation_mode =
        crate::product::workspace_engine::select_internal_generation_mode(&decision_input);
    #[cfg(test)]
    super::record_single_candidate_generation_step(&engine.session().session_id, "selector");
    let generation_diagnostic = format!(
        "internal generation mode={generation_mode:?}; provider={:?}; candidate_item_count={}; prompt_bytes={}; provider_input_budget_bytes={}",
        decision_input.provider,
        decision_input.candidate_item_count,
        decision_input.prompt_bytes,
        decision_input.provider_input_budget_bytes,
    );
    tracing::info!(
        session_id = %engine.session().session_id,
        provider = ?decision_input.provider,
        candidate_item_count = decision_input.candidate_item_count,
        prompt_bytes = decision_input.prompt_bytes,
        provider_input_budget_bytes = decision_input.provider_input_budget_bytes,
        generation_mode = ?generation_mode,
        "single-candidate internal generation mode selected"
    );
    engine
        .emit_execution_event(
            ProviderExecutionEvent {
                event_id: format!("single_candidate_generation_mode_{node_id}"),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Completed,
                title: "SingleCandidate 内部生成模式已选择".to_string(),
                detail: Some(generation_diagnostic),
                command: None,
                cwd: None,
                output: None,
                exit_code: None,
            },
            Some(node_id.clone()),
            Some(author_provider.clone()),
        )
        .await;
    #[cfg(test)]
    super::record_single_candidate_generation_step(
        &engine.session().session_id,
        "full_markdown_author",
    );
    engine
        .emit_provider_prompt_event(
            &node_id,
            full_prompt.clone(),
            "发送给 SingleCandidate markdown author 的完整提示词",
            Some(author_provider.clone()),
        )
        .await;
    let provider_input = engine.build_work_item_plan_streaming_input(
        crate::product::work_item_split_engine::types::provider_name_to_type(&author_provider),
        full_prompt,
        repository.path.to_string_lossy().to_string(),
        author_provider.clone(),
    );
    let provider_session =
        start_work_item_plan_author(full_launch, provider_for_run, provider_input, run_cancel)
            .await;
    let full_output = match engine
        .drive_work_item_plan_provider_session_to_output(
            provider_session,
            command_rx,
            node_id,
            author_provider,
        )
        .await
    {
        Ok(output) => output,
        Err(_) => {
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::AlreadyFinished);
        }
    };
    if let Err(message) = engine
        .complete_single_candidate_work_item_plan_author(
            full_output,
            repository.id,
            trusted_command_catalog,
        )
        .await
    {
        engine.persist_single_candidate_terminal_phase(
            crate::product::models::SingleCandidatePhase::Failed,
        );
        return Err(SingleCandidateProviderRunError::Message(message));
    }
    #[cfg(test)]
    {
        super::record_work_item_plan_parser_path(
            &engine.session().session_id,
            "single_candidate_markdown",
        );
        super::record_single_candidate_generation_step(
            &engine.session().session_id,
            "parse_source_revision",
        );
    }
    Ok(SingleCandidateProviderRunOutcome::Completed)
}
