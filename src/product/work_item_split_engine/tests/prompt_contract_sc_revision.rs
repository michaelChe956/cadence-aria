// F5 组1（A findings 回灌）：SC author 修订轮 prompt 必须回灌 reviewer findings
// 与硬性修复指令；首轮（无 verdict）prompt 逐字节不变。契约测试风格对齐
// tests/prompt_contract.rs（include 进 prompt_contract_sc_revision 模块）。
// B4（REQ-WSC-12 窄例外·五入口之五）：作者只消费会话有效 structured
// findings——[review_findings] 的 actionable 条目是唯一执行清单；raw
// comments/summary 散文只留审计面，不进作者 prompt。

use crate::product::work_item_plan_policy::{FindingClassHint, ReviewFindingCategory};
use crate::web::workspace_ws_types::{
    ReviewFinding, ReviewFindingDisposition, ReviewFindingSeverity, ReviewGate, ReviewVerdict,
    ReviewVerdictType,
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
        machine_repair: None,
        owner_question: None,
        owner_options: None,
    disposition: None, target_work_item_id: None, finding_key: None, }
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
        machine_repair: None,
        owner_question: None,
        owner_options: None,
    disposition: None, target_work_item_id: None, finding_key: None, }
}

/// owner 已处置、被平台强制降级为 advisory 的 finding：severity 仍是 must_fix。
fn advisory_must_fix_finding(message: &str, required_action: &str) -> ReviewFinding {
    ReviewFinding {
        severity: ReviewFindingSeverity::MustFix,
        message: message.to_string(),
        evidence: "docs/frontend-spec.md:12".to_string(),
        required_action: required_action.to_string(),
        category: None,
        class_hint: None,
        contract_field: None,
        machine_repair: None,
        owner_question: None,
        owner_options: None,
        disposition: Some(ReviewFindingDisposition::Advisory),
        target_work_item_id: None,
        finding_key: None,
    }
}

/// B4（REQ-WSC-12 窄例外·五入口之五）：SC 会话内被平台降级为 advisory 的
/// 建议级非契约 finding——severity=Suggestion + class_hint=Repairable +
/// category=Completeness，B1 投影后的会话有效视图（disposition=advisory）。
/// 生产链路：`finalize_review_verdict` / `latest_review_verdict_for_session`
/// 投影 → `single_candidate_pending_revision_verdict` 传入本 builder。
fn narrow_exception_demoted_suggestion(message: &str, required_action: &str) -> ReviewFinding {
    ReviewFinding {
        severity: ReviewFindingSeverity::Suggestion,
        message: message.to_string(),
        evidence: "WI-003 Traceability 登记块".to_string(),
        required_action: required_action.to_string(),
        category: Some(ReviewFindingCategory::Completeness),
        class_hint: Some(FindingClassHint::Repairable),
        contract_field: None,
        machine_repair: None,
        owner_question: None,
        owner_options: None,
        disposition: Some(ReviewFindingDisposition::Advisory),
        target_work_item_id: None,
        finding_key: None,
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
        plan_options: &TEST_ALL_FALSE_PLAN_OPTIONS,
        routing_context: &RoutingReferenceContext::Legacy,
        member_targets: None,
        change_order: &[],
        baseline_tree: None,
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

    // findings 文本回灌：结构化 actionable 清单逐条在场（B4 新契约：raw
    // comments/summary 散文不进作者面，见下方 forbidden 断言）。
    for required in [
        "[review_revision]",
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
    // B4（REQ-WSC-12 窄例外·五入口之五）：raw comments/summary 散文只留审计面。
    for forbidden_prose in [
        "Reviewer 审核意见:",
        "Reviewer 摘要:",
        "CT-001 与 WI-002 的 input contract 能力供需不一致",
        "契约缺口需返修",
    ] {
        assert!(
            !revision.contains(forbidden_prose),
            "raw 散文 {forbidden_prose} 不得进入作者返修 prompt: {revision}"
        );
    }

    // 硬性修复指令。
    for required in [
        "[revision_directives]",
        "返修=逐字应用：每条 required_action 给出的补丁行/删除操作逐字应用到上一版 source 的对应位置",
        "已通过部分逐字节保持不变，禁止重写、重排或润色未涉及的 section",
        "必须在本轮内逐条修复全部 must_fix/blocking findings",
        "逐条对齐其 required_action",
        "不得只修复其中部分",
        "唯一执行清单",
        "不得新增与上述 findings 无关的其他变更",
    ] {
        assert!(
            revision.contains(required),
            "revision prompt must carry the hard repair directive {required}: {revision}"
        );
    }
}

/// REQ-WSC-14（作者返修输入面）：owner 已处置、被平台强制降级为 advisory 的
/// finding（severity 仍是 must_fix）MUST NOT 出现在 `[review_findings]`，也
/// MUST NOT 进入 `[revision_directives]` 的「必须逐条修复」清单；充其量以
/// 「仅供提示，本轮不要修改」单列。
#[test]
fn sc_revision_prompt_excludes_owner_downgraded_findings_from_repair_surface() {
    let (request, issue, repository) = split_prompt_fixture();
    let verdict = revise_verdict_fixture(
        "本轮只修可收敛项。",
        "范围外事实已被 owner 排除",
        vec![
            must_fix_finding(
                "WI-002 required_capabilities 缺 capability.field-constraints",
                "在 CT-001 显式声明 capability.field-constraints。",
            ),
            advisory_must_fix_finding("中文 locale 尚未配置（产品侧文档事实）", "回写产品侧文档"),
        ],
    );
    let revision =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_revision_prompt(
            &request, &issue, &repository, author_context(), &verdict,
        )
        .expect("revision markdown author prompt with owner-downgraded finding");

    assert!(
        revision.contains("message: WI-002 required_capabilities 缺 capability.field-constraints"),
        "actionable finding 必须进入 [review_findings]: {revision}"
    );
    assert!(
        !revision.contains("中文 locale 尚未配置（产品侧文档事实）"),
        "降级后的 finding message 不得进入注入 provider 的 prompt: {revision}"
    );
    assert!(
        !revision.contains("回写产品侧文档"),
        "降级后的 finding required_action 不得进入注入 provider 的 prompt: {revision}"
    );
    // advisory 只允许单列提示，并写明本轮不要修改。
    let advisory_at = revision
        .find("[review_advisory_findings]")
        .expect("advisory findings 必须单列提示段");
    let directives_at = revision
        .find("[revision_directives]")
        .expect("revision directives section");
    assert!(
        advisory_at < directives_at,
        "advisory 提示段必须位于 [revision_directives] 之前"
    );
    let advisory_section = &revision[advisory_at..directives_at];
    assert!(
        advisory_section.contains("仅供提示，本轮不要修改"),
        "advisory 段必须写明仅供提示、本轮不要修改: {advisory_section}"
    );
    // 「必须逐条修复」只覆盖 [review_findings] 里的 actionable 条目。
    assert!(
        revision.contains(
            "必须在本轮内逐条修复全部 must_fix/blocking findings（仅限 [review_findings] 列出的 actionable 条目）"
        ),
        "修复指令必须把范围钉在 actionable findings 上: {revision}"
    );
}

/// B4（REQ-WSC-12 窄例外·五入口之五「作者返修」，tasks 5.1）：作者返修轮只
/// 消费会话有效 structured findings——`[review_findings]` 的 actionable 条目
/// 是本轮唯一执行清单。被窄例外降级的建议（severity=Suggestion +
/// class_hint=Repairable + category=Completeness，B1 投影后 disposition=
/// advisory）不得以任何形态进入作者 prompt；raw comments/summary 散文只留
/// 审计面，不得把降级建议复活成「全部要改」；真必修的 required_action/
/// evidence 不丢。resume 与 fresh 两个 builder 同源走
/// `render_review_revision_block`，两侧契约一致。
#[test]
fn sc_revision_prompt_consumes_effective_findings_only_and_keeps_raw_prose_out() {
    let (request, issue, repository) = split_prompt_fixture();
    let verdict = revise_verdict_fixture(
        "「建议为 WI-003 补验收基线引用」等全部 8 条问题（含建议项）本轮都必须修复。",
        "全部 8 条（含建议项）必须修复",
        vec![
            must_fix_finding(
                "WI-002 required_capabilities 缺 capability.field-constraints",
                "在 CT-001 显式声明 capability.field-constraints。",
            ),
            narrow_exception_demoted_suggestion(
                "建议为 WI-003 补验收基线引用",
                "补 Traceability 登记行（仅建议）",
            ),
        ],
    );
    let revision =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_revision_prompt(
            &request, &issue, &repository, author_context(), &verdict,
        )
        .expect("revision markdown author prompt with narrow-exception finding");
    let fresh = crate::product::work_item_split_engine::prompts::
        build_work_item_plan_markdown_fresh_revision_prompt(
            &request,
            &issue,
            &repository,
            author_context(),
            &verdict,
            "# Work Item Plan\n## Work Item WI-001: x\n（上一版 source 全文）\n",
        )
        .expect("fresh revision markdown author prompt with narrow-exception finding");

    for prompt in [&revision, &fresh] {
        // raw 散文不进作者面：comments/summary 留在审计面（durable verdict 与
        // review message），作者侧只见结构化清单与非执行计数说明。
        for forbidden_prose in [
            "Reviewer 审核意见:",
            "Reviewer 摘要:",
            "全部 8 条",
            "（含建议项）本轮都必须修复",
        ] {
            assert!(
                !prompt.contains(forbidden_prose),
                "raw 散文 {forbidden_prose} 不得进入作者返修 prompt: {prompt}"
            );
        }
        // 被窄例外降级的建议：message/evidence/required_action 不得经任何通道
        // （结构化清单或散文）复活为执行项。
        for forbidden_finding in [
            "建议为 WI-003 补验收基线引用",
            "WI-003 Traceability 登记块",
            "补 Traceability 登记行（仅建议）",
            "2. severity:",
        ] {
            assert!(
                !prompt.contains(forbidden_finding),
                "被窄例外降级的建议 {forbidden_finding} 不得出现在作者返修 prompt: {prompt}"
            );
        }
        // 唯一执行清单 + 真必修 required_action/evidence 不丢。
        for required in [
            "[review_revision]",
            "[review_findings]",
            "1. severity: must_fix",
            "message: WI-002 required_capabilities 缺 capability.field-constraints",
            "evidence: work-item-plan.md WI-002 Inputs",
            "required_action: 在 CT-001 显式声明 capability.field-constraints。",
            "唯一执行清单",
            "必须在本轮内逐条修复全部 must_fix/blocking findings（仅限 [review_findings] 列出的 actionable 条目）",
        ] {
            assert!(
                prompt.contains(required),
                "作者返修 prompt 必须携带唯一执行清单与真必修事实 {required}: {prompt}"
            );
        }
        // 降级项只以非执行计数说明出现：1 条 advisory，不带原文。
        let advisory_at = prompt
            .find("[review_advisory_findings]")
            .expect("advisory findings 必须单列计数提示段");
        let directives_at = prompt
            .find("[revision_directives]")
            .expect("revision directives section");
        assert!(advisory_at < directives_at);
        let advisory_section = &prompt[advisory_at..directives_at];
        assert!(
            advisory_section.contains("本轮另有 1 条 finding 已被平台判定为 advisory"),
            "advisory 段必须只给非执行计数说明: {advisory_section}"
        );
        assert!(
            advisory_section.contains("仅供提示，本轮不要修改"),
            "advisory 段必须写明仅供提示、本轮不要修改: {advisory_section}"
        );
        assert!(
            !advisory_section.contains("WI-003"),
            "advisory 计数段不得携带降级项原文: {advisory_section}"
        );
    }
}

/// B4（REQ-WSC-12 窄例外·五入口之五）：无 findings 的返修轮仍携带返修框与
/// 修复指令，但 raw comments/summary 散文不进作者面（只留审计面）——散文
/// 不得在无结构化执行清单时凭空制造「必须修改」的指令。
#[test]
fn sc_revision_prompt_without_findings_carries_directives_without_raw_prose() {
    let (request, issue, repository) = split_prompt_fixture();
    let verdict = revise_verdict_fixture("整体结构需按 reviewer 意见调整。", "返修", Vec::new());
    let revision =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_revision_prompt(
            &request, &issue, &repository, author_context(), &verdict,
        )
        .expect("revision markdown author prompt without findings");
    assert!(revision.contains("[review_revision]"));
    assert!(revision.contains("[review_findings]"));
    assert!(revision.contains("无必须修复项"));
    assert!(revision.contains("[revision_directives]"));
    assert!(revision.contains("必须在本轮内逐条修复"));
    for forbidden_prose in [
        "Reviewer 审核意见:",
        "Reviewer 摘要:",
        "整体结构需按 reviewer 意见调整。",
    ] {
        assert!(
            !revision.contains(forbidden_prose),
            "raw 散文 {forbidden_prose} 不得进入作者返修 prompt: {revision}"
        );
    }
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
            member_targets: None,
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

// ---------------------------------------------------------------------------
// SG5.1（REQ-CG-03、REQ-SG-02）：同源 author/revision grammar 教学——初轮与
// SC 修订 prompt 都必须携带 [confirmed_member_order]（已确认成员归属/仓序
// 事实 + 仓序自动边规则）；成员事实与 compile member targets/confirmed
// design change_order 单源同构，不另立白名单；Plan Intent 保持可选，不造
// 创建/入站授权；author 质量预算不扩大（legacy 单仓教学段整段缺席）。
// ---------------------------------------------------------------------------

/// 真实形态 LC 夹具：两个已确认成员仓（logical UUID → 物理成员仓 ID）与
/// confirmed design 的两仓执行顺序（rep 形态与生产 member targets 同构）。
fn sg_05_1_member_facts() -> (
    crate::product::work_item_plan_compiler::PlanMemberTargets,
    Vec<crate::product::logical_codebase::LogicalRepositoryId>,
) {
    use crate::product::logical_codebase::LogicalRepositoryId;
    let backend = LogicalRepositoryId(
        uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("fixture uuid"),
    );
    let web = LogicalRepositoryId(
        uuid::Uuid::parse_str("22222222-2222-2222-2222-222222222222").expect("fixture uuid"),
    );
    let mut physical_by_logical = std::collections::BTreeMap::new();
    physical_by_logical.insert(backend, "repo_backend_0001".to_string());
    physical_by_logical.insert(web, "repo_web_0002".to_string());
    (
        crate::product::work_item_plan_compiler::PlanMemberTargets {
            physical_by_logical,
            fallback_physical_id: Some("repo_backend_0001".to_string()),
        },
        vec![backend, web],
    )
}

fn sg_05_1_author_context<'a>(
    member_targets: &'a crate::product::work_item_plan_compiler::PlanMemberTargets,
    change_order: &'a [crate::product::logical_codebase::LogicalRepositoryId],
) -> crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext<'a> {
    use crate::product::cadence_skills::routing_reference::LogicalPolicyReference;
    static SG_05_1_ROUTING:
        crate::product::cadence_skills::routing_reference::RoutingReferenceContext =
        crate::product::cadence_skills::routing_reference::RoutingReferenceContext::Logical(
            LogicalPolicyReference {
                policy_id: String::new(),
                policy_revision: 0,
                policy_digest: String::new(),
                authority_root: String::new(),
            },
        );
    crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
        story_context: "story_spec_0001: level selection",
        design_context: "design_spec_0001: levels API + gateway/web 两仓划分",
        design_requirement_ids: &[],
        repository_structure: "cadence/src/product/levels; web/src/levels",
        language_rules: TEST_SINGLE_CANDIDATE_LANGUAGE_RULES,
        plan_options: &TEST_ALL_FALSE_PLAN_OPTIONS,
        routing_context: &SG_05_1_ROUTING,
        member_targets: Some(member_targets),
        change_order,
        baseline_tree: None,
    }
}

#[test]
fn sg_05_1_shared_grammar_template_teaches_confirmed_member_order_and_auto_edges_in_initial_and_revision_prompts()
 {
    let (request, issue, repository) = split_prompt_fixture();
    let (members, change_order) = sg_05_1_member_facts();
    let context = sg_05_1_author_context(&members, &change_order);
    let initial =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            context,
        )
        .expect("initial markdown author prompt with member facts");

    // 成员表：真实 logical UUID → 物理成员仓 ID 逐字成行（确定性 BTreeMap 序）。
    let member_line_backend = "- logical_repository: 11111111-1111-1111-1111-111111111111 → physical_repository: repo_backend_0001";
    let member_line_web = "- logical_repository: 22222222-2222-2222-2222-222222222222 → physical_repository: repo_web_0002";
    // 仓序：confirmed change_order 先后如实注入（1. 先执行仓 → 2. 后执行仓）。
    let order_line_backend = "1. 11111111-1111-1111-1111-111111111111（repo_backend_0001）";
    let order_line_web = "2. 22222222-2222-2222-2222-222222222222（repo_web_0002）";
    for required in [
        "[confirmed_member_order]",
        member_line_backend,
        member_line_web,
        order_line_backend,
        order_line_web,
    ] {
        assert!(
            initial.contains(required),
            "initial prompt must carry confirmed member/order fact {required}: {initial}"
        );
    }
    assert!(
        initial.find(order_line_backend).expect("order line 1")
            < initial.find(order_line_web).expect("order line 2"),
        "change_order teaching must preserve the confirmed execution order"
    );

    // auto 边规则与编译器仓序规则同源：前仓为后仓依赖前置、同仓无自动边、
    // 逆写仓序成环拒绝；WI 合法 ID 仍指回本计划 `## Work Item` 标题。
    for required in [
        "仓序自动边规则（与编译器仓序规则同源）",
        "provider_logical_work_item_id 逐字填前仓 WI 在本计划 `## Work Item` 标题中的 `WI-<数字>`",
        "同一成员仓内的 WI 之间无仓序自动边",
        "dependency_cycle",
    ] {
        assert!(
            initial.contains(required),
            "initial prompt must carry the auto-edge rule {required}: {initial}"
        );
    }

    // 成员归属纪律：归属合法值=上列成员事实；native root/聚合根视图不得替换
    // 业务成员 target。
    for required in [
        "intent_target_logical_repo 只能逐字取上列成员 logical UUID",
        "intent_target_repo 只能逐字取上列物理成员仓 ID",
        "不得替代业务成员 target",
        "work_item_target_missing 拒绝",
    ] {
        assert!(
            initial.contains(required),
            "initial prompt must carry member enrollment discipline {required}: {initial}"
        );
    }
    let member_section = &initial[initial
        .find("[confirmed_member_order]")
        .expect("member section start")
        ..initial
            .find("[format_clamp]")
            .expect("format clamp must follow")];
    assert!(
        !member_section.contains("repo_0001")
            && !member_section.contains("lc_aggregate_root_view_"),
        "member teaching must keep real business member targets, not the session native root or aggregate view: {member_section}"
    );
    assert_eq!(
        initial.matches("[confirmed_member_order]").count(),
        1,
        "member/order teaching renders exactly once"
    );

    // 同源 grammar/template/WI 合法 ID 教学在初轮仍在场（新教学不替换旧面）。
    for required in [
        "[markdown_grammar]",
        "[minimum_legal_source]",
        "provider_logical_work_item_id 的合法值只能来自本计划的 `## Work Item` 标题中的 `WI-<数字>`",
    ] {
        assert!(
            initial.contains(required),
            "initial prompt must keep the shared grammar/template teaching {required}"
        );
    }

    // SC 修订轮：以初轮 prompt 为逐字节前缀，成员/仓序教学同源继承。
    let verdict = revise_verdict_fixture(
        "WI 归属与仓序依赖需返修。",
        "成员归属返修",
        vec![must_fix_finding(
            "WI-002 intent_target_logical_repo 编造成员 UUID",
            "intent_target_logical_repo 逐字改取已确认成员表中的 22222222-2222-2222-2222-222222222222。",
        )],
    );
    let revision =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_revision_prompt(
            &request, &issue, &repository,
            sg_05_1_author_context(&members, &change_order),
            &verdict,
        )
        .expect("revision prompt with member facts");
    let initial_prefix_end = initial
        .rfind(SC_REVISION_OUTPUT_DIRECTIVE)
        .expect("initial prompt ends with the shared output directive");
    assert!(
        revision.starts_with(&initial[..initial_prefix_end]),
        "revision prompt must keep the initial prompt bytes (incl. member/order teaching) as prefix"
    );
    for required in [
        "[confirmed_member_order]",
        member_line_backend,
        member_line_web,
        "[markdown_grammar]",
        "[minimum_legal_source]",
    ] {
        assert!(
            revision.contains(required),
            "revision prompt must inherit shared member/grammar teaching {required}"
        );
    }

    // 显式 fresh 返修轮同样自足携带成员/仓序教学。
    let fresh = crate::product::work_item_split_engine::prompts::
        build_work_item_plan_markdown_fresh_revision_prompt(
            &request,
            &issue,
            &repository,
            sg_05_1_author_context(&members, &change_order),
            &verdict,
            "# Work Item Plan\n## Work Item WI-001: x\n（上一版 source 全文）\n",
        )
        .expect("fresh revision prompt with member facts");
    assert!(fresh.contains("[confirmed_member_order]"));
    assert!(fresh.contains(member_line_web));
}

#[test]
fn sg_05_1_shared_grammar_template_keeps_plan_intent_optional_and_author_budget_unexpanded() {
    let (request, issue, repository) = split_prompt_fixture();
    let (members, change_order) = sg_05_1_member_facts();
    let context = sg_05_1_author_context(&members, &change_order);
    let initial =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            context,
        )
        .expect("initial markdown author prompt with member facts");

    // Plan Intent 保持可选 section：教学明示不改可选性、不新增创建/入站授权；
    // 成员段不重列 intent 值域（grammar 段是唯一白名单来源，不另白名单）。
    let member_section = &initial[initial
        .find("[confirmed_member_order]")
        .expect("member section start")
        ..initial
            .find("[format_clamp]")
            .expect("format clamp must follow")];
    for required in ["Plan Intent 仍为可选 section", "不新增任何创建/入站授权"] {
        assert!(
            member_section.contains(required),
            "member teaching must state {required}: {member_section}"
        );
    }
    assert!(
        !member_section.contains("create、existing"),
        "member teaching must not re-list the intent value domain (grammar is the single whitelist source)"
    );

    // author 质量预算不扩大：常量保持 23,500（历史红线，SG5.1 不提额），
    // legacy 单仓 prompt（无成员事实）不渲染成员段且仍在预算内。
    assert_eq!(
        crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES,
        23_500
    );
    let legacy =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            author_context(),
        )
        .expect("legacy single-repo markdown author prompt");
    assert!(
        !legacy.contains("[confirmed_member_order]"),
        "legacy single-repo prompt must stay free of member/order teaching"
    );
    assert!(
        legacy.len()
            < crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES,
        "legacy prompt must remain below the unexpanded author budget: {} bytes",
        legacy.len()
    );

    // 缺席事实不编造：LC 成员在场但 confirmed change_order 为空时，如实教
    // 「无已确认仓序、无仓序自动边」，成员表照常教学。
    let empty_order_context = sg_05_1_author_context(&members, &[]);
    let empty_order =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            empty_order_context,
        )
        .expect("LC prompt without confirmed change order");
    for required in [
        "无已确认仓序",
        "无仓序自动边，跨仓依赖仅由 depends_on 与 Inputs 契约显式声明",
        "不得虚构先后顺序",
        "- logical_repository: 22222222-2222-2222-2222-222222222222 → physical_repository: repo_web_0002",
    ] {
        assert!(
            empty_order.contains(required),
            "empty change order must be taught as-is without inventing an order: {required}"
        );
    }
}

/// 硬兜底抬到 256 KiB（262_144）：真机 run 中修订轮 prompt 达 70,326 字节
/// （59,460 基础 prompt + 10,866 修订块），被旧的 64 KiB 兜底拒绝后卡死自动化。
/// 硬兜底的设计意图只是拦截病态序列化回归；真实物理边界是模型上下文窗口
/// （prompt 经 stdin JSON 发送，无 OS ARG_MAX 约束）。本测试锁住两点：
/// 70 KB 级修订 prompt 必须放行；超过 256 KiB 的仍 fail closed 且报告新上限。
#[test]
fn sc_revision_prompt_accepts_seventy_kilobyte_budget_and_still_caps_at_256kib() {
    let (request, issue, repository) = split_prompt_fixture();
    assert_eq!(
        crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES,
        262_144,
        "markdown author hard backstop must be 256 KiB"
    );

    let base =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            author_context(),
        )
        .expect("first-round markdown author prompt");
    assert!(
        base.len() < 65_536,
        "fixture base prompt must sit below the legacy backstop: {} bytes",
        base.len()
    );

    // 把修订 prompt 总量校准到 ~70 KB（旧 64 KiB 兜底之上、新 256 KiB 兜底之下）。
    // B4 后 raw comments 不再进作者 prompt，校准改走结构化 actionable finding
    // 的 message 回灌通道（[review_findings] 仍是修订块的可增长面）。
    let padding = "x".repeat(70_000usize.saturating_sub(base.len()));
    let verdict = revise_verdict_fixture(
        "评论散文不进作者 prompt",
        "返修",
        vec![must_fix_finding(&padding, "逐字应用该补丁行。")],
    );
    let revision =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_revision_prompt(
            &request,
            &issue,
            &repository,
            author_context(),
            &verdict,
        )
        .expect("70KB revision prompt must stay invocable under the 256KiB backstop");
    assert!(
        revision.len() > 65_536,
        "fixture must actually exceed the legacy 64KiB limit: {} bytes",
        revision.len()
    );
    assert!(
        revision.len() < 262_144,
        "fixture must stay below the raised 256KiB backstop: {} bytes",
        revision.len()
    );

    // 病态回归拦截能力不丢：超过 256 KiB 的仍 fail closed，并报告新上限。
    let oversized = revise_verdict_fixture(
        "评论散文不进作者 prompt",
        "返修",
        vec![must_fix_finding(&"x".repeat(300_000), "逐字应用该补丁行。")],
    );
    let error =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_revision_prompt(
            &request,
            &issue,
            &repository,
            author_context(),
            &oversized,
        )
        .expect_err("prompt above the 256KiB hard backstop must fail closed");
    assert!(
        error.contains("exceeds hard budget"),
        "budget error must stay diagnosable, got: {error}"
    );
    assert!(
        error.contains("262144"),
        "budget error must report the raised 256KiB limit, got: {error}"
    );
}
