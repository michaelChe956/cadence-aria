use super::*;

#[test]
fn coding_plan_repair_prompt_contracts_require_canonical_schema_and_legacy_mapping() {
    let attempt = test_attempt("coding_attempt_prompt_contract");
    let context = CodingExecutionContext::default();
    let test_plan = TestPlan {
        id: "test_plan_prompt_contract".to_string(),
        attempt_id: attempt.id.clone(),
        role_run_id: None,
        run_no: None,
        summary: "prompt contract".to_string(),
        context_warnings: Vec::new(),
        assumptions: Vec::new(),
        steps: Vec::new(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
    };

    for prompt in [
        build_coding_prompt(&attempt, &context, None, None),
        crate::product::tester_agent_loop::build_tester_system_prompt(&attempt, &context, &[]),
        build_tester_execute_plan_prompt(&attempt, &test_plan, "{}"),
        build_tester_execute_repair_prompt("{}", &["step_1".to_string()]),
    ] {
        assert_plan_defect_output_contract(&prompt, "plan_defect_findings");
        assert!(prompt.contains("普通 implementation defect"));
    }

    for prompt in [
        code_review_material_protocol(),
        group_final_review_material_protocol(),
    ] {
        assert_plan_defect_output_contract(prompt, "findings");
        assert!(prompt.contains("普通 implementation defect"));
    }
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
