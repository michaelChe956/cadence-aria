// A+（openspec change `unify-work-item-group-structure` tasks.md 5.0 + 5.4，
// REQ-WG-07「互斥取值来源必须清除」）：`Plan Intent` 的 `intent_target_repo`
// 在 author prompt 内**不得**并存两条互斥取值来源——取值来源必须按会话类型
// 唯一：单仓会话取 `[issue]` 头 `repo=`，LC 多仓会话取成员表物理成员仓 ID。
//
// 旧形态（2026-10-31 物证）：grammar 段无条件要求「逐字取 [issue] 头 repo=」，
// LC 成员段又改口「不取 [issue] 头 repo=，改为取成员表物理成员仓 ID」——同一
// 字段两个来源并存，模型两头都可能填对一半、填错一半。
//
// 本文件只锁 prompt 文本契约，不碰编译器/候选门（那是 A1 的范围）。
// A1（平台接管归属填写）未落地前，「模型自己填归属」这条规则**不得**删除——
// 本模块用 `plan_intent_target_teaching_keeps_model_filled_ownership_rule`
// 把它钉在场。

use crate::product::work_item_plan_compiler::PlanMemberTargets;
use crate::product::work_item_split_engine::prompts::{
    WorkItemPlanMarkdownAuthorContext, build_work_item_plan_markdown_prompt,
};

/// 本模块自带语言规则夹具（`TEST_SINGLE_CANDIDATE_LANGUAGE_RULES` 住在
/// `prompt_contract` 子模块内，不可跨模块引用）。
const A_PLUS_LANGUAGE_RULES: &str = "结构标题与字段 key 保持 grammar 指定的英文原样。";

/// 会话类型限定词：规定 `intent_target_repo` 取值来源的句子必须至少带一个，
/// 否则该句对两类会话同时生效 = 互斥来源并存（形态一在 prompt 侧的唯一成因）。
const SESSION_TYPE_QUALIFIERS: [&str; 4] = [
    "单仓会话",
    "LC 多仓会话",
    "无成员仓清单",
    "有成员仓清单",
];

/// 取值来源标记：出现其一即认为该句在规定「值从哪来」。
const TARGET_SOURCE_MARKERS: [&str; 2] = ["[issue] 头 repo=", "物理成员仓 ID"];

/// 取值来源唯一规则句（grammar 段，单一权威句；两类会话各一支）。
const PLAN_INTENT_TARGET_SOURCE_RULE: &str = "intent_target_repo 取值来源按会话类型唯一、不得混填：单仓会话（本 prompt 无成员仓清单）逐字取 [issue] 头 repo= 的 repository id；LC 多仓会话（有成员仓清单）逐字取该清单的物理成员仓 ID。";

/// 计量锁（只降不升；tasks.md 5.3 的局部覆盖——本块体量不得回长）：
/// 取值来源规则句的字节上限。
const PLAN_INTENT_TARGET_SOURCE_RULE_MAX_BYTES: usize = 320;

/// 硬性措辞数上限（只降不升）：`[issue] 头 repo=` 在 prompt 内出现的次数。
/// 单仓会话 = 1（唯一规则句）；LC 多仓会话 = 2（唯一规则句 + LC 段否定式提醒）。
const ISSUE_HEADER_SOURCE_MAX_OCCURRENCES: usize = 2;

/// 旧噪声句（逐字印着 lowering_error 原文）与其附属表述必须整体缺席。
const RETIRED_CLAUSES: [&str; 5] = [
    "（逐字取 [issue] 头 repo= 的 repository id，不得填成员仓 UUID）",
    "另一分支的 key 不顶替",
    "LC 多仓限定（本段在场即生效）：intent_target_repo",
    "仅是单仓默认取值",
    "Plan Intent 声明在场时该字段必填。",
];

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
            repository_structure: "src/product/levels; web/src/levels",
            language_rules: A_PLUS_LANGUAGE_RULES,
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
    use crate::product::logical_codebase::LogicalRepositoryId;
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
            repository_structure: "busi/src/product/levels; web/src/levels",
            language_rules: A_PLUS_LANGUAGE_RULES,
            plan_options: &TEST_ALL_FALSE_PLAN_OPTIONS,
            routing_context: &RoutingReferenceContext::Legacy,
            member_targets: Some(&members),
            change_order: &[backend, web],
            baseline_tree: None,
        },
    )
    .expect("LC multi-repo markdown author prompt")
}

/// 抽出 prompt 里所有「规定 `intent_target_repo` 值从哪来」的句子。
///
/// 切分口径：`。` 与换行（`；` 不切——同一句内的分句共享同一会话类型限定）。
fn target_source_sentences(prompt: &str) -> Vec<String> {
    prompt
        .split(['。', '\n'])
        .filter(|sentence| sentence.contains("intent_target_repo"))
        .filter(|sentence| {
            TARGET_SOURCE_MARKERS
                .iter()
                .any(|marker| sentence.contains(marker))
        })
        .map(str::to_string)
        .collect()
}

#[test]
fn plan_intent_target_repo_source_is_unique_per_session_type() {
    for (session, prompt) in [("单仓会话", single_repo_prompt()), ("LC 多仓会话", lc_prompt())] {
        let sentences = target_source_sentences(&prompt);
        assert!(
            !sentences.is_empty(),
            "{session}必须教 intent_target_repo 的取值来源，否则模型无从取值"
        );
        for sentence in &sentences {
            assert!(
                SESSION_TYPE_QUALIFIERS
                    .iter()
                    .any(|qualifier| sentence.contains(qualifier)),
                "{session}出现未带会话类型限定的取值来源句（=互斥来源并存，形态一成因）：{sentence}\n--- prompt ---\n{prompt}"
            );
        }
        assert!(
            prompt.contains(PLAN_INTENT_TARGET_SOURCE_RULE),
            "{session}必须携带唯一的取值来源规则句：{PLAN_INTENT_TARGET_SOURCE_RULE}\n--- prompt ---\n{prompt}"
        );
    }
}

#[test]
fn plan_intent_target_teaching_drops_contradictory_and_error_verbatim_clauses() {
    for (session, prompt) in [("单仓会话", single_repo_prompt()), ("LC 多仓会话", lc_prompt())] {
        for retired in RETIRED_CLAUSES {
            assert!(
                !prompt.contains(retired),
                "{session}必须移除互斥/噪声条款「{retired}」：\n--- prompt ---\n{prompt}"
            );
        }
    }
}

#[test]
fn plan_intent_target_teaching_keeps_model_filled_ownership_rule() {
    // A1（平台接管归属填写）未落地前，「模型自己填归属」这条规则不得删除：
    // 删了规则而平台也不接管 = 模型既无依据也无校验面（比矛盾更糟）。
    for (session, prompt) in [("单仓会话", single_repo_prompt()), ("LC 多仓会话", lc_prompt())] {
        assert!(
            prompt.contains("intent_target_kind=single_repository 时必写 intent_target_repo"),
            "{session}必须保留「模型自己填 intent_target_repo」的分支必填规则：\n--- prompt ---\n{prompt}"
        );
        assert!(
            prompt.contains("intent_target_kind=logical_codebase 时必写 intent_target_codebase 与 intent_target_logical_repo"),
            "{session}必须保留 logical_codebase 分支的必填 key 教学：\n--- prompt ---\n{prompt}"
        );
        assert!(
            prompt.contains("两分支 key 不顶替"),
            "{session}必须保留「两分支 key 不互相顶替」的纪律（kimi-10 零重驱现场形态）：\n--- prompt ---\n{prompt}"
        );
    }
    // LC 会话额外保留否定式提醒：成员表在场时不得回头取 [issue] 头 repo=。
    let lc = lc_prompt();
    assert!(
        lc.contains("（不取 [issue] 头 repo=）"),
        "LC 多仓会话必须保留「不取 [issue] 头 repo=」的否定式提醒：\n--- prompt ---\n{lc}"
    );
}

#[test]
fn plan_intent_target_source_rule_is_size_locked() {
    for (session, prompt) in [("单仓会话", single_repo_prompt()), ("LC 多仓会话", lc_prompt())] {
        assert!(
            PLAN_INTENT_TARGET_SOURCE_RULE.len() <= PLAN_INTENT_TARGET_SOURCE_RULE_MAX_BYTES,
            "{session}取值来源规则句体量回长（只降不升）：{} > {}",
            PLAN_INTENT_TARGET_SOURCE_RULE.len(),
            PLAN_INTENT_TARGET_SOURCE_RULE_MAX_BYTES
        );
        assert!(
            prompt.matches("取值来源").count() == 1,
            "{session}必须只有一条取值来源权威句（硬性措辞数上限）：{}",
            prompt.matches("取值来源").count()
        );
        assert!(
            prompt.matches("[issue] 头 repo=").count() <= ISSUE_HEADER_SOURCE_MAX_OCCURRENCES,
            "{session}的「[issue] 头 repo=」硬性措辞数超限（只降不升）：{}",
            prompt.matches("[issue] 头 repo=").count()
        );
    }
}
