use super::*;
use crate::product::work_item_plan_compiler::grammar;

pub(crate) enum SingleCandidateProviderRunOutcome {
    Completed,
    AlreadyReserved,
}

pub(crate) enum SingleCandidateProviderRunError {
    AlreadyFinished,
    Message(String),
}

/// 丢弃 provider 在 markdown 文档标题前输出的前言，保留既有 parser 的失败语义。
///
/// 定位首个固定文档标题的字节偏移并从该处修剪；找不到标题时原样返回，避免把
/// 缺少标题的输出静默转换成另一种错误。
fn trim_provider_preamble(source: &str) -> &str {
    let document_heading = format!("{}\n", grammar::DOCUMENT_HEADING);
    source
        .find(&document_heading)
        .map(|offset| &source[offset..])
        .unwrap_or(source)
}

/// SC author 交付进入 compiler 前的确定性净化（结构标题归一化 + 前言修剪）。
///
/// 归一化先于修剪：前言修剪锚定的是规范英文文档标题，provider 输出「前言 +
/// 中文标题」时必须先把标题归一化才能锚定修剪。归一化只吸收固定词表的
/// 中文翻译抖动；表外未知标题不改，由 compiler fail-closed。
fn prepare_author_delivery_for_compile(
    raw: &str,
) -> crate::product::work_item_plan_compiler::NormalizedPlanSource {
    let normalized = crate::product::work_item_plan_compiler::normalize_structural_headings(raw);
    crate::product::work_item_plan_compiler::NormalizedPlanSource {
        source: trim_provider_preamble(&normalized.source).to_string(),
        normalized_heading_lines: normalized.normalized_heading_lines,
    }
}

/// SingleCandidate 的单次 markdown author 链路。
///
/// Provider 完整输出直接进入 source revision 与 compiler；内部 selector 只在编译成功后
/// 基于 IR item 数和 provider profile 记录诊断，不触碰 legacy outline/draft/batch 链路。
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
    let language_rules_path = repository.path.join(".claude/rules/language.md");
    let language_rules = match std::fs::read_to_string(&language_rules_path) {
        Ok(rules) => rules,
        Err(error) => {
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::Message(format!(
                "load required SingleCandidate language rules failed for repository {} at {}: {error}",
                repository.path.display(),
                language_rules_path.display(),
            )));
        }
    };
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
    let design_context_blocks =
        crate::product::work_item_split_engine::context::collect_design_context(
            &lifecycle, &request, &issue,
        )
        .map_err(|error| {
            SingleCandidateProviderRunError::Message(format!(
                "load design context failed: {}",
                error.message
            ))
        })?;
    let design_requirement_ids =
        crate::product::work_item_split_engine::context::extract_design_requirement_ids(
            &design_context_blocks,
        );
    let design_context = design_context_blocks.join("\n\n");
    let repository_structure =
        crate::product::work_item_split_engine::context::summarize_repository_structure(
            &repository.path,
        );
    let author_provider = engine.session().author_provider.clone();
    let launch = resolve_plan_author_launch(
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
                design_requirement_ids: &design_requirement_ids,
                repository_structure: &repository_structure,
                language_rules: &language_rules,
                routing_context: &launch.routing_context(),
            },
        )
        .map_err(SingleCandidateProviderRunError::Message)?;
    let node_id = if engine.active_node_type()
        == Some(crate::web::workspace_ws_types::TimelineNodeType::AuthorRun)
    {
        engine.active_timeline_node_id().ok_or_else(|| {
            SingleCandidateProviderRunError::Message(
                "single candidate author run node unavailable".to_string(),
            )
        })?
    } else {
        engine.begin_work_item_plan_author_run().await
    };

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
        full_prompt.clone(),
        repository.path.to_string_lossy().to_string(),
        author_provider.clone(),
    );
    let provider_session =
        start_work_item_plan_author(launch, provider_for_run, provider_input, run_cancel).await;
    let full_output = match engine
        .drive_work_item_plan_provider_session_to_output(
            provider_session,
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
    let delivery = prepare_author_delivery_for_compile(&full_output);
    if delivery.normalized_heading_lines > 0 {
        tracing::info!(
            session_id = %engine.session().session_id,
            node_id = %node_id,
            diagnostic = crate::product::work_item_plan_compiler::PLAN_HEADING_NORMALIZATION_DIAGNOSTIC,
            normalized_heading_lines = delivery.normalized_heading_lines,
            "single-candidate author markdown 结构标题已确定性归一化后再编译"
        );
        engine
            .emit_execution_event(
                ProviderExecutionEvent {
                    event_id: format!("single_candidate_heading_normalized_{node_id}"),
                    kind: ProviderExecutionEventKind::Provider,
                    status: ProviderExecutionEventStatus::Completed,
                    title: "SingleCandidate 结构标题确定性归一化".to_string(),
                    detail: Some(format!(
                        "normalized {} structural heading lines via the fixed zh→en table before compile",
                        delivery.normalized_heading_lines
                    )),
                    command: None,
                    cwd: None,
                    output: None,
                    exit_code: None,
                },
                Some(node_id.clone()),
                Some(author_provider.clone()),
            )
            .await;
    }
    let full_output = delivery.source;
    let candidate_item_count = match engine
        .complete_single_candidate_work_item_plan_author(full_output, repository.id)
        .await
    {
        Ok(candidate_item_count) => candidate_item_count,
        Err(message) => {
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::Message(message));
        }
    };
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

    let decision_input = crate::product::workspace_engine::SingleCandidateGenerationDecisionInput {
        provider: author_provider.clone(),
        candidate_item_count,
    };
    let generation_mode =
        crate::product::workspace_engine::select_internal_generation_mode(&decision_input);
    #[cfg(test)]
    super::record_single_candidate_generation_step(&engine.session().session_id, "selector");
    let generation_diagnostic = format!(
        "internal generation mode={generation_mode:?}; provider={:?}; compiled_item_count={}",
        decision_input.provider, decision_input.candidate_item_count,
    );
    tracing::info!(
        session_id = %engine.session().session_id,
        provider = ?decision_input.provider,
        compiled_item_count = decision_input.candidate_item_count,
        generation_mode = ?generation_mode,
        "single-candidate internal generation mode diagnosed after compilation"
    );
    engine
        .emit_execution_event(
            ProviderExecutionEvent {
                event_id: format!("single_candidate_generation_mode_{node_id}"),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Completed,
                title: "SingleCandidate 内部生成模式已诊断".to_string(),
                detail: Some(generation_diagnostic),
                command: None,
                cwd: None,
                output: None,
                exit_code: None,
            },
            Some(node_id),
            Some(author_provider),
        )
        .await;
    Ok(SingleCandidateProviderRunOutcome::Completed)
}

#[cfg(test)]
mod tests {
    use super::{prepare_author_delivery_for_compile, trim_provider_preamble};
    use crate::product::work_item_plan_compiler::WorkItemPlanSourceContext;

    const FIELD_REP1: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/field-pi-zh-headings-rep1.md"
    ));

    #[test]
    fn trims_provider_preamble_before_document_heading() {
        let source = "我会先读取上下文，再生成计划。\n\n# Work Item Plan\n## Work Item WI-001: x\n";

        assert_eq!(
            trim_provider_preamble(source),
            "# Work Item Plan\n## Work Item WI-001: x\n"
        );
    }

    #[test]
    fn trims_glued_preamble_before_document_heading() {
        let source = "我会先读取上下文，再生成计划。# Work Item Plan\n## Work Item WI-001: x\n";

        assert_eq!(
            trim_provider_preamble(source),
            "# Work Item Plan\n## Work Item WI-001: x\n"
        );
    }

    #[test]
    fn leaves_source_without_preamble_unchanged() {
        let source = "# Work Item Plan\n## Work Item WI-001: x\n";

        assert_eq!(trim_provider_preamble(source), source);
    }

    #[test]
    fn leaves_source_without_document_heading_unchanged() {
        let source = "我会先读取上下文，再生成计划。\n## Work Item WI-001: x\n";

        assert_eq!(trim_provider_preamble(source), source);
    }

    #[test]
    fn trims_code_fence_before_document_heading() {
        let source = "```markdown\n# Work Item Plan\n## Work Item WI-001: x\n```\n";

        assert_eq!(
            trim_provider_preamble(source),
            "# Work Item Plan\n## Work Item WI-001: x\n```\n"
        );
    }

    fn compile_item_count(source: &str) -> Result<usize, String> {
        crate::product::work_item_plan_compiler::compile_work_item_plan(
            source,
            &WorkItemPlanSourceContext {
                target_repository_id: "repository_0001".to_string(),
            },
        )
        .map(|ir| ir.items.len())
        .map_err(|diagnostics| {
            diagnostics
                .first()
                .map(|diagnostic| diagnostic.message.clone())
                .unwrap_or_default()
        })
    }

    #[test]
    fn author_delivery_normalizes_field_chinese_headings_and_compiles() {
        // 现场 pi-full rep1 原件 + 前言:author 链路净化后必须可编译。
        let raw = format!("provider preamble\n{FIELD_REP1}");

        let delivery = prepare_author_delivery_for_compile(&raw);

        assert_eq!(delivery.normalized_heading_lines, 15);
        assert!(delivery.source.starts_with("# Work Item Plan\n"));
        assert_eq!(
            compile_item_count(&delivery.source).expect("归一化后必须编译通过"),
            1
        );
    }

    #[test]
    fn author_delivery_leaves_english_source_unchanged() {
        let source =
            "# Work Item Plan\n## Work Item WI-001: x\n### Identity\n- schema_version: 1\n";

        let delivery = prepare_author_delivery_for_compile(source);

        assert_eq!(delivery.normalized_heading_lines, 0);
        assert_eq!(delivery.source, source);
    }

    #[test]
    fn author_delivery_keeps_unknown_headings_fail_closed() {
        // 表外中文标题不改写,仍由 compiler fail-closed 拒绝。
        let source = "# 工作项计划\n## 工作项 WI-001: x\n### 溯源清单\n";

        let delivery = prepare_author_delivery_for_compile(source);

        assert_eq!(delivery.normalized_heading_lines, 2);
        assert!(delivery.source.contains("### 溯源清单\n"));
        assert!(compile_item_count(&delivery.source).is_err());
    }
}
