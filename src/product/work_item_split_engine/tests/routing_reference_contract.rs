// 路由引用注入契约测试（T3 裁决 A）：
// - Legacy 上下文下 outline/draft prompt 注入改造前的原始路由引用文本；
// - Logical 上下文下注入聚合政策 locator（authority_root/policy_id/revision/digest）。
//
// 基线：`git show cf2b0ba2:src/product/work_item_split_engine/prompts.rs` 的
// :69/:89 注入文本（`direct_cadence_routing_rules_reference_legacy()`）。
//
// 注意：本目录测试经 `tests.rs` 的 `include!` 内联进同一模块，故这里只 import
// 未被他文件引入的新名字，其余全部全限定引用，避免 E0252 重名 import。

use crate::product::cadence_skills::routing_reference::LogicalPolicyReference;

fn logical_policy_fixture() -> LogicalPolicyReference {
    LogicalPolicyReference {
        policy_id: "pol_1".into(),
        policy_revision: 3,
        policy_digest: "abc123".into(),
        authority_root: "/data/aria/aggregate/policy".into(),
    }
}

#[test]
fn work_item_draft_prompt_legacy_injects_original_routing_reference() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let prompt = crate::product::work_item_split_engine::prompts::build_work_item_draft_prompt(
        &outline,
        &outline.work_item_outlines[0],
        crate::product::models::WorkItemGenerationMode::Serial,
        &[],
        &[],
        None,
        "nonce",
        &RoutingReferenceContext::Legacy,
    );

    assert!(prompt.contains("[cadence_project_rules]\n当前目标仓库根目录的 AGENTS.md"));
    assert!(!prompt.contains("authority_root:"));
}

#[test]
fn work_item_draft_prompt_logical_injects_aggregate_policy_reference() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    let context = RoutingReferenceContext::Logical(logical_policy_fixture());

    let prompt = crate::product::work_item_split_engine::prompts::build_work_item_draft_prompt(
        &outline,
        &outline.work_item_outlines[0],
        crate::product::models::WorkItemGenerationMode::Serial,
        &[],
        &[],
        None,
        "nonce",
        &context,
    );

    assert!(prompt.contains("authority_root:"));
    assert!(prompt.contains("policy_digest: abc123"));
    assert!(prompt.contains("不作为政策正文"));
}

#[test]
fn outline_prompt_legacy_injects_original_routing_reference() {
    let (request, issue, repository) = split_prompt_fixture();

    let (prompt, _nonce) =
        crate::product::work_item_split_engine::prompts::build_outline_prompt_with_nonce(
            &request,
            &issue,
            &repository,
            &[],
            &[],
            "(empty)",
            &[],
            &[],
            &RoutingReferenceContext::Legacy,
        );

    assert!(prompt.contains("[cadence_project_rules]\n当前目标仓库根目录的 AGENTS.md"));
    assert!(!prompt.contains("authority_root:"));
}

#[test]
fn outline_prompt_logical_injects_aggregate_policy_reference() {
    let (request, issue, repository) = split_prompt_fixture();
    let context = RoutingReferenceContext::Logical(logical_policy_fixture());

    let (prompt, _nonce) =
        crate::product::work_item_split_engine::prompts::build_outline_prompt_with_nonce(
            &request,
            &issue,
            &repository,
            &[],
            &[],
            "(empty)",
            &[],
            &[],
            &context,
        );

    assert!(prompt.contains("authority_root:"));
    assert!(prompt.contains("policy_digest: abc123"));
    assert!(prompt.contains("不作为政策正文"));
}
