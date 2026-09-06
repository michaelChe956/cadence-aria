// F2-C（SC author 反前导语教学行）：kimi/r28b 教训——provider 会在文档标题前输出
// 前言/寒暄/说明性开场/路由回执。`build_work_item_plan_markdown_prompt` 的
// `[output]` 段前必须有一行硬性禁令；🔴 首轮既有教学段不删不改，只增此行
// （`[format_clamp]` 钳制块的最后一行、`[output]` 指令之前）。

use super::*;

const TEST_SINGLE_CANDIDATE_LANGUAGE_RULES: &str =
    "## 语言规则\n\n- **必须使用中文** - 所有响应、解释、注释和文档必须使用中文。\n";

const ANTI_PREAMBLE_LINE: &str =
    "禁止任何前言、寒暄、说明性开场或路由回执；输出的第一个字符必须是文档标题。";

fn anti_preamble_context() -> crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext<'static> {
    crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
        story_context: "story_spec_0001: level selection",
        design_context: "design_spec_0001: levels API",
        design_requirement_ids: &[],
        repository_structure: "src/product/levels; web/src/levels; tests/integration",
        language_rules: TEST_SINGLE_CANDIDATE_LANGUAGE_RULES,
        routing_context: &RoutingReferenceContext::Legacy,
    }
}

#[test]
fn sc_author_prompt_carries_anti_preamble_line_right_before_output_directive() {
    let (request, issue, repository) = split_prompt_fixture();
    let prompt =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            anti_preamble_context(),
        )
        .expect("markdown author prompt");

    let clamp_pos = prompt.find("[format_clamp]").expect("format clamp marker");
    let output_pos = prompt.find("[output]").expect("output directive marker");
    let anti_pos = prompt
        .find(ANTI_PREAMBLE_LINE)
        .expect("anti-preamble teaching line must exist");
    assert!(
        clamp_pos < anti_pos,
        "反前导语行必须位于 [format_clamp] 块内（尾部钳制区），实测 clamp={clamp_pos} anti={anti_pos}"
    );
    assert!(
        anti_pos < output_pos,
        "反前导语行必须在 [output] 段之前一行，实测 anti={anti_pos} output={output_pos}"
    );

    // 既有教学段不删不改：[output] 指令仍逐字节居尾，共享尾部指令保持唯一。
    assert_eq!(
        prompt.matches("[output] 现在仅输出完整 markdown source。").count(),
        1
    );
    assert!(prompt.ends_with("[output] 现在仅输出完整 markdown source。"));

    // 红线：教学增行后仍须低于质量预算（20,000，P1-A 第 7 次提额）。
    let budget =
        crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES;
    assert!(
        prompt.len() < budget,
        "F2-C 增行后 fixture prompt 必须仍低于质量预算，实测 {} bytes / 预算 {budget}",
        prompt.len()
    );
    eprintln!(
        "F2-C SC author prompt bytes={} margin={}",
        prompt.len(),
        budget - prompt.len()
    );
}
