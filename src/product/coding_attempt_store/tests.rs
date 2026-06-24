use tempfile::TempDir;

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{
    CodingExecutionStage, CodingGateAction, CodingGateActionType, CodingProviderRole, TestPlan,
    TestPlanRiskLevel, TestPlanStep, TestPlanTool,
};
use crate::product::models::ProviderName;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const WORK_ITEM_ID: &str = "work_item_0001";

fn setup() -> (TempDir, CodingAttemptStore, CodingExecutionAttempt) {
    let tmp = TempDir::new().unwrap();
    let store = CodingAttemptStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .unwrap();
    (tmp, store, attempt)
}

#[test]
fn persists_test_plan_raw_output_and_blocked_gate() {
    let (_tmp, store, attempt) = setup();

    let raw_ref = store
        .save_provider_raw_output(
            &attempt.id,
            CodingExecutionStage::Testing,
            "plan_tests",
            "raw test plan output",
        )
        .unwrap();
    assert_eq!(raw_ref, "provider-raw/testing/plan_tests_0001.txt");

    let plan = TestPlan {
        id: "test_plan_0001".to_string(),
        attempt_id: attempt.id.clone(),
        role_run_id: None,
        run_no: None,
        summary: "unit tests".to_string(),
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
        raw_provider_output_ref: Some(raw_ref.clone()),
    };
    store.save_test_plan(&plan).unwrap();
    let plans = store
        .list_test_plans(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0].raw_provider_output_ref.as_deref(),
        Some(raw_ref.as_str())
    );

    let gate = store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            node_id: Some("coding_node_0001".to_string()),
            role: Some(CodingProviderRole::Tester),
            title: "Testing blocked".to_string(),
            description: "required step missing".to_string(),
            reason_code: Some("missing_required_steps".to_string()),
            evidence_refs: vec!["testing_report_0001.json".to_string()],
            raw_provider_output_ref: Some(raw_ref),
            available_actions: vec![CodingGateAction {
                action_id: "retry_test_plan".to_string(),
                label: "重试测试计划".to_string(),
                action_type: CodingGateActionType::RetryTestPlan,
            }],
        })
        .unwrap();
    let open = store
        .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(
        open[0].reason_code.as_deref(),
        Some("missing_required_steps")
    );
    assert_eq!(open[0].evidence_refs, vec!["testing_report_0001.json"]);
    assert_eq!(
        open[0].available_actions[0].action_type,
        CodingGateActionType::RetryTestPlan
    );

    store
        .resolve_blocked_gate(PROJECT_ID, ISSUE_ID, &attempt.id, &gate.gate_id)
        .unwrap();
    let open = store
        .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap();
    assert!(open.is_empty());
}

#[test]
fn blocked_gate_creation_is_idempotent_for_same_node_and_reason() {
    let (_tmp, store, attempt) = setup();
    let first = store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            node_id: Some("coding_node_0001".to_string()),
            role: Some(CodingProviderRole::Tester),
            title: "Testing blocked".to_string(),
            description: "required step missing".to_string(),
            reason_code: Some("missing_required_steps".to_string()),
            evidence_refs: vec!["testing_report_0001.json".to_string()],
            raw_provider_output_ref: None,
            available_actions: vec![CodingGateAction {
                action_id: "retry_test_plan".to_string(),
                label: "重试测试计划".to_string(),
                action_type: CodingGateActionType::RetryTestPlan,
            }],
        })
        .unwrap();

    let second = store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            node_id: Some("coding_node_0001".to_string()),
            role: Some(CodingProviderRole::Tester),
            title: "Testing still blocked".to_string(),
            description: "required step still missing".to_string(),
            reason_code: Some("missing_required_steps".to_string()),
            evidence_refs: vec![
                "testing_report_0001.json".to_string(),
                "testing_report_0002.json".to_string(),
            ],
            raw_provider_output_ref: None,
            available_actions: vec![CodingGateAction {
                action_id: "rerun_missing_steps".to_string(),
                label: "补跑缺失步骤".to_string(),
                action_type: CodingGateActionType::RerunMissingSteps,
            }],
        })
        .unwrap();

    assert_eq!(second.gate_id, first.gate_id);
    let open = store
        .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(
        open[0].evidence_refs,
        vec!["testing_report_0001.json", "testing_report_0002.json"]
    );
    assert_eq!(
        open[0].available_actions[0].action_type,
        CodingGateActionType::RerunMissingSteps
    );
}
