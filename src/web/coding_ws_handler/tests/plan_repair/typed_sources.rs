use super::*;
use crate::cross_cutting::streaming_provider::{ProviderCompletion, ProviderEvent};
use crate::product::coding_models::{TestingOverallStatus, TestingReport};
use crate::product::coding_workspace_engine::{
    CodeReviewFlowDecision, CodingExecutionContext, ExecutionPlanDefectReport, PlanDefectSource,
};
use crate::product::plan_repair::{PlanDefectFinding, PlanDefectSeverity};
use crate::web::coding_ws_handler::start_plan_repair_for_execution_outcome_if_needed;
use std::fs;

struct PlanDefectOutputProvider {
    output: String,
}

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
    let fixture = plan_repair_fixture();
    let testing_report = TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: fixture.attempt.id.clone(),
        role_run_id: None,
        run_no: None,
        commands: Vec::new(),
        overall_status: TestingOverallStatus::Blocked,
        provider_claim: None,
        backend_verified: true,
        started_at: "2026-07-19T00:00:00Z".to_string(),
        completed_at: Some("2026-07-19T00:00:01Z".to_string()),
        plan_id: Some("test_plan_0001".to_string()),
        plan_summary: Some("tester found a plan defect".to_string()),
        steps: Vec::new(),
        unplanned_commands: Vec::new(),
        unplanned_evidence: Vec::new(),
        missing_required_steps: Vec::new(),
        skipped_required_steps: Vec::new(),
        context_warnings: Vec::new(),
        raw_provider_output_ref: None,
        plan_defect_findings: vec![canonical_execution_finding("tester_canonical_finding")],
    };
    let report = ExecutionPlanDefectReport {
        source: PlanDefectSource::Tester,
        findings: testing_report.plan_defect_findings,
    };

    let paused = fixture
        .engine
        .start_plan_repair_from_execution_report(&fixture.attempt, &report)
        .await
        .unwrap();

    assert_execution_request(&fixture, &paused, "tester_canonical_finding");
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
