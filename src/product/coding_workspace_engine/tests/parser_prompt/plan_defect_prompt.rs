use super::*;

#[test]
fn tester_execute_prompt_blocks_insufficient_test_plan_without_replanning() {
    let plan = TestPlan {
        id: "test_plan_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        summary: "execute fixed plan".to_string(),
        context_warnings: Vec::new(),
        assumptions: Vec::new(),
        steps: vec![crate::product::coding_models::TestPlanStep {
            id: "step_t1".to_string(),
            title: "read targeted context".to_string(),
            intent: "verify available context".to_string(),
            required: true,
            tool: crate::product::coding_models::TestPlanTool::ReadFile,
            risk_level: crate::product::coding_models::TestPlanRiskLevel::Low,
            command_or_tool_input: serde_json::json!({"path": "src/lib.rs"}),
            evidence_expectation: "source evidence".to_string(),
            related_requirements: vec!["REQ-001".to_string()],
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
        }],
        created_at: "2026-06-10T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
    };

    let prompt = build_tester_execute_plan_prompt(
        &test_attempt("coding_attempt_0001"),
        &plan,
        r#"{"source_artifacts":{"design_specs":[]}}"#,
    );

    assert!(prompt.contains("Do not generate new TestPlan steps during execute_test_plan"));
    assert!(prompt.contains("provider_analysis prefixed by \"test_plan_insufficient:\""));
    assert!(prompt.contains("mark the affected required step blocked"));
    assert!(prompt.contains("[cadence_project_rules]"));
    assert!(prompt.contains("AGENTS.md"));
    assert!(prompt.contains("CLAUDE.md"));
    assert!(!prompt.contains("Cadence-skills/"));
    assert!(prompt.contains("verification-before-completion"));
    assert!(!prompt.contains("cadence-workflow"));
}

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
        assert_plan_defect_output_contract(&prompt, "findings");
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
