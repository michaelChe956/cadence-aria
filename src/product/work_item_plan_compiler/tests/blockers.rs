use super::{
    WorkItemPlanSourceContext, compile_work_item_plan, lint_work_item_plan_source,
    parse_work_item_plan,
};
use crate::product::work_item_plan_compiler::lower;

const REP4_FIXTURE: &str = include_str!(
    "../../../../openspec/changes/rearch-workitem-plan-pipeline/fixtures/work-item-plan-rep4.md"
);

#[test]
fn empty_blockers_section_parses_and_lowers_to_no_blocker_rules() {
    let source = REP4_FIXTURE
        .replace(
            "### Blockers\n- reason_code: levels_api_contract_invalid\n- route: plan_repair_current\n- target_contract_refs: contract.levels-api\n",
            "### Blockers\n",
        )
        .replace(
            "### Blockers\n- reason_code: level_selector_contract_invalid\n- route: plan_repair_upstream\n- target_contract_refs: contract.levels-api\n",
            "### Blockers\n",
        )
        .replace(
            "### Blockers\n- reason_code: levels_integration_contract_invalid\n- route: verification_retry\n- target_contract_refs: contract.levels-api\n- target_contract_refs: contract.level-selector\n",
            "### Blockers\n",
        );
    let context = WorkItemPlanSourceContext {
        target_repository_id: "repo-levels".to_string(),
    };

    let ast = parse_work_item_plan(&source).expect("空 Blockers 的完整文档应通过 parse");
    assert_eq!(ast.items.len(), 3);
    let ir = lower::lower_work_item_plan(&source, ast, &context)
        .expect("空 Blockers 的完整文档应通过 lower");
    assert!(
        ir.items
            .iter()
            .all(|item| item.contract.blocker_rules.is_empty())
    );

    let non_empty_ir = compile_work_item_plan(REP4_FIXTURE, &context)
        .expect("非空 Blockers 的完整文档行为应保持不变");
    assert!(
        non_empty_ir
            .items
            .iter()
            .all(|item| !item.contract.blocker_rules.is_empty())
    );
}

#[test]
fn omitted_handoff_refs_section_still_rejects_missing_required_field() {
    let source = REP4_FIXTURE.replace(
        "- provided_contract_refs: contract.levels-integration\n",
        "",
    );
    let diagnostics = lint_work_item_plan_source(&source);
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "missing_section" && diagnostic.field == "provided_contract_refs"
        }),
        "省略 Handoff Schema 的 provided_contract_refs 仍必须拒绝：{diagnostics:#?}"
    );
}

#[test]
fn empty_non_blockers_section_still_rejects_missing_required_field() {
    let source = REP4_FIXTURE.replace(
        "### Goal\n- summary: WHEN a level list request arrives THE SYSTEM SHALL return the configured levels JSON.\n",
        "### Goal\n",
    );
    let diagnostics = lint_work_item_plan_source(&source);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "missing_section" && diagnostic.field == "summary"),
        "除 Blockers 外的空 section 仍必须拒绝：{diagnostics:#?}"
    );
}
