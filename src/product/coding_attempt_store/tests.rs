use tempfile::TempDir;

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{
    CodingAttemptScope, CodingExecutionStage, CodingExecutionUnitStatus, CodingGateAction,
    CodingGateActionType, CodingProviderRole, TestPlan, TestPlanRiskLevel, TestPlanStep,
    TestPlanTool,
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

fn provider_snapshot() -> ProviderConfigSnapshot {
    ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::ClaudeCode),
        review_rounds: 1,
    }
}

#[test]
fn legacy_attempt_without_scope_deserializes_as_work_item_scope() {
    let json = serde_json::json!({
        "id": "coding_attempt_0001",
        "project_id": "project_0001",
        "issue_id": "issue_0001",
        "work_item_id": "work_item_0001",
        "attempt_no": 1,
        "status": "created",
        "stage": "prepare_context",
        "base_branch": "main",
        "branch_name": "aria/issues/issue_0001",
        "worktree_path": null,
        "provider_config_snapshot": { "author": "codex", "reviewer": "codex", "review_rounds": 1 },
        "rework_count": 0,
        "max_auto_rework": 2,
        "head_commit": null,
        "pushed_remote": null,
        "review_request_id": null,
        "provider_conversations": [],
        "created_at": "2026-06-27T00:00:00Z",
        "updated_at": "2026-06-27T00:00:00Z",
        "completed_at": null
    });

    let attempt: CodingExecutionAttempt = serde_json::from_value(json).expect("attempt");

    assert_eq!(attempt.scope, CodingAttemptScope::WorkItem);
    assert_eq!(
        attempt.current_work_item_id.as_deref(),
        Some("work_item_0001")
    );
    assert!(attempt.work_item_group_id.is_none());
}

#[test]
fn creates_group_attempt_and_units_with_single_active_unit() {
    let (_tmp, store, _attempt) = setup();

    let group_attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("group attempt");

    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("unit 1");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0002".to_string(),
            order_index: 1,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("unit 2");

    let units = store
        .list_coding_units("project_0001", "issue_0001", &group_attempt.id)
        .expect("units");
    let active = store
        .get_active_coding_unit("project_0001", "issue_0001", &group_attempt.id)
        .expect("active lookup")
        .expect("active");

    assert_eq!(group_attempt.scope, CodingAttemptScope::WorkItemGroup);
    assert_eq!(
        group_attempt.work_item_group_id.as_deref(),
        Some("work_item_plan_0001")
    );
    assert_eq!(units.len(), 2);
    assert_eq!(active.work_item_id, "work_item_0001");
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
