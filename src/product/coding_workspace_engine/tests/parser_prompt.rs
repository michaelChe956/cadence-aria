use super::*;

#[test]
fn derive_reason_code_prefers_explicit() {
    let report = blocked_report_with(Vec::new(), vec!["S018".to_string()]);
    let reason = derive_testing_blocked_reason_code(
        Some("high_risk_test_step_requires_permission".to_string()),
        &report,
    );
    assert_eq!(reason, "high_risk_test_step_requires_permission");
}

#[test]
fn derive_reason_code_uses_missing_when_present() {
    let report = blocked_report_with(vec!["unit".to_string()], vec!["S018".to_string()]);
    assert_eq!(
        derive_testing_blocked_reason_code(None, &report),
        "missing_required_steps"
    );
}

#[test]
fn derive_reason_code_uses_skipped_when_only_skipped() {
    let report = blocked_report_with(Vec::new(), vec!["S018".to_string(), "S027".to_string()]);
    assert_eq!(
        derive_testing_blocked_reason_code(None, &report),
        "skipped_required_steps"
    );
}

#[test]
fn derive_reason_code_falls_back_to_testing_blocked() {
    let report = blocked_report_with(Vec::new(), Vec::new());
    assert_eq!(
        derive_testing_blocked_reason_code(None, &report),
        "testing_blocked"
    );
}

#[test]
fn coding_prompt_guides_pnpm_install_when_frontend_dependencies_are_missing() {
    let attempt = test_attempt("coding_attempt_0001");
    let context = CodingExecutionContext::default();

    let prompt = build_coding_prompt(&attempt, &context, None, None);

    assert!(prompt.contains("node_modules missing"));
    assert!(prompt.contains("tsc EACCES"));
    assert!(prompt.contains("vitest EACCES"));
    assert!(prompt.contains("pnpm --version"));
    assert!(prompt.contains("pnpm -C <package-dir> install --frozen-lockfile"));
    assert!(prompt.contains("不要把缺少 node_modules 误判为 pnpm 不可用"));
}

#[test]
fn coding_delta_prompt_guides_pnpm_install_when_frontend_dependencies_are_missing() {
    let attempt = test_attempt("coding_attempt_0001");
    let context = CodingExecutionContext::default();

    let prompt = build_coding_delta_prompt(&attempt, &context, None, None);

    assert!(prompt.contains("node_modules missing"));
    assert!(prompt.contains("pnpm -C <package-dir> install --frozen-lockfile"));
    assert!(prompt.contains("不要把缺少 node_modules 误判为 pnpm 不可用"));
}

#[test]
fn review_parser_preserves_findings_with_common_aliases() {
    let payload = r#"{
      "verdict": "request_changes",
      "summary": "needs changes",
      "findings": [
        {
          "file": "src/lib.rs",
          "line": 42,
          "description": "missing validation",
          "recommendation": "add validation"
        }
      ]
    }"#;

    let parsed = parse_review_payload(payload, CodingExecutionStage::CodeReview);

    assert_eq!(parsed.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(
        parsed.findings[0].severity,
        crate::product::coding_models::FindingSeverity::Warning
    );
    assert_eq!(
        parsed.findings[0].source_stage,
        CodingExecutionStage::CodeReview
    );
    assert_eq!(parsed.findings[0].file_path.as_deref(), Some("src/lib.rs"));
    assert_eq!(parsed.findings[0].message, "missing validation");
    assert_eq!(
        parsed.findings[0].required_action.as_deref(),
        Some("add validation")
    );
}

#[test]
fn rework_and_internal_review_prompts_require_openspec_and_superpowers() {
    let attempt = test_attempt("coding_attempt_0001");
    let context_notes = ReworkContextNoteInput {
        text: "manual context".to_string(),
        truncated: false,
    };
    let prompt = build_rework_prompt(
        &attempt,
        "testing blocked",
        &CodingExecutionStage::Testing,
        1,
        &context_notes,
        "{}",
        None,
    );

    assert!(prompt.contains("[openspec_contract]"));
    assert!(prompt.contains("[superpowers_contract]"));
    assert!(prompt.contains("Story Spec"));
    assert!(prompt.contains("Design Spec"));
    assert!(prompt.contains("Work Item"));

    let internal_contract = provider_runtime_contract("InternalReviewer");
    assert!(internal_contract.contains("InternalReviewer"));
    assert!(internal_contract.contains("[openspec_contract]"));
    assert!(internal_contract.contains("[superpowers_contract]"));
}

#[test]
fn dangerous_test_plan_step_requires_permission_or_blocks() {
    let plan = TestPlan {
        id: "test_plan_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        summary: "dangerous checks".to_string(),
        context_warnings: Vec::new(),
        assumptions: Vec::new(),
        steps: vec![crate::product::coding_models::TestPlanStep {
            id: "destructive".to_string(),
            title: "destructive command".to_string(),
            intent: "should require approval".to_string(),
            required: true,
            tool: crate::product::coding_models::TestPlanTool::RunCommand,
            risk_level: crate::product::coding_models::TestPlanRiskLevel::High,
            command_or_tool_input: serde_json::json!({
                "command": ["rm", "-rf", "/tmp/some-target"]
            }),
            evidence_expectation: "must not run without approval".to_string(),
            related_requirements: Vec::new(),
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
        }],
        created_at: "2026-06-10T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
    };
    let call = ProviderToolCall {
        id: "run_command_0001".to_string(),
        tool_name: "run_command".to_string(),
        input: serde_json::json!({
            "step_id": "destructive",
            "command": ["rm", "-rf", "/tmp/some-target"]
        }),
    };

    assert_eq!(
        high_risk_test_step_block_reason(&plan, &call),
        Some("high_risk_test_step_requires_permission")
    );
}
