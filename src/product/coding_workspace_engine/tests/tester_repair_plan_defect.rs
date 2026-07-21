use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct TesterRepairPlanDefectProvider {
    conflict_on_repair: bool,
    calls: AtomicUsize,
}

impl TesterRepairPlanDefectProvider {
    fn new(conflict_on_repair: bool) -> Self {
        Self {
            conflict_on_repair,
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for TesterRepairPlanDefectProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let output = if input.prompt.contains("Phase: plan_tests") {
            serde_json::json!({
                "summary": "two required steps",
                "steps": [
                    test_plan_step("unit"),
                    test_plan_step("integration")
                ]
            })
        } else {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            match call {
                0 => serde_json::json!({
                    "step_results": [test_step_result("unit")],
                    "plan_defect_findings": [plan_finding("work_item_revision_0001")]
                }),
                _ if self.conflict_on_repair => serde_json::json!({
                    "step_results": [test_step_result("integration")],
                    "plan_defect_findings": [plan_finding("work_item_revision_9999")]
                }),
                _ => serde_json::json!({
                    "step_results": [test_step_result("integration")]
                }),
            }
        }
        .to_string();
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        output, None,
                    ),
                ))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

fn test_plan_step(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": id,
        "intent": "verify behavior",
        "required": true,
        "tool": "provider_managed",
        "risk_level": "low",
        "command_or_tool_input": {"command": ["true"]},
        "evidence_expectation": "provider evidence",
        "related_requirements": [format!("REQ-{id}")],
        "related_design_constraints": [format!("DEC-{id}")],
        "related_work_item_tasks": [format!("TASK-{id}")]
    })
}

fn test_step_result(id: &str) -> serde_json::Value {
    serde_json::json!({
        "step_id": id,
        "status": "passed",
        "evidence_refs": [format!("{id}.log")],
        "provider_analysis": format!("{id} passed")
    })
}

fn plan_finding(revision_id: &str) -> serde_json::Value {
    serde_json::json!({
        "finding_id": "tester_repair_finding_0001",
        "severity": "error",
        "defect_class": "current_work_item_invalid",
        "reason_code": "current_work_item_contract_invalid",
        "message": "the current plan cannot be tested",
        "evidence": [{
            "kind": "test_execution",
            "source_ref": "unit.log",
            "message": "unit exposed the plan defect"
        }],
        "contract_refs": [],
        "capability_refs": [],
        "repair_target": {
            "kind": "current_work_item",
            "logical_work_item_ids": ["work_item_0001"],
            "work_item_revision_ids": [revision_id]
        },
        "recommended_route": "plan_repair",
        "confidence": "high"
    })
}

#[tokio::test]
async fn coding_plan_repair_tester_repair_preserves_initial_findings_and_order() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &TesterRepairPlanDefectProvider::new(false),
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("provider-driven tester repair");

    assert_eq!(report.steps.len(), 2);
    assert_eq!(report.plan_defect_findings.len(), 1);
    assert_eq!(
        report.plan_defect_findings[0].finding_id,
        "tester_repair_finding_0001"
    );
    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    let updated = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Testing);
}

#[tokio::test]
async fn coding_plan_repair_tester_repair_rejects_conflicting_finding_identity() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let error = engine
        .execute_testing_with_provider(
            &attempt,
            &TesterRepairPlanDefectProvider::new(true),
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect_err("conflicting repair finding must fail closed");

    assert!(
        error
            .to_string()
            .contains("tester_plan_defect_identity_conflict")
    );
}
