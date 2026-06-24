use super::*;

impl CodingWorkspaceEngine {
    pub(crate) async fn build_code_review_prompt(
        &self,
        attempt: &CodingExecutionAttempt,
        worktree_path: &Path,
        retry_diagnostic: Option<&str>,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let diff = self
            ._git_service
            .git_diff(worktree_path, &attempt.base_branch)
            .await?;
        let work_item = self.work_item_markdown_for_attempt(attempt)?;
        let evaluation_context_json =
            self.evaluation_context_json_for_role(attempt, EvaluationContextRole::CodeReviewer)?;
        let retry_diagnostic_section = retry_diagnostic
            .map(|summary| format!("\n上一轮 role run 诊断摘要:\n{}\n", summary))
            .unwrap_or_default();
        Ok(format!(
            "Coding Workspace CodeReviewer\n\
             {}\n\
             你是 CodeReviewer，只分析当前变更 diff，不修改代码、不执行写操作。\n\
             Project: {}\n\
             Issue: {}\n\
             Work Item: {}\n\
             Attempt: {}\n\
             Branch: {}\n\
             Base: {}\n\
             \n代码规范:\n\
             - 优先检查正确性、边界条件、测试覆盖、安全、性能和可维护性。\n\
             - findings 必须包含 severity、file_path、line、message、required_action、source_stage=code_review。\n\
             - 如果没有阻塞问题，verdict 使用 approve。\n\
             \n原始需求上下文:\n````markdown\n{}\n````\n\
             \nEvaluationContextPack:\n````json\n{}\n````\n\
             \ngit diff:\n````diff\n{}\n````\n\
             {}\
             \n只输出 JSON：{{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[...]}}\n",
            provider_runtime_contract("CodeReviewer"),
            attempt.project_id,
            attempt.issue_id,
            attempt.work_item_id,
            attempt.id,
            attempt.branch_name,
            attempt.base_branch,
            work_item.unwrap_or_else(
                || "未找到 Work Item markdown，上下文仅包含 attempt 元数据。".to_string()
            ),
            evaluation_context_json,
            truncate_prompt_section(&diff, 30_000),
            retry_diagnostic_section
        ))
    }

    pub(crate) async fn build_internal_pr_review_prompt(
        &self,
        attempt: &CodingExecutionAttempt,
        review_request: &ReviewRequest,
        worktree_path: &Path,
        retry_diagnostic: Option<&str>,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let diff = self
            ._git_service
            .git_diff(worktree_path, &attempt.base_branch)
            .await?;
        let work_item = self.work_item_markdown_for_attempt(attempt)?;
        let evaluation_context_json = self
            .evaluation_context_json_for_role(attempt, EvaluationContextRole::InternalReviewer)?;
        let retry_diagnostic_section = retry_diagnostic
            .map(|summary| format!("\n上一轮 role run 诊断摘要:\n{}\n", summary))
            .unwrap_or_default();
        Ok(format!(
            "Coding Workspace InternalReviewer\n\
             {}\n\
             你是 InternalReviewer，在 ReviewRequest(push) 之后做内部 PR 审查。\n\
             Project: {}\n\
             Issue: {}\n\
             Work Item: {}\n\
             Attempt: {}\n\
             Branch: {}\n\
             Review Request: {}\n\
             Review Remote: {}\n\
             Commit: {}\n\
             \n功能需求上下文:\n````markdown\n{}\n````\n\
             \nEvaluationContextPack:\n````json\n{}\n````\n\
             \n完整变更 git diff:\n````diff\n{}\n````\n\
             {}\
             \n输出要求:\n\
             - 分析影响范围（影响范围/impact_scope）。\n\
             - 给出 PR description 预览。\n\
             - 给出 commit message 建议。\n\
             - findings 必须包含 source_stage=internal_pr_review。\n\
             \n只输出 JSON：{{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[...],\"impact_scope\":[\"...\"],\"pr_description\":\"...\",\"commit_message_suggestion\":\"...\"}}\n",
            provider_runtime_contract("InternalReviewer"),
            attempt.project_id,
            attempt.issue_id,
            attempt.work_item_id,
            attempt.id,
            attempt.branch_name,
            review_request.id,
            review_request.remote,
            review_request.commit_sha,
            work_item.unwrap_or_else(
                || "未找到 Work Item markdown，上下文仅包含 attempt 元数据。".to_string()
            ),
            evaluation_context_json,
            truncate_prompt_section(&diff, 30_000),
            retry_diagnostic_section
        ))
    }
}

pub(crate) fn build_coding_prompt(
    attempt: &CodingExecutionAttempt,
    context: &CodingExecutionContext,
    rework_instruction: Option<&CodingReworkInstruction>,
    context_notes: Option<&ReworkContextNoteInput>,
) -> String {
    let mut prompt = format!(
        "Coding Workspace\n\
         你是 Coding Workspace author。请在指定 worktree 中完成真实代码修改和测试，不要只输出计划或 Story/Design/Work Item 文档。\n\
         Project: {}\n\
         Issue: {}\n\
         Work Item: {}\n\
         Attempt: {}\n\
         Branch: {}\n",
        attempt.project_id, attempt.issue_id, attempt.work_item_id, attempt.id, attempt.branch_name
    );
    if let Some(worktree_path) = attempt.worktree_path.as_ref() {
        prompt.push_str(&format!("Worktree Path: {}\n", worktree_path.display()));
    }
    if !context.verification_commands.is_empty() {
        prompt.push_str("\n验证命令:\n");
        for command in &context.verification_commands {
            prompt.push_str("- ");
            prompt.push_str(command);
            prompt.push('\n');
        }
    }

    if let Some(markdown) = context.work_item_markdown.as_deref() {
        prompt.push_str("\n已确认 Work Item:\n````markdown\n");
        prompt.push_str(markdown.trim());
        prompt.push_str("\n````\n");
    }
    if let Some(instruction) = rework_instruction {
        prompt.push_str("\n上一轮返修要求:\n");
        prompt.push_str(&format!(
            "- 来源阶段: {:?}\n- 摘要: {}\n",
            instruction.source_stage, instruction.summary
        ));
        if !instruction.fix_hints.is_empty() {
            prompt.push_str("- 修复提示:\n");
            for (index, hint) in instruction.fix_hints.iter().enumerate() {
                prompt.push_str(&format!("  {}. {}\n", index + 1, hint));
            }
        }
        if !instruction.questions.is_empty() {
            prompt.push_str("- 待澄清问题:\n");
            for (index, question) in instruction.questions.iter().enumerate() {
                prompt.push_str(&format!("  {}. {}\n", index + 1, question));
            }
        }
        prompt.push_str(
            "\n本轮必须优先修复上述问题。完成前请检查 git diff/status，确认 reviewer 指出的文件或行为已处理。\n",
        );
    }
    append_coding_context_notes(&mut prompt, context_notes);
    prompt.push_str(dependency_bootstrap_guidance());
    prompt.push_str(
        "\n执行要求:\n\
         - 遵循仓库规则和 TDD 流程。\n\
         - 优先按已确认 Work Item 的文件落点、范围和验证命令执行。\n\
         - 完成后报告修改文件、测试命令和结果。\n",
    );
    prompt
}

pub(crate) fn build_coding_delta_prompt(
    attempt: &CodingExecutionAttempt,
    context: &CodingExecutionContext,
    rework_instruction: Option<&CodingReworkInstruction>,
    context_notes: Option<&ReworkContextNoteInput>,
) -> String {
    let mut prompt = format!(
        "Coding Workspace\n\
         你是 Coding Workspace Coder。请继续在指定 worktree 中完成真实代码修改和测试，不要只输出计划。\n\
         Project: {}\n\
         Issue: {}\n\
         Work Item: {}\n\
         Attempt: {}\n\
         Branch: {}\n",
        attempt.project_id, attempt.issue_id, attempt.work_item_id, attempt.id, attempt.branch_name
    );
    if let Some(worktree_path) = attempt.worktree_path.as_ref() {
        prompt.push_str(&format!("Worktree Path: {}\n", worktree_path.display()));
    }
    prompt.push_str(
        "\n这是对当前 provider 会话的增量代码编写指令。不要重新发送或复述完整 Work Item；请基于本会话已有上下文、当前 worktree 状态和以下新增要求，直接继续修改代码。\n",
    );
    if !context.verification_commands.is_empty() {
        prompt.push_str("\n验证命令:\n");
        for command in &context.verification_commands {
            prompt.push_str("- ");
            prompt.push_str(command);
            prompt.push('\n');
        }
    }

    if let Some(instruction) = rework_instruction {
        prompt.push_str("\n本轮返修要求:\n");
        prompt.push_str(&format!(
            "- 来源阶段: {:?}\n- 摘要: {}\n",
            instruction.source_stage, instruction.summary
        ));
        if !instruction.fix_hints.is_empty() {
            prompt.push_str("- 修复提示:\n");
            for (index, hint) in instruction.fix_hints.iter().enumerate() {
                prompt.push_str(&format!("  {}. {}\n", index + 1, hint));
            }
        }
        if !instruction.questions.is_empty() {
            prompt.push_str("- 待澄清问题:\n");
            for (index, question) in instruction.questions.iter().enumerate() {
                prompt.push_str(&format!("  {}. {}\n", index + 1, question));
            }
        }
        prompt.push_str(
            "\n本轮必须优先修复上述问题。完成前请检查 git diff/status，确认 reviewer 指出的文件或行为已处理。\n",
        );
    } else {
        prompt.push_str(
            "\n本轮没有新增返修要求。请基于当前会话和 worktree 状态继续完成未结束的代码编写任务。\n",
        );
    }
    append_coding_context_notes(&mut prompt, context_notes);
    prompt.push_str(dependency_bootstrap_guidance());
    prompt.push_str(
        "\n执行要求:\n\
         - 遵循仓库规则和 TDD 流程。\n\
         - 不要重新生成 Story/Design/Work Item 文档。\n\
         - 完成后报告修改文件、测试命令和结果。\n",
    );
    prompt
}

pub(crate) fn append_coding_context_notes(
    prompt: &mut String,
    context_notes: Option<&ReworkContextNoteInput>,
) {
    let Some(context_notes) = context_notes else {
        return;
    };
    if context_notes.text.trim().is_empty() || context_notes.text.trim() == "无" {
        return;
    }
    prompt.push_str("\n本轮补充上下文:\n");
    prompt.push_str(&format!(
        "ContextNotes Truncated: {}\n{}\n",
        context_notes.truncated, context_notes.text
    ));
    prompt.push_str(
        "请将这些人工补充要求与本轮返修要求一起执行；如有冲突，优先遵循更具体的人工补充上下文。\n",
    );
}

pub(crate) fn dependency_bootstrap_guidance() -> &'static str {
    "\n依赖初始化诊断要求:\n\
     - 如果前端命令出现 `Local package.json exists, but node_modules missing`、`tsc EACCES`、`vitest EACCES`、`Permission denied` 或 `spawn ... EACCES`，先不要判定 pnpm 环境不可用。\n\
     - 先运行 `pnpm --version` 区分 pnpm 是否存在；只有该命令失败时，才报告 pnpm 不可用。\n\
     - 如果 pnpm 可用且对应 package 目录存在 lockfile，请先运行 `pnpm -C <package-dir> install --frozen-lockfile`，例如 Aria 前端为 `pnpm -C web install --frozen-lockfile`，然后重试 build/test。\n\
     - 不要把缺少 node_modules 误判为 pnpm 不可用。\n"
}

pub(crate) fn build_rework_prompt(
    attempt: &CodingExecutionAttempt,
    evidence: &str,
    source_stage: &CodingExecutionStage,
    rework_round: u32,
    context_notes: &ReworkContextNoteInput,
    evaluation_context_json: &str,
    retry_diagnostic: Option<&str>,
) -> String {
    let retry_diagnostic_section = retry_diagnostic
        .map(|summary| format!("\n上一轮 Analyst role run 诊断摘要:\n{}\n", summary))
        .unwrap_or_default();
    format!(
        "CRITICAL: Return ONLY a single JSON object. No markdown, no explanations, no validation reports, no tables.\n\
         Coding Workspace Rework 分析官\n\
         {}\n\
         你是 Coding Workspace Rework 分析官，只做分析和路由决策。\n\
         严格要求：不要修改代码，不要调用 tool_use，不要执行命令。\n\
         仅根据上一阶段 summary/evidence、本轮新增 ContextNote 与 EvaluationContextPack 输出 AnalystDecision JSON。\n\
         JSON 必须以 {{ 开头，以 }} 结尾。\n\
         JSON 格式：{{\"verdict\":\"needs_fix|rerun_testing|proceed|human_required|blocked\",\"next_stage\":\"coding|testing|code_review|review_request|internal_pr_review|final_confirm|human_gate\",\"reason\":\"...\",\"evidence_refs\":[\"...\"],\"raw_provider_output_refs\":[\"...\"],\"rework_instructions\":null,\"human_gate\":null}}\n\
         路由规则：TestingReport 因 test_plan_missing_json、test_plan_invalid_json 或 test_plan_repair_failed 阻塞时，优先判断为 Tester 输出契约问题；若可重试，输出 verdict=rerun_testing,next_stage=testing；只有环境、权限或需求缺失不可自动处理时才 next_stage=human_gate。\n\
         Project: {}\n\
         Issue: {}\n\
         Work Item: {}\n\
         Attempt: {}\n\
         Branch: {}\n\
         Previous Stage: {:?}\n\
         Rework Round: {}\n\
         ContextNotes Truncated: {}\n\
         \n上一阶段 summary/evidence:\n{}\n\
         \n本轮新增 ContextNote:\n{}\n\
         \nEvaluationContextPack:\n````json\n{}\n````\n\
         {}\
         \nCRITICAL: Return ONLY a single JSON object. Do not summarize validation. Do not include markdown.\n\
         END OF INSTRUCTIONS: output JSON only.",
        provider_runtime_contract("Analyst"),
        attempt.project_id,
        attempt.issue_id,
        attempt.work_item_id,
        attempt.id,
        attempt.branch_name,
        source_stage,
        rework_round,
        context_notes.truncated,
        evidence,
        context_notes.text,
        evaluation_context_json,
        retry_diagnostic_section
    )
}

pub(crate) fn provider_runtime_contract(role: &str) -> String {
    format!(
        "[openspec_contract]\n\
         Role: {role}\n\
         - 使用 Story Spec、Design Spec、Work Item 的追踪关系做判断。\n\
         - 发现 Story Spec、Design Spec、Work Item、diff 或实现之间冲突时，必须 blocked 或请求人工澄清。\n\
         - 不得忽略需求、设计、任务之间的证据链。\n\
         \n\
         [superpowers_contract]\n\
         - 先证据后结论。\n\
         - 验证前置；结论必须能追溯到已执行检查或明确证据。\n\
         - 不用未执行推断替代证据。\n"
    )
}

pub(crate) fn provider_prompt_event(
    node_id: &str,
    provider: &ProviderName,
    prompt: String,
    detail: &str,
) -> WsExecutionEvent {
    WsExecutionEvent {
        event_id: format!("{node_id}_prompt"),
        node_id: Some(node_id.to_string()),
        agent: Some(provider.clone()),
        kind: WsExecutionEventKind::Output,
        status: WsExecutionEventStatus::Started,
        title: "Provider Prompt".to_string(),
        detail: Some(detail.to_string()),
        command: None,
        cwd: None,
        output: Some(prompt),
        exit_code: None,
    }
}

pub(crate) fn streaming_input_from_adapter(
    input: &AdapterInput,
    working_dir: PathBuf,
) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type: input.provider_type.clone(),
        role: input.role.clone(),
        prompt: input.prompt.clone(),
        working_dir,
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Supervised,
        env_vars: BTreeMap::new(),
        timeout_secs: input.timeout,
    }
}

pub(crate) fn build_tester_execute_plan_prompt(
    attempt: &CodingExecutionAttempt,
    plan: &TestPlan,
    evaluation_context_json: &str,
) -> String {
    let plan_json = serde_json::to_string_pretty(plan).unwrap_or_else(|_| "{}".to_string());
    format!(
        "Tester Provider Runtime\n\
         Phase: execute_test_plan\n\
         Attempt: {}\n\
         Work Item: {}\n\
         \n\
         Execute the following TestPlan. You may execute commands or inspect files yourself.\n\
         Every required TestPlan step must have exactly one corresponding step_results item.\n\
         If you cannot run a required step, emit status=\"blocked\" or status=\"skipped\" with provider_analysis explaining why.\n\
         Do not claim overall success in prose without step_results JSON.\n\
         Tool calls meant to satisfy a plan step must include the exact step_id in their input. Tool calls without step_id are unplanned evidence and cannot satisfy required steps.\n\
         At the end of execute_test_plan, output a JSON object with:\n\
         {{\"step_results\":[{{\"step_id\":\"...\",\"status\":\"passed|failed|blocked|skipped\",\"evidence_refs\":[\"...\"],\"provider_analysis\":\"...\"}}]}}\n\
         \n\
         TestPlan:\n```json\n{}\n```\n\
         \n\
         Evaluation Context JSON:\n```json\n{}\n```\n",
        attempt.id, attempt.work_item_id, plan_json, evaluation_context_json
    )
}

pub(crate) struct ReworkContextNoteInput {
    pub(crate) text: String,
    pub(crate) truncated: bool,
}

pub(crate) fn format_rework_context_notes(
    notes: &[CodingContextNote],
    limit: usize,
) -> ReworkContextNoteInput {
    if notes.is_empty() {
        return ReworkContextNoteInput {
            text: "无".to_string(),
            truncated: false,
        };
    }
    let blocks = notes
        .iter()
        .map(|note| {
            format!(
                "- ContextNote {} ({})\n{}",
                note.id,
                note.created_at,
                note.content.trim()
            )
        })
        .collect::<Vec<_>>();
    let mut remaining = limit;
    let mut selected = Vec::new();
    let mut truncated = false;

    for block in blocks.iter().rev() {
        let block_len = block.chars().count();
        if block_len <= remaining {
            selected.push(block.clone());
            remaining -= block_len;
            continue;
        }

        truncated = true;
        let marker = "[...已截断最早 ContextNote...]\n";
        let marker_len = marker.chars().count();
        if remaining > marker_len {
            let partial = take_last_chars(block, remaining - marker_len);
            selected.push(format!("{marker}{partial}"));
        }
        break;
    }

    if selected.len() < blocks.len() {
        truncated = true;
    }
    selected.reverse();
    let mut text = selected.join("\n");
    if text.chars().count() > limit {
        text = take_last_chars(&text, limit);
        truncated = true;
    }

    ReworkContextNoteInput { text, truncated }
}

pub(crate) fn take_last_chars(value: &str, limit: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let start = chars.len().saturating_sub(limit);
    chars[start..].iter().collect()
}
