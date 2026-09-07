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
/// SingleCandidate author 归一化审计事件的稳定 event_id。
///
/// execution event 按 event_id upsert:同一 AuthorRun 节点被重驱(中断恢复、
/// 重试)时若 event_id 只含 node_id,后一次归一化会覆盖前一次的审计记录。
/// 以 provider 原文内容摘要为去重键(与修订链 report id 的 `source_hash[..16]`
/// 模式一致):不同尝试内容各留一条审计,同内容重放保持同 ID 幂等去重。
fn author_heading_normalized_event_id(node_id: &str, raw_output: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = hex::encode(Sha256::digest(raw_output.as_bytes()));
    format!(
        "single_candidate_heading_normalized_{node_id}_{}",
        &digest[..16]
    )
}

/// 归一化审计事件发射（首轮与 F2-B 教学重驱轮共用）：归一化行数>0 时才发。
async fn emit_author_heading_normalized_event(
    engine: &mut WorkspaceEngine,
    node_id: &str,
    raw_output: &str,
    delivery: &crate::product::work_item_plan_compiler::NormalizedPlanSource,
    author_provider: &ProviderName,
) {
    if delivery.normalized_heading_lines == 0 {
        return;
    }
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
                event_id: author_heading_normalized_event_id(node_id, raw_output),
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
            Some(node_id.to_string()),
            Some(author_provider.clone()),
        )
        .await;
}

/// compile 失败原因原文（与 single_candidate.rs `format_compiler_diagnostics`
/// 同源的 code:line:message 形态，供教学重驱 prompt 回灌与终态错误拼装）。
fn format_compile_failure_reasons(
    diagnostics: &[crate::product::work_item_plan_compiler::CompilerDiagnostic],
) -> Vec<String> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{}:{}:{}",
                diagnostic.code, diagnostic.line, diagnostic.message
            )
        })
        .collect()
}

/// 3.6 弱模型基线加固：IR 预校验——在持久化前跑与
/// `complete_single_candidate_work_item_plan_author` 内部同源的
/// `validate_plan_candidate_ir`（上下文组装同源：plan 的 source spec ids 与
/// repository profile），使 IR 校验失败（unknown_requirement_ref /
/// acceptance_criterion_without_reviewer_check 等）可在 handler 分类并享有
/// 恰一次教学重驱。仅当预校验确定性失败时返回 Some(reasons)；上下文装载
/// 失败返回 None（不阻断流程），由 complete_... 权威路径以既有终态形态兜底。
fn prevalidate_plan_candidate_ir(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
    request: &crate::web::types::GenerateWorkItemsRequest,
    ir: &crate::product::work_item_plan_compiler::PlanCandidateIr,
) -> Option<Vec<String>> {
    let session = engine.session();
    let plan = lifecycle
        .get_issue_work_item_plan(&session.project_id, &session.issue_id, &session.entity_id)
        .ok()?;
    let repository_profile = plan
        .repository_profile_ref
        .as_deref()
        .and_then(|profile_id| {
            lifecycle
                .get_repository_profile(&session.project_id, &session.issue_id, profile_id)
                .ok()
        });
    let validation_now = chrono::Utc::now().to_rfc3339();
    let validation = crate::product::work_item_plan_compiler::validate_plan_candidate_ir(
        ir,
        &crate::product::work_item_plan_compiler::PlanCandidateValidationContext {
            project_id: &session.project_id,
            issue_id: &session.issue_id,
            plan_id: &session.entity_id,
            source_story_spec_ids: &request.story_spec_ids,
            source_design_spec_ids: &request.design_spec_ids,
            repository_profile: repository_profile.as_ref(),
            now: &validation_now,
        },
    );
    match validation {
        Ok(_) => None,
        Err(diagnostics) => Some(format_compile_failure_reasons(&diagnostics)),
    }
}

/// 3.6 弱模型基线加固：IR 校验失败的教学重驱 prompt。复用 F2-B 既有 compile
/// 重驱模板（错误原文已逐条回灌 + 立即输出完整 source 指令），附加「修正引用/
/// 补齐字段后重新输出完整 plan」的 IR 修复指令。
fn build_work_item_plan_ir_reredrive_prompt(blocking_reasons: &[String]) -> String {
    let mut prompt =
        crate::product::workspace_engine::build_work_item_plan_compile_reredrive_prompt(
            blocking_reasons,
        );
    prompt.push_str(
        "上述为 IR 校验失败（引用或字段不符合契约）。\n\
         修正引用/补齐字段：requirement_refs、done_when_refs、reviewer_check_refs 只能逐字引用本计划已定义 id；\
         每个 criterion_id 必须有配对的 reviewer_check_refs 行。\n\
         修正后重新输出完整 plan，第一行即 `# Work Item Plan`，不要输出解释。\n",
    );
    prompt
}

/// F2-B/3.6：教学重驱共用驱动——同 node 再驱一次 provider，返回归一化后的
/// markdown source。驱动中断（会话失败/取消）时已按既有终态形态持久化 Failed
/// 并返回 AlreadyFinished，与首轮驱动失败处理一致。
#[allow(clippy::too_many_arguments)]
async fn drive_single_candidate_reredrive(
    engine: &mut WorkspaceEngine,
    launch: &crate::web::workspace_ws_handler::run::gateway_start::PlanAuthorLaunch,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    run_cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    node_id: &str,
    author_provider: &crate::product::models::ProviderName,
    reredrive_prompt: &str,
    repository_path: &std::path::Path,
) -> Result<String, SingleCandidateProviderRunError> {
    let reredrive_input = engine.build_work_item_plan_streaming_input(
        crate::product::work_item_split_engine::types::provider_name_to_type(author_provider),
        reredrive_prompt.to_string(),
        repository_path.to_string_lossy().to_string(),
        author_provider.clone(),
    );
    let reredrive_input = engine.attach_tool_policy_audit(reredrive_input);
    let reredrive_session = start_work_item_plan_author(
        launch.clone(),
        Arc::clone(&provider_for_run),
        reredrive_input,
        run_cancel.clone(),
    )
    .await;
    let reredrive_output = match engine
        .drive_work_item_plan_provider_session_to_output(
            reredrive_session,
            command_rx,
            node_id.to_string(),
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
    let reredrive_delivery = prepare_author_delivery_for_compile(&reredrive_output);
    emit_author_heading_normalized_event(
        engine,
        node_id,
        &reredrive_output,
        &reredrive_delivery,
        author_provider,
    )
    .await;
    Ok(reredrive_delivery.source)
}

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
    // F5-A：SC 修订轮 findings 回灌——最近 verdict 要求返修时，本轮 author 重跑
    // 使用返修 prompt（在首轮完整 prompt 的尾部输出指令前注入 reviewer findings 与
    // 硬性修复指令）；首轮（无 verdict）prompt 逐字节不变。
    let revision_verdict = engine.single_candidate_pending_revision_verdict();
    let full_prompt = match revision_verdict.as_ref() {
        Some(review) => {
            crate::product::work_item_split_engine::prompts::
                build_work_item_plan_markdown_revision_prompt(
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
                    review,
                )
        }
        None => crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
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
        ),
    }
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
    let prompt_event_detail = if revision_verdict.is_some() {
        "发送给 SingleCandidate markdown author 的返修提示词（已回灌 reviewer findings）"
    } else {
        "发送给 SingleCandidate markdown author 的完整提示词"
    };
    engine
        .emit_provider_prompt_event(
            &node_id,
            full_prompt.clone(),
            prompt_event_detail,
            Some(author_provider.clone()),
        )
        .await;
    let provider_input = engine.build_work_item_plan_streaming_input(
        crate::product::work_item_split_engine::types::provider_name_to_type(&author_provider),
        full_prompt.clone(),
        repository.path.to_string_lossy().to_string(),
        author_provider.clone(),
    );
    let provider_input = engine.attach_tool_policy_audit(provider_input);
    let provider_session = start_work_item_plan_author(
        launch.clone(),
        Arc::clone(&provider_for_run),
        provider_input,
        run_cancel.clone(),
    )
    .await;
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
    emit_author_heading_normalized_event(
        engine,
        &node_id,
        &full_output,
        &delivery,
        &author_provider,
    )
    .await;
    let full_output = delivery.source;
    // F2-B：SC compile 失败教学重驱（同 candidate 恰一次）。missing_section 类
    // compile 失败不再直接终态：先在 handler 预编译分类（compile 为纯函数，与
    // complete_... 内部编译同源同果），命中则同 node 发送教学重驱 prompt（含
    // compile 错误原文）再驱一次 provider；重驱仍败→维持终态失败（错误含两轮
    // 信息）；非 missing_section 错误不触发重驱，保持既有终态错误形态。
    // 3.6 弱模型基线加固：IR 校验失败（validate plan candidate IR failed 类，
    // 如 unknown_requirement_ref / acceptance_criterion_without_reviewer_check）
    // 同样享有恰一次教学重驱——在持久化前预跑同源 validate_plan_candidate_ir
    // 分类，重驱 prompt 附错误原文与修正引用/补齐字段指令；重驱再败→终态含
    // 两轮；非 IR/非 missing_section 的其他失败（如内部错误）不触发重驱。
    // 重驱机会在 compile/IR 两类间共享（first_round_failure 单槽）：每 candidate
    // 至多一次额外 provider 驱动。
    let compile_context = crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
        target_repository_id: repository.id.clone(),
    };
    let mut compile_source = full_output;
    let mut first_round_failure: Option<String> = None;
    let candidate_item_count = loop {
        match crate::product::work_item_plan_compiler::compile_work_item_plan(
            &compile_source,
            &compile_context,
        ) {
            Ok(ir) => {
                // 3.6：持久化前 IR 预校验——与 complete_... 内部同源同果；仅确定性
                // IR 校验失败才触发重驱/终态，装载失败交给权威路径兜底。
                if let Some(reasons) =
                    prevalidate_plan_candidate_ir(engine, &lifecycle, &request, &ir)
                {
                    if first_round_failure.is_none() {
                        first_round_failure = Some(reasons.join("; "));
                        let reredrive_prompt = build_work_item_plan_ir_reredrive_prompt(&reasons);
                        engine
                            .emit_execution_event(
                                ProviderExecutionEvent {
                                    event_id: format!("{node_id}_prompt_ir_reredrive"),
                                    kind: ProviderExecutionEventKind::Output,
                                    status: ProviderExecutionEventStatus::Started,
                                    title: "SC IR 校验失败教学重驱提示词".to_string(),
                                    detail: Some(
                                        "IR 校验失败（unknown_requirement_ref / \
                                         acceptance_criterion_without_reviewer_check 等）的\
                                         教学重驱（含错误原文与修正指令），恰一次"
                                            .to_string(),
                                    ),
                                    command: None,
                                    cwd: None,
                                    output: Some(reredrive_prompt.clone()),
                                    exit_code: None,
                                },
                                Some(node_id.clone()),
                                Some(author_provider.clone()),
                            )
                            .await;
                        compile_source = drive_single_candidate_reredrive(
                            engine,
                            &launch,
                            Arc::clone(&provider_for_run),
                            &run_cancel,
                            command_rx,
                            &node_id,
                            &author_provider,
                            &reredrive_prompt,
                            &repository.path,
                        )
                        .await?;
                        continue;
                    }
                    let detail = reasons.join("; ");
                    let message = match first_round_failure.as_deref() {
                        Some(first_round) => format!(
                            "validate plan candidate IR failed (after one teaching re-drive): \
                             first round: {first_round}; re-drive round: {detail}"
                        ),
                        None => format!("validate plan candidate IR failed: {detail}"),
                    };
                    engine.persist_single_candidate_terminal_phase(
                        crate::product::models::SingleCandidatePhase::Failed,
                    );
                    return Err(SingleCandidateProviderRunError::Message(message));
                }
                break match engine
                    .complete_single_candidate_work_item_plan_author(
                        compile_source,
                        repository.id.clone(),
                    )
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
            }
            Err(diagnostics) => {
                let reasons = format_compile_failure_reasons(&diagnostics);
                if first_round_failure.is_none()
                    && diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == "missing_section")
                {
                    first_round_failure = Some(reasons.join("; "));
                    let reredrive_prompt =
                        crate::product::workspace_engine::build_work_item_plan_compile_reredrive_prompt(
                            &reasons,
                        );
                    engine
                        .emit_execution_event(
                            ProviderExecutionEvent {
                                event_id: format!("{node_id}_prompt_compile_reredrive"),
                                kind: ProviderExecutionEventKind::Output,
                                status: ProviderExecutionEventStatus::Started,
                                title: "SC compile 失败教学重驱提示词".to_string(),
                                detail: Some(
                                    "missing_section 类 compile 失败的教学重驱（含 compile 错误原文），恰一次"
                                        .to_string(),
                                ),
                                command: None,
                                cwd: None,
                                output: Some(reredrive_prompt.clone()),
                                exit_code: None,
                            },
                            Some(node_id.clone()),
                            Some(author_provider.clone()),
                        )
                        .await;
                    // 3.6：抽共用驱动 helper（与 IR 重驱同源），行为与原内联块一致。
                    compile_source = drive_single_candidate_reredrive(
                        engine,
                        &launch,
                        Arc::clone(&provider_for_run),
                        &run_cancel,
                        command_rx,
                        &node_id,
                        &author_provider,
                        &reredrive_prompt,
                        &repository.path,
                    )
                    .await?;
                    continue;
                }
                let detail = reasons.join("; ");
                let message = match first_round_failure {
                    Some(first_round) => format!(
                        "compile markdown source failed (after one teaching re-drive): \
                         first round: {first_round}; re-drive round: {detail}"
                    ),
                    None => format!("compile markdown source failed: {detail}"),
                };
                engine.persist_single_candidate_terminal_phase(
                    crate::product::models::SingleCandidatePhase::Failed,
                );
                return Err(SingleCandidateProviderRunError::Message(message));
            }
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

    #[test]
    fn author_heading_normalized_event_id_keeps_attempt_history_on_same_node() {
        // 同一 AuthorRun 节点被重驱(中断恢复/重试)时,两次不同尝试的归一化
        // 审计不得共用 event_id——execution event 按 event_id upsert,共用会让
        // 后一次覆盖前一次,丢审计历史。
        let node_id = "timeline_node_001";
        let first = super::author_heading_normalized_event_id(node_id, "attempt one output");
        let second = super::author_heading_normalized_event_id(node_id, "attempt two output");

        assert_ne!(
            first, second,
            "同节点不同尝试共用 event_id 会让 upsert 覆盖审计历史"
        );
        assert_eq!(
            first,
            super::author_heading_normalized_event_id(node_id, "attempt one output"),
            "同内容重放必须保持 event_id 稳定以幂等去重"
        );
        assert!(first.starts_with("single_candidate_heading_normalized_timeline_node_001"));
    }
}
