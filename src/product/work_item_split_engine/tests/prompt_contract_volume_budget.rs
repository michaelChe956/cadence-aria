// REQ-CRP-05 / Task 7（工作包 7.1–7.3）：SC author 固定教学块的计量闸门——
// 字符数与硬措辞数双上限（只降不升）。每删/压一条机械条款，提交信息中必须
// 指向承接它的平台侧校验/测试（「移除条款需有平台侧覆盖」）；出现既不在
// prompt 也不在平台校验中的规则即回退该条删除。
//
// 计量口径：与 `prompt_contract_plan_intent_target.rs` 同一构造入口
// （`build_work_item_plan_markdown_prompt`）+ 同型固定 fixture。
// - **字符闸门度量会话无关的固定教学块**＝单仓形态全文（与既有
//   `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES` 的 fixture 计量
//   同构）。LC 多仓形态追加 `[confirmed_member_order]` 段，其中成员表/仓序
//   行随会话成员数量伸缩（数据面，非固定教学），任何固定字符上限都无法
//   约束数据驱动的行数——故 LC 全文只作计量输出（eprintln）记入验收记录，
//   不进字符断言；LC 的固定教学增量由 isolate change 的 R-15 双边登记面
//   承接。
// - **硬措辞闸门对两种形态全文生效**：硬措辞（必须|不得|禁止|拒绝）是教学
//   措辞密度，与成员数量无关，LC 形态同样不得超限。
// fixture 的可变成分（issue/上下文/语言规则）为短常量且不含硬措辞，不引入
// 教学体量。

use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::work_item_plan_compiler::PlanMemberTargets;
use crate::product::work_item_split_engine::prompts::{
    WorkItemPlanMarkdownAuthorContext, build_work_item_plan_markdown_prompt,
};

// 基线对照值（2026-11-11 实测）：15,765 字符 / 53 硬措辞（四词：必须|不得|禁止|拒绝）
// tasks.md 记载基线 15.5K/63 为不同计数口径的历史值；以实施时实测为准写入验收记录
const TEACHING_BLOCK_MAX_CHARS: usize = 8_000;
const TEACHING_BLOCK_MAX_HARD_MANDATES: usize = 25;

/// 计量 fixture 的语言规则：与 `prompt_contract_plan_intent_target` 同口径的
/// 短常量，不含硬措辞，避免 fixture 污染硬措辞计数。
const VOLUME_BUDGET_LANGUAGE_RULES: &str = "结构标题与字段 key 保持 grammar 指定的英文原样。";

fn single_repo_prompt() -> String {
    let (request, issue, repository) = split_prompt_fixture();
    build_work_item_plan_markdown_prompt(
        &request,
        &issue,
        &repository,
        WorkItemPlanMarkdownAuthorContext {
            story_context: "story_spec_0001: level selection",
            design_context: "design_spec_0001: levels API",
            design_requirement_ids: &[],
            repository_structure: "a; b",
            language_rules: VOLUME_BUDGET_LANGUAGE_RULES,
            plan_options: &TEST_ALL_FALSE_PLAN_OPTIONS,
            routing_context: &RoutingReferenceContext::Legacy,
            member_targets: None,
            change_order: &[],
            baseline_tree: None,
        },
    )
    .expect("single-repo markdown author prompt")
}

fn lc_prompt() -> String {
    let (request, issue, repository) = split_prompt_fixture();
    let backend = LogicalRepositoryId(
        uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("fixture uuid"),
    );
    let web = LogicalRepositoryId(
        uuid::Uuid::parse_str("22222222-2222-2222-2222-222222222222").expect("fixture uuid"),
    );
    let members = PlanMemberTargets {
        physical_by_logical: std::collections::BTreeMap::from([
            (backend, "repo_backend_0001".to_string()),
            (web, "repo_web_0002".to_string()),
        ]),
        fallback_physical_id: Some("repo_backend_0001".to_string()),
    };
    build_work_item_plan_markdown_prompt(
        &request,
        &issue,
        &repository,
        WorkItemPlanMarkdownAuthorContext {
            story_context: "story_spec_0001: level selection",
            design_context: "design_spec_0001: levels API + backend/web 两仓划分",
            design_requirement_ids: &[],
            repository_structure: "busi/a; web/b",
            language_rules: VOLUME_BUDGET_LANGUAGE_RULES,
            plan_options: &TEST_ALL_FALSE_PLAN_OPTIONS,
            routing_context: &RoutingReferenceContext::Legacy,
            member_targets: Some(&members),
            change_order: &[backend, web],
            baseline_tree: None,
        },
    )
    .expect("LC multi-repo markdown author prompt")
}

/// 硬措辞计数：正则 `必须|不得|禁止|拒绝` 的非重叠匹配总数。四词互不为
/// 对方子串、首字互不相同，逐词计数求和与正则交替语义等价。
fn hard_mandate_count(text: &str) -> usize {
    ["必须", "不得", "禁止", "拒绝"]
        .iter()
        .map(|word| text.matches(word).count())
        .sum()
}

#[test]
fn teaching_block_volume_within_budget() {
    // 先完整计量两种会话形态再统一断言：输出行进验收记录（LC 字符数为
    // 数据驱动面的观测值，不进断言）。
    let single = single_repo_prompt();
    let lc = lc_prompt();
    let single_chars = single.chars().count();
    let single_mandates = hard_mandate_count(&single);
    let lc_chars = lc.chars().count();
    let lc_mandates = hard_mandate_count(&lc);
    eprintln!(
        "TEACHING_BLOCK_VOLUME 单仓会话(固定教学块) chars={single_chars} hard_mandates={single_mandates}; \
         LC 多仓会话(含数据驱动成员面) chars={lc_chars} hard_mandates={lc_mandates}"
    );
    assert!(
        single_chars <= TEACHING_BLOCK_MAX_CHARS,
        "固定教学块（单仓形态）{single_chars} 字符 > 上限 {TEACHING_BLOCK_MAX_CHARS}"
    );
    assert!(
        single_mandates <= TEACHING_BLOCK_MAX_HARD_MANDATES,
        "固定教学块（单仓形态）硬措辞 {single_mandates} 个 > 上限 {TEACHING_BLOCK_MAX_HARD_MANDATES}"
    );
    assert!(
        lc_mandates <= TEACHING_BLOCK_MAX_HARD_MANDATES,
        "LC 多仓形态硬措辞 {lc_mandates} 个 > 上限 {TEACHING_BLOCK_MAX_HARD_MANDATES}"
    );
}
