use serde_json::json;

use crate::product::coding_models::{
    CodeReviewReport, CodingExecutionStage, CodingGateAction, CodingGateActionType, CodingGateKind,
    CodingGateRequired, CodingProviderPermissionMode, CodingProviderRole,
    CodingRoleProviderConfigSnapshot, FindingSeverity, InternalPrReview, ReviewFinding,
    ReviewVerdict, TestCommandStatus, TestPlan, TestPlanRiskLevel, TestPlanStep, TestPlanTool,
    TestingOverallStatus, TestingReport, TestingStepResult,
};

#[test]
fn role_provider_config_deserializes_legacy_json_with_default_permission_modes() {
    let legacy = r#"{
      "coder": "codex",
      "tester": "claude_code",
      "analyst": "claude_code",
      "code_reviewer": "codex",
      "internal_reviewer": "claude_code",
      "review_rounds": 1
    }"#;

    let snapshot: CodingRoleProviderConfigSnapshot =
        serde_json::from_str(legacy).expect("legacy role config");

    assert_eq!(
        snapshot.permission_mode_for_role(&CodingProviderRole::Coder),
        CodingProviderPermissionMode::Supervised
    );
    assert_eq!(
        snapshot.permission_mode_for_role(&CodingProviderRole::Tester),
        CodingProviderPermissionMode::Auto
    );
    assert_eq!(
        snapshot.permission_mode_for_role(&CodingProviderRole::Analyst),
        CodingProviderPermissionMode::Auto
    );
    assert_eq!(
        snapshot.permission_mode_for_role(&CodingProviderRole::CodeReviewer),
        CodingProviderPermissionMode::Supervised
    );
    assert_eq!(
        snapshot.permission_mode_for_role(&CodingProviderRole::InternalReviewer),
        CodingProviderPermissionMode::Supervised
    );
}

#[test]
fn test_plan_and_testing_report_round_trip_preserve_step_evidence() {
    let plan = TestPlan {
        id: "test_plan_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        summary: "unit and smoke checks".to_string(),
        context_warnings: vec!["missing_design_spec".to_string()],
        assumptions: vec!["target repo is already checked out".to_string()],
        steps: vec![TestPlanStep {
            id: "unit".to_string(),
            title: "Unit tests".to_string(),
            intent: "verify local unit behavior".to_string(),
            required: true,
            tool: TestPlanTool::RunCommand,
            risk_level: TestPlanRiskLevel::Low,
            command_or_tool_input: json!({"command": ["cargo", "test", "--locked"]}),
            evidence_expectation: "exit 0 and stdout/stderr refs".to_string(),
            related_requirements: vec!["REQ-1".to_string()],
            related_design_constraints: vec!["DES-1".to_string()],
            related_work_item_tasks: vec!["TASK-1".to_string()],
        }],
        created_at: "2026-06-10T00:00:00Z".to_string(),
        raw_provider_output_ref: Some("provider-raw/testing/plan_tests_0001.txt".to_string()),
    };
    let plan_value = serde_json::to_value(&plan).expect("serialize test plan");
    assert_eq!(plan_value["steps"][0]["tool"], "run_command");

    let report = TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        commands: Vec::new(),
        overall_status: TestingOverallStatus::PassedWithWarnings,
        provider_claim: Some(json!({"summary": "passed with warnings"})),
        backend_verified: true,
        started_at: "2026-06-10T00:00:00Z".to_string(),
        completed_at: Some("2026-06-10T00:00:01Z".to_string()),
        plan_id: Some("test_plan_0001".to_string()),
        plan_summary: Some("unit and smoke checks".to_string()),
        steps: vec![TestingStepResult {
            step_id: "unit".to_string(),
            status: TestCommandStatus::Passed,
            evidence_refs: vec!["unit.stdout.log".to_string()],
            command: Some(vec![
                "cargo".to_string(),
                "test".to_string(),
                "--locked".to_string(),
            ]),
            provider_analysis: Some("unit tests passed".to_string()),
        }],
        unplanned_commands: Vec::new(),
        unplanned_evidence: Vec::new(),
        missing_required_steps: Vec::new(),
        skipped_required_steps: vec!["security".to_string()],
        context_warnings: vec!["missing_design_spec".to_string()],
        raw_provider_output_ref: Some(
            "provider-raw/testing/execute_test_plan_0001.txt".to_string(),
        ),
    };
    let report_value = serde_json::to_value(&report).expect("serialize testing report");
    assert_eq!(report_value["overall_status"], "passed_with_warnings");
    assert_eq!(report_value["steps"][0]["step_id"], "unit");

    let review = CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        round: 1,
        verdict: ReviewVerdict::RequestChanges,
        findings: vec![ReviewFinding {
            severity: FindingSeverity::Warning,
            file_path: Some("src/lib.rs".to_string()),
            line: Some(42),
            message: "missing validation".to_string(),
            required_action: Some("add validation".to_string()),
            source_stage: CodingExecutionStage::CodeReview,
            evidence: vec!["diff:src/lib.rs".to_string()],
            related_requirements: vec!["REQ-1".to_string()],
            related_design_constraints: vec!["DES-1".to_string()],
            related_work_item_tasks: vec!["TASK-1".to_string()],
        }],
        tested_evidence_refs: vec!["testing_report_0001.json".to_string()],
        diff_refs: vec!["attempt.diff".to_string()],
        summary: "needs validation".to_string(),
        created_at: "2026-06-10T00:00:02Z".to_string(),
        raw_provider_output_ref: Some("provider-raw/code_review/code_review_0001.txt".to_string()),
        role_run_id: None,
        run_no: None,
    };
    let review_value = serde_json::to_value(&review).expect("serialize code review");
    assert_eq!(
        review_value["raw_provider_output_ref"],
        "provider-raw/code_review/code_review_0001.txt"
    );

    let gate = CodingGateRequired {
        gate_id: "coding_blocked_gate_0001".to_string(),
        kind: CodingGateKind::Blocked,
        title: "Review blocked".to_string(),
        description: "review payload could not be parsed".to_string(),
        stage: Some(CodingExecutionStage::CodeReview),
        role: Some(CodingProviderRole::CodeReviewer),
        expires_at: None,
        provider_snapshot: None,
        available_actions: vec![CodingGateAction {
            action_id: "retry_review".to_string(),
            label: "重试审查".to_string(),
            action_type: CodingGateActionType::RetryReview,
        }],
        reason_code: Some("review_payload_parse_error".to_string()),
        evidence_refs: vec!["code_review_0001.json".to_string()],
        raw_provider_output_ref: Some("provider-raw/code_review/code_review_0001.txt".to_string()),
    };
    let gate_value = serde_json::to_value(&gate).expect("serialize gate");
    assert_eq!(gate_value["reason_code"], "review_payload_parse_error");
    assert_eq!(gate_value["evidence_refs"][0], "code_review_0001.json");
    assert_eq!(
        gate_value["raw_provider_output_ref"],
        "provider-raw/code_review/code_review_0001.txt"
    );
}

#[test]
fn legacy_coding_qa_records_deserialize_with_defaults() {
    let legacy_testing_report = r#"{
      "id": "testing_report_0001",
      "attempt_id": "coding_attempt_0001",
      "commands": [],
      "overall_status": "passed",
      "provider_claim": null,
      "backend_verified": true,
      "started_at": "2026-06-10T00:00:00Z",
      "completed_at": "2026-06-10T00:00:01Z"
    }"#;

    let report: TestingReport = serde_json::from_str(legacy_testing_report).unwrap();
    assert_eq!(report.plan_id, None);
    assert!(report.steps.is_empty());
    assert!(report.missing_required_steps.is_empty());
    assert_eq!(report.raw_provider_output_ref, None);

    let legacy_code_review = r#"{
      "id": "code_review_0001",
      "attempt_id": "coding_attempt_0001",
      "round": 1,
      "verdict": "request_changes",
      "findings": [
        {
          "severity": "warning",
          "file_path": "src/lib.rs",
          "line": 42,
          "message": "missing validation",
          "required_action": "add validation"
        }
      ],
      "tested_evidence_refs": [],
      "diff_refs": [],
      "summary": "needs validation",
      "created_at": "2026-06-10T00:00:02Z"
    }"#;
    let review: CodeReviewReport = serde_json::from_str(legacy_code_review).unwrap();
    assert_eq!(review.raw_provider_output_ref, None);
    assert_eq!(
        review.findings[0].source_stage,
        CodingExecutionStage::CodeReview
    );
    assert!(review.findings[0].evidence.is_empty());
    assert!(review.findings[0].related_requirements.is_empty());
    assert!(review.findings[0].related_design_constraints.is_empty());
    assert!(review.findings[0].related_work_item_tasks.is_empty());

    let legacy_internal_review = r#"{
      "id": "internal_pr_review_0001",
      "attempt_id": "coding_attempt_0001",
      "review_request_id": "review_request_0001",
      "verdict": "approve",
      "findings": [],
      "impact_scope": [],
      "pr_description": "ready",
      "commit_message_suggestion": "feat: ready",
      "tested_evidence_refs": [],
      "diff_refs": [],
      "summary": "ready",
      "created_at": "2026-06-10T00:00:03Z"
    }"#;
    let internal_review: InternalPrReview = serde_json::from_str(legacy_internal_review).unwrap();
    assert_eq!(internal_review.raw_provider_output_ref, None);

    let legacy_gate = r#"{
      "gate_id": "coding_gate_0001",
      "kind": "stage_gate",
      "title": "Confirm",
      "description": "confirm stage",
      "stage": "testing",
      "role": "tester",
      "expires_at": null,
      "provider_snapshot": null,
      "available_actions": []
    }"#;
    let gate: CodingGateRequired = serde_json::from_str(legacy_gate).unwrap();
    assert_eq!(gate.reason_code, None);
    assert!(gate.evidence_refs.is_empty());
    assert_eq!(gate.raw_provider_output_ref, None);
}
