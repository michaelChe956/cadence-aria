use super::*;
use crate::product::cadence_skills::routing_reference::RoutingReferenceContext;

#[test]
fn coding_plan_repair_prompt_contracts_require_canonical_schema_and_legacy_mapping() {
    let attempt = test_attempt("coding_attempt_prompt_contract");
    let context = CodingExecutionContext::default();
    let coding_prompt = build_coding_prompt(
        &attempt,
        &context,
        None,
        None,
        &RoutingReferenceContext::Legacy,
    );
    assert_plan_defect_output_contract(&coding_prompt, "plan_defect_findings");
    assert!(coding_prompt.contains("普通 implementation defect"));

    for prompt in [
        code_review_material_protocol(&RoutingReferenceContext::Legacy),
        group_final_review_material_protocol(&RoutingReferenceContext::Legacy),
    ] {
        assert_plan_defect_output_contract(&prompt, "findings");
        assert!(prompt.contains("普通 implementation defect"));
    }
}

#[test]
fn plan_defect_output_contract_declares_field_value_constraints() {
    let contract = crate::product::plan_repair::plan_defect_structured_output_contract();

    assert!(contract.contains("severity 只能使用 error、warning"));
    assert!(contract.contains("confidence 只能使用 low、medium、high"));
    assert!(contract.contains("repair_target 必须是对象"));
    assert!(contract.contains("logical_work_item_ids"));
    assert!(contract.contains("work_item_revision_ids"));
}

pub(super) fn assert_plan_defect_output_contract(prompt: &str, container: &str) {
    assert!(prompt.contains(container), "missing {container}: {prompt}");
    for field in [
        "defect_class",
        "reason_code",
        "contract_refs",
        "capability_refs",
        "repair_target",
        "recommended_route",
        "confidence",
        "evidence",
    ] {
        assert!(prompt.contains(field), "missing {field}: {prompt}");
    }
}
