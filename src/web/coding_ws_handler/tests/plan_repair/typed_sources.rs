use super::*;
use crate::cross_cutting::streaming_provider::{ProviderCompletion, ProviderEvent};
use crate::product::coding_workspace_engine::{
    CodeReviewFlowDecision, CodingExecutionContext, ExecutionPlanDefectReport, PlanDefectSource,
};
use crate::product::plan_repair::{PlanDefectFinding, PlanDefectSeverity};
use crate::product::tester_agent_loop::TesterAgentOptions;
use crate::web::coding_ws_handler::start_plan_repair_for_execution_outcome_if_needed;
use std::fs;

struct PlanDefectOutputProvider {
    output: String,
}

struct TesterPlanDefectProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for PlanDefectOutputProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        let output = self.output.clone();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output, None,
                )))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for TesterPlanDefectProvider {
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
                "summary": "run the required unit check",
                "steps": [{
                    "id": "unit",
                    "title": "unit",
                    "intent": "verify behavior",
                    "required": true,
                    "tool": "provider_managed",
                    "risk_level": "low",
                    "command_or_tool_input": {"command": ["true"]},
                    "evidence_expectation": "provider evidence",
                    "related_requirements": ["REQ-unit"],
                    "related_design_constraints": ["DEC-unit"],
                    "related_work_item_tasks": ["TASK-unit"]
                }]
            })
        } else {
            serde_json::json!({
                "step_results": [{
                    "step_id": "unit",
                    "status": "passed",
                    "evidence_refs": ["unit.log"],
                    "provider_analysis": "unit exposed the plan defect"
                }],
                "plan_defect_findings": [canonical_execution_finding(
                    "tester_provider_canonical_finding"
                )]
            })
        }
        .to_string();
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output, None,
                )))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn coding_plan_repair_coder_outcome_preserves_report_and_starts_repair() {
    let fixture = plan_repair_fixture_with_dependency(false);
    fs::create_dir_all(fixture.attempt.worktree_path.as_ref().unwrap()).unwrap();
    let finding = canonical_execution_finding("coder_canonical_finding");
    let provider = PlanDefectOutputProvider {
        output: serde_json::json!({"plan_defect_findings": [finding]}).to_string(),
    };
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let outcome = fixture
        .engine
        .execute_coding_with_commands_outcome(
            &fixture.attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .unwrap();

    assert_eq!(
        outcome.plan_defect_decision,
        Some(CodeReviewFlowDecision::StartPlanRepair)
    );
    let report = outcome
        .plan_defect_report
        .expect("Coder outcome must retain the canonical report");
    assert_eq!(report.source, PlanDefectSource::Coder);
    assert_eq!(report.findings[0].finding_id, "coder_canonical_finding");
    let paused = fixture
        .engine
        .start_plan_repair_from_execution_report(&outcome.attempt, &report)
        .await
        .unwrap();
    assert_execution_request(&fixture, &paused, "coder_canonical_finding");
}

#[tokio::test]
async fn coding_plan_repair_coder_rework_outcome_preserves_report_and_starts_repair() {
    let fixture = plan_repair_fixture_with_dependency(false);
    fs::create_dir_all(fixture.attempt.worktree_path.as_ref().unwrap()).unwrap();
    let finding = canonical_execution_finding("coder_rework_canonical_finding");
    let provider = PlanDefectOutputProvider {
        output: serde_json::json!({"plan_defect_findings": [finding]}).to_string(),
    };
    let review = plan_defect_report(plan_defect_finding("review_rework"));
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let outcome = fixture
        .engine
        .execute_coder_fix_from_review_outcome(
            &fixture.attempt,
            &review,
            &CodingExecutionContext::default(),
            &provider,
            &mut command_rx,
        )
        .await
        .unwrap();

    assert_eq!(
        outcome.plan_defect_decision,
        Some(CodeReviewFlowDecision::StartPlanRepair)
    );
    let report = outcome
        .plan_defect_report
        .expect("CoderRework outcome must retain the canonical report");
    assert_eq!(report.source, PlanDefectSource::Coder);
    assert_eq!(
        report.findings[0].finding_id,
        "coder_rework_canonical_finding"
    );
    let paused = fixture
        .engine
        .start_plan_repair_from_execution_report(&outcome.attempt, &report)
        .await
        .unwrap();
    assert_execution_request(&fixture, &paused, "coder_rework_canonical_finding");
}

#[tokio::test]
async fn coding_plan_repair_tester_report_findings_start_repair_without_review_id() {
    let mut fixture = plan_repair_fixture();
    fs::create_dir_all(fixture.attempt.worktree_path.as_ref().expect("worktree")).unwrap();
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    for work_item_id in ["wi_upstream", "wi_current"] {
        lifecycle
            .create_work_item(crate::product::lifecycle_store::CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: fixture.attempt.project_id.clone(),
                issue_id: fixture.attempt.issue_id.clone(),
                repository_id: "repository_0001".to_string(),
                title: work_item_id.to_string(),
                work_item_set_id: Some(fixture.plan.id.clone()),
                plan_status: crate::product::models::WorkItemPlanStatus::Confirmed,
                ..Default::default()
            })
            .unwrap();
    }
    lifecycle
        .create_issue_work_item_plan(
            crate::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                id: Some(fixture.plan.id.clone()),
                project_id: fixture.attempt.project_id.clone(),
                issue_id: fixture.attempt.issue_id.clone(),
                source_story_spec_ids: Vec::new(),
                source_design_spec_ids: Vec::new(),
                options: crate::product::models::IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: crate::product::models::IssueWorkItemPlanStatus::Confirmed,
                work_item_ids: vec!["wi_upstream".to_string(), "wi_current".to_string()],
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            },
        )
        .unwrap();
    let mut attempt = fixture.attempt.clone();
    attempt.stage = CodingExecutionStage::Coding;
    fixture.store.save_coding_attempt(&attempt).unwrap();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let report = fixture
        .engine
        .execute_testing_with_provider_commands(
            &attempt,
            &TesterPlanDefectProvider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
            &mut command_rx,
        )
        .await
        .unwrap();

    assert_eq!(report.plan_defect_findings.len(), 1);
    assert_eq!(
        report.plan_defect_findings[0].finding_id,
        "tester_provider_canonical_finding"
    );
    let paused = fixture
        .store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();

    assert_execution_request(&fixture, &paused, "tester_provider_canonical_finding");
    let unit_run = fixture.store.get_active_unit_run(&paused).unwrap();
    assert_eq!(unit_run.status, CodingUnitRunStatus::BlockedByPlanDefect);
    assert_eq!(unit_run.plan_repair_count, 1);
    let mut saw_plan_repair_event = false;
    while let Ok(event) = fixture.event_rx.try_recv() {
        if matches!(event, CodingWsOutMessage::PlanRepairRequired { .. }) {
            saw_plan_repair_event = true;
        }
    }
    assert!(
        saw_plan_repair_event,
        "Tester path must emit PlanRepairRequired"
    );
}

#[tokio::test]
async fn coding_plan_repair_runner_routes_only_exact_start_plan_repair_decision() {
    let fixture = plan_repair_fixture();
    let report = ExecutionPlanDefectReport {
        source: PlanDefectSource::Coder,
        findings: vec![canonical_execution_finding("runner_canonical_finding")],
    };

    let paused = start_plan_repair_for_execution_outcome_if_needed(
        &fixture.engine,
        &fixture.attempt,
        Some(CodeReviewFlowDecision::StartPlanRepair),
        Some(&report),
    )
    .await
    .unwrap()
    .expect("runner must start repair for the exact typed decision");

    assert_execution_request(&fixture, &paused, "runner_canonical_finding");

    let safe_stop_fixture = plan_repair_fixture();
    let not_started = start_plan_repair_for_execution_outcome_if_needed(
        &safe_stop_fixture.engine,
        &safe_stop_fixture.attempt,
        Some(CodeReviewFlowDecision::StartStoryAmendment),
        Some(&report),
    )
    .await
    .unwrap();
    assert!(not_started.is_none());
    assert!(
        safe_stop_fixture
            .revision_store
            .list_open_repair_requests(&safe_stop_fixture.plan)
            .unwrap()
            .is_empty()
    );
}

fn canonical_execution_finding(finding_id: &str) -> PlanDefectFinding {
    let finding = plan_defect_finding("execution_evidence");
    PlanDefectFinding {
        finding_id: finding_id.to_string(),
        severity: PlanDefectSeverity::Error,
        defect_class: finding.defect_class,
        reason_code: finding.reason_code.unwrap(),
        message: finding.message,
        evidence: finding.plan_defect_evidence,
        contract_refs: finding.contract_refs,
        capability_refs: finding.capability_refs,
        repair_target: finding.repair_target,
        recommended_route: finding.recommended_route,
        confidence: finding.confidence.unwrap(),
    }
}

fn assert_execution_request(
    fixture: &PlanRepairFixture,
    paused: &CodingExecutionAttempt,
    finding_id: &str,
) {
    assert_eq!(paused.status, CodingAttemptStatus::AwaitingPlanAmendment);
    let request = fixture
        .revision_store
        .list_open_repair_requests(&fixture.plan)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(request.trigger_review_id, None);
    assert_eq!(request.trigger_finding_id, finding_id);
}
