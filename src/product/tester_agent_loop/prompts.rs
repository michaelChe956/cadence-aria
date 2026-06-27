use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::coding_workspace_engine::CodingExecutionContext;
use crate::product::test_executor::{TestCommandSpec, infer_test_commands};

use super::tools::detect_changed_files;

pub fn tester_allowed_tools() -> [&'static str; 4] {
    ["run_command", "read_file", "list_files", "search_code"]
}

pub fn build_tester_system_prompt(
    attempt: &CodingExecutionAttempt,
    context: &CodingExecutionContext,
    specs: &[TestCommandSpec],
) -> String {
    let inferred_specs = attempt
        .worktree_path
        .as_ref()
        .map(infer_test_commands)
        .unwrap_or_default();
    let prompt_specs = if specs.is_empty() {
        inferred_specs.as_slice()
    } else {
        specs
    };
    let changed_files = attempt
        .worktree_path
        .as_ref()
        .map(|path| detect_changed_files(path.as_path()))
        .unwrap_or_default();

    let mut prompt = format!(
        "Tester Agent Loop\n\
         你是 Coding Workspace tester。你只能验证和分析，不允许修改源码。\n\
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
    prompt.push_str("\n允许工具:\n");
    for tool in tester_allowed_tools() {
        prompt.push_str("- ");
        prompt.push_str(tool);
        prompt.push('\n');
    }
    prompt.push_str(
        "\n禁止工具:\n\
         - write_file\n\
         - edit_file\n\
         - delete_file\n",
    );
    prompt.push_str("\n可用测试命令:\n");
    if prompt_specs.is_empty() {
        prompt.push_str(
            "- 未推断到测试命令，请先使用 list_files/read_file/search_code 分析项目结构。\n",
        );
    } else {
        for spec in prompt_specs {
            prompt.push_str("- ");
            prompt.push_str(&spec.command.join(" "));
            prompt.push('\n');
        }
    }
    prompt.push_str("\n变更文件:\n");
    if changed_files.is_empty() {
        prompt.push_str("- 未检测到 git 变更文件。\n");
    } else {
        for file in changed_files {
            prompt.push_str("- ");
            prompt.push_str(&file);
            prompt.push('\n');
        }
    }
    if !context.verification_commands.is_empty() {
        prompt.push_str("\nWork Item 验证命令:\n");
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
    prompt.push_str(
        "\n输出要求:\n\
         - 优先调用 run_command 执行测试。\n\
         - 如发现失败，继续收集足够证据并在最终 JSON 中列出 bugs_found。\n\
         - 最终只输出 JSON：{\"summary\":\"...\",\"bugs_found\":[]}。\n",
    );
    prompt
}

pub fn build_tester_plan_prompt(
    attempt: &CodingExecutionAttempt,
    evaluation_context_json: &str,
    retry_diagnostic: Option<&str>,
) -> String {
    let retry_diagnostic_section = retry_diagnostic
        .map(|summary| {
            format!(
                "[retry_diagnostic]\n\
                 以下为上一轮 role run 的压缩诊断摘要，只用于规划本轮测试；不要把这段内容原样放入最终 JSON。\n\
                 过程进度通过 provider events 实时输出，最终回答仍必须是 TestPlan JSON。\n\
                 \n{}\n",
                summary
            )
        })
        .unwrap_or_default();
    format!(
        "CRITICAL: Return ONLY a single JSON object. No markdown, no explanations, no validation reports, no tables.\n\
         Tester Provider Runtime\n\
         Phase: plan_tests -> execute_test_plan\n\
         Project: {}\n\
         Issue: {}\n\
         Work Item: {}\n\
         Attempt: {}\n\
         Branch: {}\n\
         \n\
         [openspec_contract]\n\
         - 依据 Evaluation Context 中的 actual Work Item、Story Spec、Design Spec、diff 与 project rules 设计验证计划。\n\
         - 不要按通用模板生成固定步骤；每个 required 验证步骤都必须服务于实际 Work Item / story / design / diff 变更。\n\
         - 仅允许仓库规则、diff 收集等前置上下文步骤没有业务追踪；其他 required 步骤必须填写 related_requirements、related_design_constraints 或 related_work_item_tasks，优先绑定 TASK/REQ/DEC/AC ID。\n\
         - 如果 Story Spec、Design Spec、Work Item 之间存在冲突，必须 blocked 或请求人工澄清。\n\
         - 先输出 TestPlan JSON，不要直接声称测试通过。\n\
         - 对 Rust 单元测试，定向快反馈只能使用单个过滤词，例如 `cargo test --locked --lib provider_catalog`；禁止生成 `cargo test --locked --lib filter_a filter_b` 或等价单次多个过滤词命令。\n\
         \n\
         [superpowers_contract]\n\
         - 先证据后结论；不要用未执行的推断替代验证证据。\n\
         - 验证前置：执行 execute_test_plan 后，每个 required step 都必须有证据。\n\
         \n\
         工具与 step 绑定:\n\
         - plan_tests 阶段只生成 TestPlan。\n\
         - execute_test_plan 阶段调用 run_command/read_file/list_files/search_code 时必须在 input 中携带 step_id。\n\
         - 无 step_id 的工具结果只能进入 unplanned_commands 或 unplanned evidence，不能满足 required step。\n\
         - 不存在的 step_id 不能满足 required step。\n\
         \n\
         通用项目约束:\n\
         - Aria 是通用项目工作台，不要硬编码某种语言或包管理器。\n\
         - 不要默认 pnpm、cargo、pytest、npm 或任何单一生态；必须从上下文和仓库证据中决策。\n\
         \n\
         输出契约:\n\
         - 只返回一个原始 JSON object；不要输出 Markdown 标题、代码块、表格或验证报告。\n\
         - JSON 必须以 {{ 开头，以 }} 结尾。\n\
         - Required shape: {{\"summary\":\"...\",\"context_warnings\":[],\"assumptions\":[],\"steps\":[{{\"id\":\"...\",\"title\":\"...\",\"intent\":\"...\",\"required\":true,\"tool\":\"run_command|read_file|list_files|search_code|provider_managed\",\"risk_level\":\"low|medium|high\",\"command_or_tool_input\":{{}},\"evidence_expectation\":\"...\",\"related_requirements\":[\"REQ-...\"],\"related_design_constraints\":[\"DEC-...\"],\"related_work_item_tasks\":[\"TASK-...\"]}}]}}\n\
         \n\
         Evaluation Context JSON:\n\
         ```json\n{}\n```\n\
         \n\
         {}\
         \n\
         CRITICAL: Return ONLY a single JSON object. Do not summarize validation. Do not include markdown.\n\
         END OF INSTRUCTIONS: output JSON only.",
        attempt.project_id,
        attempt.issue_id,
        attempt.work_item_id,
        attempt.id,
        attempt.branch_name,
        evaluation_context_json,
        retry_diagnostic_section
    )
}

pub fn build_tester_plan_repair_prompt(raw_output: &str, parse_error: &str) -> String {
    let truncated_raw = truncate_for_prompt(raw_output, 800);
    format!(
        "CRITICAL: Return ONLY a single JSON object. No markdown, no explanations, no validation reports, no tables.\n\
         Tester Provider Runtime\n\
         Phase: plan_tests_repair\n\
         The previous plan_tests output could not be parsed as TestPlan JSON.\n\
         Parse error: {parse_error}\n\
         \n\
         DO NOT output markdown headers (##), code fences (```), validation report tables, or repair summaries.\n\
         DO NOT summarize what you are doing.\n\
         Output MUST be a single raw JSON object starting with {{ and ending with }}.\n\
         \n\
         Required shape:\n\
         {{\"summary\":\"...\",\"context_warnings\":[],\"assumptions\":[],\"steps\":[{{\"id\":\"...\",\"title\":\"...\",\"intent\":\"...\",\"required\":true,\"tool\":\"run_command|read_file|list_files|search_code|provider_managed\",\"risk_level\":\"low|medium|high\",\"command_or_tool_input\":{{}},\"evidence_expectation\":\"...\"}}]}}\n\
         \n\
         Previous output (ERROR - this format was wrong, do not repeat it):\n\
         {truncated_raw}\n\
         \n\
         END OF INSTRUCTIONS: output JSON only."
    )
}

fn truncate_for_prompt(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    let remaining = chars.count();
    if remaining == 0 {
        return truncated;
    }
    format!("{truncated}\n...[truncated {remaining} chars]")
}

pub fn build_tester_execute_repair_prompt(
    raw_output: &str,
    missing_required_steps: &[String],
) -> String {
    format!(
        "Tester Provider Runtime\n\
         Phase: execute_test_plan_repair\n\
         The previous execute_test_plan output did not provide valid step_results for every required step.\n\
         Missing required steps: {missing_required_steps:?}\n\
         Return only JSON: {{\"step_results\":[{{\"step_id\":\"...\",\"status\":\"passed|failed|blocked|skipped\",\"evidence_refs\":[\"...\"],\"provider_analysis\":\"...\"}}]}}\n\
         Previous output:\n\
         {raw_output}"
    )
}
