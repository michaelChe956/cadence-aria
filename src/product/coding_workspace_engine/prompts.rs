use super::*;
use crate::cross_cutting::structured_output::StructuredOutputContract;
use crate::product::cadence_skills::routing_reference::direct_cadence_routing_rules_reference;
use crate::product::coding_models::CodingAttemptScope;

pub(crate) fn code_review_output_contract(nonce: &str) -> String {
    format!(
        "\n\
         输出终端结构化结论契约：\n\
         - 你可以先输出简短工作流路由回执或可读进度，但这些文本不得包含 {{ 或 }}。\n\
         - 最终审查结论必须且只能放在以下带 nonce 的终端块内；不要在 JSON 外使用 Markdown fence：\n\
         <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">\n\
         {{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[{{\"severity\":\"error|warning|info\",\"file_path\":\"...\",\"line\":1,\"message\":\"...\",\"required_action\":\"...\",\"source_stage\":\"code_review\"}}]}}\n\
         </ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">\n\
         - 不得输出 Markdown fence 包裹 JSON；最终结论的 JSON 必须是合法对象。\n"
    )
}

pub(crate) fn code_review_structured_output_contract(nonce: String) -> StructuredOutputContract {
    StructuredOutputContract {
        nonce,
        schema_name: "coding_workspace_code_review".to_string(),
    }
}

impl CodingWorkspaceEngine {
    pub(crate) async fn build_code_review_prompt(
        &self,
        attempt: &CodingExecutionAttempt,
        worktree_path: &Path,
        retry_diagnostic: Option<&str>,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let diff_base = code_review_diff_base(attempt)?;
        let diff = self._git_service.git_diff(worktree_path, diff_base).await?;
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
             {}\
             {}\
             {}\
             {}\
             {}\
             \n代码规范:\n\
             - 优先检查正确性、边界条件、测试覆盖、安全、性能和可维护性。\n\
             - findings 必须包含 severity、file_path、line、message、required_action、source_stage=code_review。\n\
             - 如果没有阻塞问题，verdict 使用 approve。\n\
             \n原始需求上下文:\n````markdown\n{}\n````\n\
             \nEvaluationContextPack:\n````json\n{}\n````\n\
             \ngit diff:\n````diff\n{}\n````\n\
             {}\
             \n输出顺序与格式：\n\
             - 首个用户可见消息必须是纯文本工作流路由回执，且不得包含 {{ 或 }}。\n\
             - 完成必调 Skill 与原始规则读取后，最终审查结论仅输出裸 JSON：{{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[...]}}\n",
            provider_runtime_contract("CodeReviewer"),
            attempt.project_id,
            attempt.issue_id,
            active_work_item_id_for_prompt(attempt),
            attempt.id,
            attempt.branch_name,
            attempt.base_branch,
            code_review_material_protocol(),
            crate::product::plan_repair::plan_defect_structured_output_contract(),
            reviewer_test_scope_contract(),
            reviewer_process_evidence_boundary_contract(),
            no_default_stack_assumption_contract(),
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
            "Coding Workspace GroupFinalReview\n\
             {}\n\
             你是 WorkItemGroup GroupFinalReview reviewer，仅在 WorkItemGroup 全部 coding units 完成且 ReviewRequest push 之后做整组功能审查。单 WorkItem scope 不应生成本 prompt。\n\
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
             {}\
             {}\
             {}\
             {}\
             {}\
             \n输出要求:\n\
             - 分析影响范围（影响范围/impact_scope）。\n\
             - 给出 PR description 预览。\n\
             - 给出 commit message 建议。\n\
             - findings 必须包含 source_stage=group_final_review。\n\
             \n只输出 JSON：{{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[...],\"impact_scope\":[\"...\"],\"pr_description\":\"...\",\"commit_message_suggestion\":\"...\"}}\n",
            provider_runtime_contract("GroupFinalReview"),
            attempt.project_id,
            attempt.issue_id,
            active_work_item_id_for_prompt(attempt),
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
            group_final_review_material_protocol(),
            crate::product::plan_repair::plan_defect_structured_output_contract(),
            reviewer_test_scope_contract(),
            reviewer_process_evidence_boundary_contract(),
            no_default_stack_assumption_contract(),
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
        attempt.project_id,
        attempt.issue_id,
        active_work_item_id_for_prompt(attempt),
        attempt.id,
        attempt.branch_name
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
        prompt.push_str("\n上一轮修复要求:\n");
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
    prompt.push_str(&coding_execution_protocol());
    prompt.push_str(no_default_stack_assumption_contract());
    prompt.push_str(coding_completion_report_contract());
    prompt.push_str(crate::product::plan_repair::plan_defect_structured_output_contract());
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
        attempt.project_id,
        attempt.issue_id,
        active_work_item_id_for_prompt(attempt),
        attempt.id,
        attempt.branch_name
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
        prompt.push_str("\n本轮修复要求:\n");
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
            "\n本轮没有新增修复要求。请基于当前会话和 worktree 状态继续完成未结束的代码编写任务。\n",
        );
    }
    append_coding_context_notes(&mut prompt, context_notes);
    prompt.push_str(&coding_delta_execution_protocol());
    prompt.push_str(no_default_stack_assumption_contract());
    prompt.push_str(coding_completion_report_contract());
    prompt.push_str(crate::product::plan_repair::plan_defect_structured_output_contract());
    prompt
}

pub(crate) fn no_default_stack_assumption_contract() -> &'static str {
    "\n不得用平台默认技术栈假设替代任务材料。语言、构建系统、包管理器、测试框架、依赖初始化和模块接入要求，必须来自 Work Item、Source Draft Supplement、Verification Plan、EvaluationContextPack、项目规则、仓库文件事实或用户补充上下文。若材料不足，必须报告不确定性，不得臆造具体命令或工具。\n"
}

pub(crate) fn reviewer_test_scope_contract() -> &'static str {
    "\nReviewer 非 E2E 测试边界:\n\
     - 你可以根据需求、当前 diff、仓库事实、测试证据和代码风险提出单元测试、非浏览器自动化的集成测试、编译、构建、类型检查、静态分析、格式检查或 lint 等验证要求。\n\
     - 这些测试建议不受 Verification Plan 已列命令的严格限制，但测试框架、命令和技术栈判断必须来自任务材料、仓库事实或项目规则，不得凭平台默认假设生成。\n\
     - 不得创建以新增、执行、补充、修复、配置或安装 E2E、端到端测试、Playwright、浏览器自动化测试或运行这些测试所需浏览器环境为目的的 finding。\n\
     - 上述测试及其所需浏览器环境的安装、配置、缺失、失败或相关证据（包括缺少证据），均不得成为 finding，也不得导致 request_changes 或 blocked；不得作为 verdict 或 summary 中的否决理由，也不得成为 Coder required_action 或任何返修要求。\n\
     - 即使 Work Item、Design Spec、Verification Plan、handoff 或 EvaluationContextPack 提到上述测试及其所需浏览器环境，也不得将其转换成 finding、verdict/summary 否决理由、Coder required_action 或任何返修要求。\n"
}

pub(crate) fn reviewer_process_evidence_boundary_contract() -> &'static str {
    "\nReviewer 过程证据边界:\n\
     - 过程事实同时满足两条：无法从当前 diff、验证命令输出、handoff 字段或人工检查结果观测；实现完成后即使 Coder 返修也无法产出该证据。\n\
     - 典型过程事实包括 red commit 的存在、失败到通过的提交序列、开发时序、提交拆分粒度、分支创建与 rebase 历史以及 Coder 会话内操作顺序。\n\
     - 不得创建以过程事实为目的的 finding。过程事实的缺失、不符合预期或缺少证据均不得成为 finding，不得导致 request_changes 或 blocked，不得作为 verdict 或 summary 中的否决理由，不得成为 Coder required_action 或任何返修要求。\n\
     - 即使 Work Item、Design Spec、Verification Plan、handoff 或 EvaluationContextPack 提到上述过程事实，也不得转换为 finding、verdict 或 summary 中的否决理由、Coder required_action 或任何返修要求。\n\
     - EvidenceKind 仅表示当前可观测证据：source_diff 表示最终代码状态；non_zero_test_execution 表示验证命令执行时实际运行了非零数量的测试，是当前可观测的执行结果；它不表达测试曾先失败、不表达提交顺序、不表达任何开发时序。manual_check 仅表示人工检查结果；handoff_field 仅表示交接字段的存在与内容。\n\
     - 本边界不削弱可观测结果的审查：仍必须审查测试文件是否存在、测试是否覆盖需求场景、验证命令是否真实执行且非零、测试输出是否与实现自相矛盾，以及 Forbidden Write Scopes 是否被越过。\n"
}

pub(crate) fn coding_execution_protocol() -> String {
    with_cadence_routing_reference(concat!(
        "当前阶段：已确认 OpenSpec 与 Plan/Work Item 范围内实施。必调 Skill：using-superpowers → executing-plans；写代码前调用 test-driven-development。若范围、架构或验收变化，停止并交给 Aria 既有审批 gate。\n",
        "Coder 执行协议:\n\
     - 在修改代码前，必须先阅读“已确认 Work Item”，并从其中提取本次任务的执行清单。\n\
     - 执行清单必须覆盖：实现目标、允许修改范围、禁止修改范围、TDD/测试要求、依赖初始化或环境诊断要求、验证命令与执行顺序、完成前自检要求、handoff 中要求交付给下游的契约。\n\
     - 如果 Work Item、Source Draft Supplement、Verification Plan 已明确给出某项要求，必须按其内容执行。\n\
     - 如果执行材料没有给出语言、构建系统、包管理器或测试框架相关要求，不得臆造具体技术栈命令。\n\
     - 需要判断环境或依赖问题时，必须优先根据 Work Item、Verification Plan、仓库文件和项目规则判断。\n\
     - 如果判断依据不足，必须在最终报告中说明“不足以确定”，并列出需要人工确认的问题。\n\
     - 人工事项不属于你的执行范围：required_evidence 含 manual_check 的验收标准、以及只有 manual_instruction 而无 command 的检查，都由人工在流程末端确认。必须在最终报告中单列“待人工处理”清单（逐项写明事项与人工执行方式），但不得因无法执行它们而报阻塞、拒绝完成或降低完成度。\n\
     - 缺少浏览器、设备、外部账号等人工环境不是运维阻塞：只有当你自己该执行的命令或代码修改无法进行时，才输出 operational_gate。\n\
     - 不得用平台默认技术栈假设替代 Work Item 内容。\n"
    ))
}

pub(crate) fn coding_delta_execution_protocol() -> String {
    with_cadence_routing_reference(concat!(
        "当前阶段：已确认 Plan/Work Item 范围内的 bounded rework。必调 Skill：using-superpowers → executing-plans；写代码前调用 test-driven-development。若范围、架构或验收变化，停止并交给 Aria 既有审批 gate。\n",
        "Coder 增量执行协议:\n\
     - 继续以本会话中的“已确认 Work Item”和 Verification Plan 作为任务来源。\n\
     - 在继续修改前，必须重新核对本轮修复要求、补充上下文和原 Work Item 中的执行要求。\n\
     - 若存在人工修复意见，人工修复意见优先级最高；当人工修复意见与 reviewer findings、原 Work Item 或既有上下文冲突时，优先遵循人工修复意见，并在最终报告说明冲突和取舍。\n\
     - 若没有人工修复意见，但本轮 reviewer findings 与原 Work Item 冲突，优先遵循更具体、更新的本轮 reviewer findings；同时在最终报告说明冲突和取舍。\n\
     - 不得引入平台默认技术栈假设；语言、构建系统、包管理器、测试框架相关动作必须来自 Work Item、Verification Plan、仓库文件或项目规则。\n\
     - 如果判断依据不足，必须在最终报告中说明“不足以确定”，并列出需要人工确认的问题。\n"
    ))
}

pub(crate) fn coding_completion_report_contract() -> &'static str {
    "\n完成报告要求:\n\
     - 先列出你从 Work Item / Final Compile / Verification Plan 提取出的执行清单。\n\
     - 列出实际修改文件。\n\
     - 列出实际执行的验证命令。\n\
     - 粘贴每条验证命令的完整输出。\n\
     - 报告 git status --short（含未跟踪文件）与 git diff --stat。\n\
     - 基于两者明确说明是否触碰 Forbidden Write Scopes；若出现允许范围外的已修改或未跟踪文件，必须报告，不得声称未触碰。\n\
     - 如果测试命令显示没有测试被执行或没有实际测试被执行，包括 \"0 tests\" 或 \"running 0 tests\"，不能直接视为已覆盖；必须说明处理方式或风险。\n\
     - 如果某项要求无法执行，说明阻塞原因、已尝试的诊断步骤和需要人工确认的内容。\n\
     - 必须单列“待人工处理”小节：逐项写明需要人工核对、人工测试或人工操作的事项及其执行方式；无此类事项时明确写“无”。该清单是交接内容，不是未完成项。\n"
}

pub(crate) fn code_review_material_protocol() -> String {
    with_cadence_routing_reference(concat!(
        "当前阶段：只读代码审查。必调 Skill：using-superpowers → requesting-code-review。不得绕过现有 gate 或修改文件。\n",
        "首个用户可见消息必须是工作流路由回执；该回执不属于最终审查结论，不能因 JSON 合同省略。完成必调 Skill 与原始规则读取后，最终审查结论必须只输出一个 JSON 对象，不要输出 Markdown、解释、验证报告或表格。\n\
     CodeReviewer 审查协议:\n\
     - 只分析当前变更 diff，不修改代码、不执行写操作。\n\
     - 在给出 verdict 前，必须从“原始需求上下文”和 EvaluationContextPack 中提取本次任务的审查清单。\n\
     - 审查清单必须覆盖：实现目标、允许修改范围、禁止修改范围、TDD/测试要求、验证命令与证据、完成前自检要求、交接契约与能力、需求/设计追踪关系。\n\
     - EvaluationContextPack.CoderEvidencePack 是 coder 已执行工作的证据包；必须优先审查其中的 role run、raw/artifact refs、completion report 和 evidence_warnings。\n\
     - WorkItemGroup 当前 Unit 的 completion commit 与 HandoffRevision 在 Code Review approve 后才生成；Code Review 前为空是正常状态，不得据此创建 finding、request_changes 或 blocked。\n\
     - Code Review 阶段应以 Coder completion report、raw/artifact refs、实际测试输出和当前 Unit diff 判断验证证据；真正缺失或自相矛盾的 required verification evidence 仍必须记录。\n\
     - 不得重复执行 required verification commands；除非证据缺失、证据自相矛盾或用户/Work Item 明确要求 reviewer 复跑，否则只基于 CoderEvidencePack、diff 和任务材料判断。\n\
     - 必须审查 diff 是否满足 Work Item 的实现目标、写入范围、禁止范围、验证计划、自检要求和交接契约。\n\
     - 如果 coder 报告或 EvaluationContextPack 中缺少 required 验证命令的执行证据，必须作为 finding 记录；若该证据是完成本 Work Item 的必要条件，verdict 应为 request_changes 或 blocked。\n\
     - 待人工处理事项不是缺陷：required_evidence 含 manual_check 的验收标准、以及只有 manual_instruction 而无 command 的检查，由人工在流程末端确认。coder 如实登记这些事项即为正确交付，不得因其“尚未验证”创建 finding、给出 request_changes 或 blocked；缺少浏览器等人工环境同理。\n\
     - 如果测试输出显示没有实际测试被执行，不能把它当作有效覆盖；必须结合 Work Item 要求判断是否需要修复。\n\
     - EvidenceKind 仅表示当前可观测证据：source_diff 表示最终代码状态；non_zero_test_execution 表示验证命令执行时实际运行了非零数量的测试，是当前可观测的执行结果；它不表达测试曾先失败、不表达提交顺序、不表达任何开发时序。manual_check 仅表示人工检查结果；handoff_field 仅表示交接字段的存在与内容。\n\
     - 不得提出执行材料之外的技术栈默认要求。\n\
     - verdict 只能使用 approve、request_changes、blocked。\n\
     - finding.severity 只能使用 error、warning、info。\n\
     - verdict=blocked 时，阻塞 finding 使用 severity=error；不得使用 severity=blocked。\n\
     - findings 必须包含 defect_class、reason_code、contract_refs、capability_refs、repair_target、recommended_route、confidence、evidence；普通 implementation defect 使用 defect_class=implementation_defect 和 recommended_route=coder_rework。\n\
     - 除最终结论 JSON 外，其余任何内容（包括路由回执、验证证据、示例和表格）不得出现 { 或 }；证据中的 JSON 片段必须改写为自然语言描述。\n\
     - JSON 必须以 { 开头，以 } 结尾；不要输出 Markdown 代码块或自然语言总结。\n"
    ))
}

pub(crate) fn group_final_review_material_protocol() -> String {
    with_cadence_routing_reference(concat!(
        "当前阶段：组级 PR 最终只读审查。必调 Skill：using-superpowers → requesting-code-review。不得绕过现有 gate 或修改文件。\n",
        "WorkItemGroup GroupFinalReview 审查协议:\n\
     - 你必须从 Completed Units、HandoffRevision、EvaluationContextPack 和完整 diff 中提取整组审查清单。\n\
     - 跨 unit 交接的审查对象是 HandoffRevision 的契约与能力语义：Completed Units 段落中每个 unit 的 Provided Contracts 与 Provided Capabilities。不存在自然语言交接摘要，不得要求或期待它。\n\
     - 必须确认每个 completed unit 的 HandoffRevision 所声明的契约与能力是否体现在最终 diff 中。\n\
     - 必须检查依赖交接是否断裂：上游 unit 声明的契约与能力（API、状态、文件）是否被下游正确消费。\n\
     - 必须检查整组 diff 是否越过任何 unit 的 Forbidden Write Scopes。\n\
     - 如果某个 unit 的验证证据缺失、声明的契约与能力未在 diff 中落地、或最终 PR 描述遗漏关键影响，必须 request_changes 或 blocked。\n\
     - 待人工处理事项不是缺陷：required_evidence 含 manual_check 的验收标准、以及只有 manual_instruction 而无 command 的检查，由人工在本流程之后确认。这些事项尚未验证不得成为 finding、request_changes 或 blocked 的理由；必须在 summary 中汇总整组的待人工处理清单，供人工接手。\n\
     - 如果 ReviewRequest 已 push 的 commit 与 completed units、diff 或验证证据不一致，必须 request_changes 或 blocked。\n\
     - impact_scope、pr_description、commit_message_suggestion 必须基于实际 diff、completed units 和 HandoffRevision，不得编造未实现内容。\n\
     - 不得用平台默认技术栈假设替代 HandoffRevision 或 Work Item 内容。\n\
     - EvidenceKind 仅表示当前可观测证据：source_diff 表示最终代码状态；non_zero_test_execution 表示验证命令执行时实际运行了非零数量的测试，是当前可观测的执行结果；它不表达测试曾先失败、不表达提交顺序、不表达任何开发时序。manual_check 仅表示人工检查结果；handoff_field 仅表示交接字段的存在与内容。\n\
     - verdict 只能使用 approve、request_changes、blocked。\n\
     - finding.severity 只能使用 error、warning、info。\n\
     - verdict=blocked 时，阻塞 finding 使用 severity=error；不得使用 severity=blocked。\n\
     - findings 必须包含 source_stage=group_final_review。\n\
     - findings 必须包含 defect_class、reason_code、contract_refs、capability_refs、repair_target、recommended_route、confidence、evidence；普通 implementation defect 使用 defect_class=implementation_defect 和 recommended_route=coder_rework。\n\
     - 除最终结论 JSON 外，其余任何内容（包括路由回执、验证证据、示例和表格）不得出现 { 或 }；证据中的 JSON 片段必须改写为自然语言描述。\n"
    ))
}

fn with_cadence_routing_reference(protocol: &'static str) -> String {
    let mut rendered = String::from("\n");
    rendered.push_str(direct_cadence_routing_rules_reference());
    rendered.push_str(protocol);
    rendered
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
        "请将这些人工补充要求与本轮修复要求一起执行；如有冲突，优先遵循更具体的人工补充上下文。\n",
    );
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
    permission_mode: ProviderPermissionMode,
) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type: input.provider_type.clone(),
        role: input.role.clone(),
        prompt: input.prompt.clone(),
        working_dir,
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: input.timeout,
    }
}

pub(crate) fn active_work_item_id_for_prompt(attempt: &CodingExecutionAttempt) -> &str {
    attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id)
}

pub(crate) fn code_review_diff_base(
    attempt: &CodingExecutionAttempt,
) -> Result<&str, CodingWorkspaceEngineError> {
    if attempt.scope == CodingAttemptScope::WorkItemGroup {
        if active_work_item_id_for_prompt(attempt) == attempt.work_item_id {
            return Ok(&attempt.base_branch);
        }
        return attempt.head_commit.as_deref().ok_or_else(|| {
            CodingWorkspaceEngineError::CompletionCommitMissing(attempt.id.clone())
        });
    }
    Ok(&attempt.base_branch)
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
