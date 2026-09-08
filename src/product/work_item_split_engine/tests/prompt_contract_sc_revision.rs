// F5 组1（A findings 回灌）：SC author 修订轮 prompt 必须回灌 reviewer findings
// 与硬性修复指令；首轮（无 verdict）prompt 逐字节不变。契约测试风格对齐
// tests/prompt_contract.rs（include 进 prompt_contract_sc_revision 模块）。

use crate::web::workspace_ws_types::{
    ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
};

const TEST_SINGLE_CANDIDATE_LANGUAGE_RULES: &str =
    "## 语言规则\n\n- **必须使用中文** - 所有响应、解释、注释和文档必须使用中文。\n";

const SC_REVISION_OUTPUT_DIRECTIVE: &str = "[output] 现在仅输出完整 markdown source。";

fn revise_verdict_fixture(
    comments: &str,
    summary: &str,
    findings: Vec<ReviewFinding>,
) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: comments.to_string(),
        summary: summary.to_string(),
        findings,
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

fn must_fix_finding(message: &str, required_action: &str) -> ReviewFinding {
    ReviewFinding {
        severity: ReviewFindingSeverity::MustFix,
        message: message.to_string(),
        evidence: "work-item-plan.md WI-002 Inputs".to_string(),
        required_action: required_action.to_string(),
        category: None,
        class_hint: None,
        contract_field: None,
    }
}

fn suggestion_finding(message: &str) -> ReviewFinding {
    ReviewFinding {
        severity: ReviewFindingSeverity::Suggestion,
        message: message.to_string(),
        evidence: "Traceability section".to_string(),
        required_action: "补登记行".to_string(),
        category: None,
        class_hint: None,
        contract_field: None,
    }
}

fn author_context()
-> crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext<'static> {
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
fn sc_revision_prompt_injects_findings_and_directives_on_top_of_byte_identical_base() {
    let (request, issue, repository) = split_prompt_fixture();
    let verdict = revise_verdict_fixture(
        "CT-001 与 WI-002 的 input contract 能力供需不一致，需返修。",
        "契约缺口需返修",
        vec![
            must_fix_finding(
                "WI-002 required_capabilities 缺 capability.field-constraints",
                "在 CT-001 显式声明 capability.field-constraints，或改写 WI-002 的 required_capabilities。",
            ),
            suggestion_finding("建议补 Traceability 登记行"),
        ],
    );
    let base =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            author_context(),
        )
        .expect("first-round markdown author prompt");
    let revision =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_revision_prompt(
            &request, &issue, &repository, author_context(), &verdict,
        )
        .expect("revision markdown author prompt");

    // 修订轮必须以首轮 prompt 的逐字节前缀开头，只在 [output] 指令前插入返修段，
    // 且输出指令本身保持唯一、居尾。
    let base_prefix_end = base
        .rfind(SC_REVISION_OUTPUT_DIRECTIVE)
        .expect("base prompt must end with the shared output directive");
    assert!(
        revision.starts_with(&base[..base_prefix_end]),
        "revision prompt must keep the first-round prompt bytes as its prefix"
    );
    assert!(revision.ends_with(SC_REVISION_OUTPUT_DIRECTIVE));
    assert_eq!(revision.matches(SC_REVISION_OUTPUT_DIRECTIVE).count(), 1);
    assert!(revision.len() > base.len());

    // findings 文本回灌：comments/summary（对齐 legacy revision.rs 的拼装）与逐条 findings。
    for required in [
        "[review_revision]",
        "Reviewer 审核意见:",
        "CT-001 与 WI-002 的 input contract 能力供需不一致，需返修。",
        "Reviewer 摘要:",
        "契约缺口需返修",
        "[review_findings]",
        "1. severity: must_fix",
        "message: WI-002 required_capabilities 缺 capability.field-constraints",
        "evidence: work-item-plan.md WI-002 Inputs",
        "required_action: 在 CT-001 显式声明 capability.field-constraints，或改写 WI-002 的 required_capabilities。",
        "2. severity: suggestion",
        "message: 建议补 Traceability 登记行",
    ] {
        assert!(
            revision.contains(required),
            "revision prompt must inject review feedback {required}: {revision}"
        );
    }

    // 硬性修复指令。
    for required in [
        "[revision_directives]",
        "必须在本轮内逐条修复全部 must_fix/blocking findings",
        "逐条对齐其 required_action",
        "不得只修复其中部分",
        "不得新增与上述 findings 无关的其他变更",
    ] {
        assert!(
            revision.contains(required),
            "revision prompt must carry the hard repair directive {required}: {revision}"
        );
    }
}

#[test]
fn sc_revision_prompt_without_findings_still_carries_comments_and_directives() {
    let (request, issue, repository) = split_prompt_fixture();
    let verdict = revise_verdict_fixture("整体结构需按 reviewer 意见调整。", "返修", Vec::new());
    let revision =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_revision_prompt(
            &request, &issue, &repository, author_context(), &verdict,
        )
        .expect("revision markdown author prompt without findings");
    assert!(revision.contains("[review_revision]"));
    assert!(revision.contains("整体结构需按 reviewer 意见调整。"));
    assert!(revision.contains("Reviewer 摘要:"));
    assert!(revision.contains("[revision_directives]"));
    assert!(revision.contains("必须在本轮内逐条修复"));
}

#[test]
fn sc_first_round_prompt_stays_byte_identical_and_revision_marker_free() {
    let (request, issue, repository) = split_prompt_fixture();
    let first =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            author_context(),
        )
        .expect("first-round markdown author prompt");
    let second =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            author_context(),
        )
        .expect("first-round markdown author prompt rebuilt");
    assert_eq!(
        first, second,
        "首轮 prompt 对相同输入必须逐字节确定（无 verdict 注入路径）"
    );
    for marker in [
        "[review_revision]",
        "[review_findings]",
        "[revision_directives]",
    ] {
        assert!(
            !first.contains(marker),
            "first-round prompt must never contain revision marker {marker}"
        );
    }
    assert!(first.ends_with(SC_REVISION_OUTPUT_DIRECTIVE));
}

/// F5 回灌扩展（契约校验前移）：机械 verdict（canonical 契约缺口适配层输出）
/// 走同一条返修 prompt 通道——逐条回灌 code 前缀 message/定位 evidence/
/// required_action 模板，硬性修复指令照常生效；字节预算硬闸不破。
#[test]
fn sc_revision_prompt_carries_mechanical_contract_gap_verdict() {
    let (request, issue, repository) = split_prompt_fixture();
    let gap_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
    ))
    .replace(
        "- provided_contract_refs: contract.levels-integration",
        "- provided_contract_refs: []",
    )
    .replace(
        "- required_capabilities: api.levels.read\n",
        "- required_capabilities: api.levels.read, api.levels.write\n",
    );
    let ir = crate::product::work_item_plan_compiler::compile_work_item_plan(
        &gap_source,
        &crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
            target_repository_id: "repo_fixture".to_string(),
        },
    )
    .expect("compile gap fixture");
    let verdict =
        crate::product::workspace_engine::contract_prerevision::single_candidate_contract_prerevision_verdict(
            &ir,
        )
        .expect("gap fixture must yield a mechanical verdict");

    let revision =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_revision_prompt(
            &request, &issue, &repository, author_context(), &verdict,
        )
        .expect("revision prompt with mechanical verdict");

    for required in [
        "[review_revision]",
        "[review_findings]",
        "1. severity: must_fix",
        "message: required_capability_missing: provider WI-001 contract contract.levels-api lacks capability api.levels.write required by WI-002",
        "evidence: work_item=WI-002 contract=contract.levels-api capability=api.levels.write",
        "required_action: 在 WI-001 的 Outputs 契约 contract.levels-api 的 capabilities 列表追加一行（逐字复制）：- api.levels.write",
        "[revision_directives]",
        "必须在本轮内逐条修复全部 must_fix/blocking findings",
    ] {
        assert!(
            revision.contains(required),
            "mechanical verdict must reach the revision prompt verbatim: {required}\n{revision}"
        );
    }
}
