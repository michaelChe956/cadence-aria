// C1-T5（REQ-WSC-06）：SC author prompt 的创建计划选项镜像教学 + AC 基线纪律。
// F-51 根因：author 不知道创建计划已启用 integration/e2e/前后端拆分选项，首轮缺
// 对应 kind 的 Work Item 被编译前预检 Error 拒，浪费一轮 provider 驱动；F-56 根因：
// AC 引用 fork 基线树外路径（其他分支才存在的文件，如 status.html）。镜像行与
// AC 修复动作 MUST 逐字引用校验器共享常量（禁止消费侧重述第二份文案）。

const PLAN_OPTIONS_TEST_LANGUAGE_RULES: &str =
    "## 语言规则\n\n- **必须使用中文** - 所有响应、解释、注释和文档必须使用中文。\n";

fn all_true_plan_options() -> crate::product::models::IssueWorkItemPlanOptions {
    crate::product::models::IssueWorkItemPlanOptions {
        include_integration_tests: true,
        include_e2e_tests: true,
        force_frontend_backend_split: true,
        require_execution_plan_confirm: false,
    }
}

fn all_false_plan_options() -> crate::product::models::IssueWorkItemPlanOptions {
    crate::product::models::IssueWorkItemPlanOptions {
        include_integration_tests: false,
        include_e2e_tests: false,
        force_frontend_backend_split: false,
        require_execution_plan_confirm: false,
    }
}

fn plan_options_prompt(
    options: &crate::product::models::IssueWorkItemPlanOptions,
) -> String {
    let (request, issue, repository) = split_prompt_fixture();
    crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
        &request,
        &issue,
        &repository,
        crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
            story_context: "story_spec_0001: level selection",
            design_context: "design_spec_0001: levels API",
            design_requirement_ids: &[],
            repository_structure: "src/product/levels; web/src/levels; tests/integration",
            language_rules: PLAN_OPTIONS_TEST_LANGUAGE_RULES,
            plan_options: options,
            routing_context: &RoutingReferenceContext::Legacy,
        },
    )
    .expect("markdown author prompt")
}

/// flag=true：三条镜像行逐字携带校验器共享修复动作常量（两种修复路径），
/// 且明示「启用而缺对应 kind → Error」后果；段位于 few-shot 之后、[format_clamp] 之前。
#[test]
fn sc_author_prompt_mirrors_enabled_plan_options_with_verbatim_repair_actions() {
    let prompt = plan_options_prompt(&all_true_plan_options());

    for required in [
        "[plan_options_mirror]",
        "启用而缺对应 kind 的 Work Item 将被编译前预检 Error 拒绝",
        "- include_integration_tests=true：",
        "- include_e2e_tests=true：",
        "- force_frontend_backend_split=true：",
    ] {
        assert!(
            prompt.contains(required),
            "options 镜像教学必须包含 {required}: {prompt}"
        );
    }
    for constant in [
        crate::product::work_item_split_validator::INTEGRATION_WORK_ITEM_REQUIRED_REPAIR_ACTION,
        crate::product::work_item_split_validator::E2E_WORK_ITEM_REQUIRED_REPAIR_ACTION,
        crate::product::work_item_split_validator::FRONTEND_BACKEND_SPLIT_REQUIRED_REPAIR_ACTION,
    ] {
        assert!(
            prompt.contains(constant),
            "镜像教学必须逐字引用校验器共享常量（禁止重述第二份文案）：{constant}"
        );
    }

    let few_shot_pos = prompt.find("[real_finding_few_shot]").expect("few-shot marker");
    let mirror_pos = prompt.find("[plan_options_mirror]").expect("mirror marker");
    let clamp_pos = prompt.find("[format_clamp]").expect("format clamp marker");
    assert!(
        few_shot_pos < mirror_pos && mirror_pos < clamp_pos,
        "镜像教学必须位于 few-shot 之后、[format_clamp] 之前（尾部输出指令前）"
    );
}

/// flag=false：镜像行整段缺席（条件渲染，禁污染未启用选项的 prompt）。
#[test]
fn sc_author_prompt_omits_disabled_option_mirror_teaching() {
    let prompt = plan_options_prompt(&all_false_plan_options());

    assert!(
        !prompt.contains("[plan_options_mirror]"),
        "未启用任何选项时不得渲染镜像教学段: {prompt}"
    );
    for constant in [
        crate::product::work_item_split_validator::INTEGRATION_WORK_ITEM_REQUIRED_REPAIR_ACTION,
        crate::product::work_item_split_validator::E2E_WORK_ITEM_REQUIRED_REPAIR_ACTION,
        crate::product::work_item_split_validator::FRONTEND_BACKEND_SPLIT_REQUIRED_REPAIR_ACTION,
    ] {
        assert!(
            !prompt.contains(constant),
            "flag=false 时修复动作常量不得出现: {constant}"
        );
    }
    // 红线（22,000）内的净增证明点：全 false 基线 fixture（AC 固定行在场）。
    assert!(
        prompt.len()
            < crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES,
        "全 false fixture prompt 必须低于质量预算红线，实测 {} bytes",
        prompt.len()
    );
    eprintln!(
        "plan_options(all-false) SC author prompt bytes={} margin={}",
        prompt.len(),
        crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES
            - prompt.len()
    );
    eprintln!(
        "plan_options(all-true) SC author prompt bytes={}",
        plan_options_prompt(&all_true_plan_options()).len()
    );
}

/// AC 基线纪律固定行：无论选项如何都必须在场（F-56：AC/验证引用的文件路径
/// 必须存在于 plan 基线树），修复动作逐字引用编译器共享常量。
#[test]
fn sc_author_prompt_teaches_acceptance_baseline_discipline_with_verbatim_repair_action() {
    for prompt in [
        plan_options_prompt(&all_false_plan_options()),
        plan_options_prompt(&all_true_plan_options()),
    ] {
        for required in [
            "AC 与验证计划引用的仓库内文件路径必须存在于 plan 基线树，不得引用其他分支才存在的文件",
            "acceptance_path_not_in_baseline Error",
            crate::product::work_item_plan_compiler::ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION,
        ] {
            assert!(
                prompt.contains(required),
                "AC 基线纪律固定行必须存在且口径同源：{required}: {prompt}"
            );
        }
    }
}
