use std::path::PathBuf;

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage, TestCommand,
    TestCommandStatus, TestPlan, TestPlanRiskLevel, TestPlanStep, TestPlanTool,
    TestingOverallStatus, TestingStepResult,
};
use crate::product::lifecycle_store::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateStorySpecInput, CreateWorkItemInput,
    LifecycleStore,
};
use crate::product::models::ProviderName;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use tempfile::TempDir;

fn test_attempt() -> CodingExecutionAttempt {
    CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::Running,
        stage: CodingExecutionStage::Testing,
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
        },
        rework_count: 0,
        max_auto_rework: 2,
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        provider_conversations: Vec::new(),
        created_at: "2026-06-10T00:00:00Z".to_string(),
        updated_at: "2026-06-10T00:00:00Z".to_string(),
        completed_at: None,
    }
}

#[test]
fn coding_plan_repair_provider_test_execution_payload_roundtrip_preserves_plan_defect_findings() {
    let raw = serde_json::json!({
        "step_results": [{
            "step_id": "unit",
            "status": "blocked",
            "evidence_refs": ["unit.log"],
            "command": null,
            "provider_analysis": "test_plan_insufficient: contract is invalid"
        }],
        "plan_defect_findings": [{
            "finding_id": "tester_finding_0001",
            "severity": "error",
            "defect_class": "current_work_item_invalid",
            "reason_code": "current_work_item_contract_invalid",
            "message": "the current work item contract is not testable",
            "evidence": [{
                "kind": "test_execution",
                "source_ref": "unit.log",
                "message": "the required contract cannot be exercised"
            }],
            "contract_refs": ["contract.current"],
            "capability_refs": ["testability"],
            "repair_target": {
                "kind": "current_work_item",
                "logical_work_item_ids": ["work_item_0001"],
                "work_item_revision_ids": ["work_item_revision_0001"]
            },
            "recommended_route": "plan_repair",
            "confidence": "high"
        }]
    });

    let payload: ProviderTestExecutionPayload =
        serde_json::from_value(raw.clone()).expect("canonical tester execution payload");
    let roundtrip = serde_json::to_value(payload).expect("serialize tester execution payload");

    assert_eq!(roundtrip, raw);
}

#[test]
fn tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools() {
    let prompt = build_tester_plan_prompt(
        &test_attempt(),
        r#"{"story_specs":[],"design_specs":[],"work_item":{}}"#,
        None,
    );

    assert!(prompt.contains("plan_tests"));
    assert!(prompt.contains("execute_test_plan"));
    assert!(prompt.contains("[openspec_contract]"));
    assert!(prompt.contains("[superpowers_contract]"));
    assert!(prompt.contains("[cadence_project_rules]"));
    assert!(prompt.contains("AGENTS.md"));
    assert!(prompt.contains("CLAUDE.md"));
    assert!(!prompt.contains(&["Cadence-", "skills/"].concat()));
    assert!(prompt.contains("verification-before-completion"));
    assert!(!prompt.contains("cadence-workflow"));
    assert!(prompt.contains("Story Spec"));
    assert!(prompt.contains("Design Spec"));
    assert!(prompt.contains("Work Item"));
    assert!(prompt.contains("actual Work Item"));
    assert!(prompt.contains("related_work_item_tasks"));
    assert!(prompt.contains("不要按通用模板生成固定步骤"));
    assert!(prompt.contains("禁止生成 `cargo test --locked --lib filter_a filter_b`"));
    assert!(prompt.contains("step_id"));
    assert!(prompt.contains("不要硬编码某种语言或包管理器"));
    assert!(!prompt.contains("[retry_diagnostic]"));
    assert!(!prompt.contains("[previous_role_run_diagnostic]"));
    assert!(prompt.contains("CRITICAL: Return ONLY a single JSON object"));
    assert!(
        prompt
            .trim_end()
            .ends_with("END OF INSTRUCTIONS: output JSON only.")
    );

    let repair = build_tester_plan_repair_prompt("{bad", "invalid_json");
    assert!(!repair.contains("[cadence_project_rules]"));
    assert!(repair.contains("Return ONLY a single JSON object"));

    let execute_repair = build_tester_execute_repair_prompt("{bad", &["step_t1".to_string()]);
    assert!(!execute_repair.contains("[cadence_project_rules]"));
    assert!(!execute_repair.contains("当前阶段："));
    assert!(!execute_repair.contains("using-superpowers"));
}

#[test]
fn load_test_context_input_requires_step_reason_selectors_and_rejects_full_mode() {
    let input = serde_json::json!({
        "step_id": "step_t1",
        "reason": "Need exact DEC-006 package metadata",
        "artifact_refs": ["design_spec_0001@version_0002"],
        "selectors": ["DEC-006", "CMP-001"]
    });

    let parsed = super::tools::parse_load_test_context_input(&input).expect("valid input");

    assert_eq!(parsed.step_id, "step_t1");
    assert_eq!(parsed.reason, "Need exact DEC-006 package metadata");
    assert_eq!(parsed.artifact_refs, vec!["design_spec_0001@version_0002"]);
    assert_eq!(parsed.selectors, vec!["DEC-006", "CMP-001"]);

    let missing_step = serde_json::json!({
        "reason": "Need exact DEC-006 package metadata",
        "selectors": ["DEC-006"]
    });
    assert!(super::tools::parse_load_test_context_input(&missing_step).is_err());

    let too_many_selectors = serde_json::json!({
        "step_id": "step_t1",
        "reason": "Need exact DEC-006 package metadata",
        "selectors": ["S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8", "S9"]
    });
    assert!(super::tools::parse_load_test_context_input(&too_many_selectors).is_err());

    let full_mode = serde_json::json!({
        "step_id": "step_t1",
        "reason": "Need exact DEC-006 package metadata",
        "mode": "full",
        "selectors": ["DEC-006"]
    });
    assert!(super::tools::parse_load_test_context_input(&full_mode).is_err());
}

#[test]
fn load_test_context_returns_targeted_design_snippets_without_full_markdown() {
    let tmp = TempDir::new().expect("tmp");
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Story".to_string(),
        })
        .expect("story");
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "Design".to_string(),
        })
        .expect("design");
    lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: design.id.clone(),
            markdown: "# Design\n\n前置背景不应整体返回\n\n[DEC-006] provider package is @openai/codex.\n\n全文之外的段落".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: Some("user".to_string()),
        })
        .expect("design version");
    lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec![story.id],
            design_spec_ids: vec![design.id.clone()],
            title: "Work Item".to_string(),
            ..Default::default()
        })
        .expect("work item");

    let input = super::tools::parse_load_test_context_input(&serde_json::json!({
        "step_id": "step_t1",
        "reason": "Need exact DEC-006 package metadata",
        "artifact_refs": [format!("{}@version_0001", design.id)],
        "selectors": ["DEC-006"]
    }))
    .expect("input");
    let result = super::context_loader::TestContextLoader::new(paths, test_attempt())
        .load(&input)
        .expect("snippets");

    assert_eq!(result.snippets.len(), 1);
    assert_eq!(result.snippets[0].selector, "DEC-006");
    assert!(result.snippets[0].text.contains("@openai/codex"));
    assert!(!result.snippets[0].text.contains("全文之外的段落"));
    assert!(result.warnings.is_empty());
}

#[test]
fn rejects_test_plan_steps_without_work_item_traceability() {
    let raw_output = r#"
{
  "summary": "generic checks",
  "context_warnings": [],
  "assumptions": [],
  "steps": [
    {
      "id": "unit",
      "title": "Run generic unit tests",
      "intent": "run a generic test command without linking it to the work item",
      "required": true,
      "tool": "run_command",
      "risk_level": "low",
      "command_or_tool_input": { "command": "cargo test --locked" },
      "evidence_expectation": "tests pass",
      "related_requirements": [],
      "related_design_constraints": [],
      "related_work_item_tasks": []
    }
  ]
}
"#;

    let error = parse_test_plan_payload("coding_attempt_0001", "test_plan_0001", raw_output, None)
        .expect_err("generic plan should be rejected")
        .to_string();

    assert!(error.contains("step_traceability_empty: unit"));
}

#[test]
fn rejects_cargo_lib_command_with_multiple_test_filters() {
    let raw_output = r#"
{
  "summary": "invalid cargo command",
  "context_warnings": [],
  "assumptions": [],
  "steps": [
    {
      "id": "unit",
      "title": "Unit tests",
      "intent": "run targeted tests",
      "required": true,
      "tool": "run_command",
      "risk_level": "low",
      "command_or_tool_input": {
        "command": "cargo test --locked --lib provider_catalog provider_probe"
      },
      "evidence_expectation": "exit 0",
      "related_requirements": ["REQ-001"],
      "related_design_constraints": ["DEC-001"],
      "related_work_item_tasks": ["TASK-001"]
    }
  ]
}
"#;

    let error = parse_test_plan_payload("coding_attempt_0001", "test_plan_0001", raw_output, None)
        .expect_err("cargo command with multiple filters should be rejected")
        .to_string();

    assert!(error.contains("cargo_lib_multiple_filters: unit"));
}

#[test]
fn parses_test_plan_from_provider_json_and_blocks_missing_required_step() {
    let raw_output = r#"
Tester plan:

```json
{
  "summary": "unit and security checks",
  "context_warnings": [],
  "assumptions": [],
  "steps": [
    {
      "id": "unit",
      "title": "Unit tests",
      "intent": "verify unit behavior",
      "required": true,
      "tool": "run_command",
      "risk_level": "low",
      "command_or_tool_input": { "command": ["cargo", "test", "--locked", "--lib", "unit"] },
      "evidence_expectation": "exit 0",
      "related_requirements": ["REQ-UNIT"],
      "related_design_constraints": ["DEC-UNIT"],
      "related_work_item_tasks": ["TASK-UNIT"]
    },
    {
      "id": "security",
      "title": "Security review",
      "intent": "verify sensitive output handling",
      "required": true,
      "tool": "provider_managed",
      "risk_level": "medium",
      "command_or_tool_input": { "check": "manual" },
      "evidence_expectation": "provider analysis with evidence",
      "related_requirements": ["REQ-SECURITY"],
      "related_design_constraints": ["DEC-SECURITY"],
      "related_work_item_tasks": ["TASK-SECURITY"]
    }
  ]
}
```
"#;

    let plan = parse_test_plan_payload(
        "coding_attempt_0001",
        "test_plan_0001",
        raw_output,
        Some("provider-raw/testing/plan_tests_0001.txt".to_string()),
    )
    .unwrap();

    assert_eq!(plan.attempt_id, "coding_attempt_0001");
    assert_eq!(plan.id, "test_plan_0001");
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[0].id, "unit");
    assert_eq!(plan.steps[1].id, "security");

    let report = build_plan_based_testing_report(
        "testing_report_0001",
        "coding_attempt_0001",
        &plan,
        vec![TestingStepResult {
            step_id: "unit".to_string(),
            status: TestCommandStatus::Passed,
            evidence_refs: vec!["unit.stdout.log".to_string()],
            command: Some(vec![
                "cargo".to_string(),
                "test".to_string(),
                "--locked".to_string(),
                "--lib".to_string(),
                "unit".to_string(),
            ]),
            provider_analysis: None,
        }],
        Vec::new(),
        None,
        Some("provider-raw/testing/execute_tests_0001.txt".to_string()),
    );

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert_eq!(report.plan_id.as_deref(), Some("test_plan_0001"));
    assert_eq!(report.missing_required_steps, vec!["security"]);
}

#[test]
fn tester_plan_repair_prompt_includes_raw_output_and_schema_error() {
    let prompt =
        build_tester_plan_repair_prompt("## 最终测试报告\n无法执行 cargo", "missing_json_object");

    assert!(prompt.contains("Phase: plan_tests_repair"));
    assert!(prompt.contains("missing_json_object"));
    assert!(prompt.contains("## 最终测试报告"));
    assert!(prompt.contains("\"summary\""));
    assert!(prompt.contains("\"steps\""));
    assert!(prompt.contains("CRITICAL: Return ONLY a single JSON object"));
    assert!(prompt.contains("DO NOT output markdown headers"));
    assert!(prompt.contains("ERROR - this format was wrong"));
    assert!(
        prompt
            .trim_end()
            .ends_with("END OF INSTRUCTIONS: output JSON only.")
    );
}

#[test]
fn tester_plan_repair_prompt_truncates_long_raw_output_without_utf8_boundary_panic() {
    let long_output = "测试报告".repeat(400);
    let prompt = build_tester_plan_repair_prompt(&long_output, "invalid_json");

    assert!(prompt.contains("...[truncated"));
    assert!(!prompt.contains(&"测试报告".repeat(300)));
}

#[test]
fn test_tool_call_without_step_id_is_unplanned_and_does_not_pass_required_step() {
    let plan = TestPlan {
        id: "test_plan_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        summary: "unit checks".to_string(),
        context_warnings: Vec::new(),
        assumptions: Vec::new(),
        steps: vec![TestPlanStep {
            id: "unit".to_string(),
            title: "Unit tests".to_string(),
            intent: "verify unit behavior".to_string(),
            required: true,
            tool: TestPlanTool::RunCommand,
            risk_level: TestPlanRiskLevel::Low,
            command_or_tool_input: serde_json::json!({"command": ["true"]}),
            evidence_expectation: "exit 0".to_string(),
            related_requirements: Vec::new(),
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
        }],
        created_at: "2026-06-10T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
    };
    let unplanned_command = TestCommand {
        command: vec!["true".to_string()],
        cwd: PathBuf::from("/tmp/worktree"),
        exit_code: Some(0),
        duration_ms: 1,
        stdout_ref: "stdout.log".to_string(),
        stderr_ref: "stderr.log".to_string(),
        status: TestCommandStatus::Passed,
    };

    let report = build_plan_based_testing_report(
        "testing_report_0001",
        "coding_attempt_0001",
        &plan,
        Vec::new(),
        vec![unplanned_command],
        None,
        None,
    );

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert_eq!(report.missing_required_steps, vec!["unit"]);
    assert!(report.steps.is_empty());
    assert_eq!(report.unplanned_commands.len(), 1);
}

#[test]
fn test_plan_insufficient_blocked_result_blocks_without_missing_required_step() {
    let plan = TestPlan {
        id: "test_plan_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        summary: "context-sensitive checks".to_string(),
        context_warnings: Vec::new(),
        assumptions: Vec::new(),
        steps: vec![TestPlanStep {
            id: "step_missing_context".to_string(),
            title: "Verify unavailable design selector".to_string(),
            intent: "verify design selector".to_string(),
            required: true,
            tool: TestPlanTool::ProviderManaged,
            risk_level: TestPlanRiskLevel::Low,
            command_or_tool_input: serde_json::json!({}),
            evidence_expectation: "blocked explanation".to_string(),
            related_requirements: vec!["REQ-001".to_string()],
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
        }],
        created_at: "2026-06-10T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
    };

    let report = build_plan_based_testing_report(
        "testing_report_0001",
        "coding_attempt_0001",
        &plan,
        vec![TestingStepResult {
            step_id: "step_missing_context".to_string(),
            status: TestCommandStatus::Blocked,
            evidence_refs: Vec::new(),
            command: None,
            provider_analysis: Some(
                "test_plan_insufficient: selector DEC-014 unavailable".to_string(),
            ),
        }],
        Vec::new(),
        None,
        None,
    );

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(report.missing_required_steps.is_empty());
    assert_eq!(
        report.skipped_required_steps,
        vec!["step_missing_context".to_string()]
    );
}
