use std::fs;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::product::coding_models::{
    CodingAgentRole, CodingProviderRole, CodingRoleRunTrigger, CodingTimelineNode,
    CodingTimelineNodeStatus, CodingUnitRunStatus,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::web::coding_ws_handler::spawn_coding_runner_reserved;
use crate::web::runtime::WebRuntime;
use crate::web::state::{CodingAttemptRunKey, WebAppState};

struct RecoveredReviewerPlanDefectProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for RecoveredReviewerPlanDefectProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let output = serde_json::json!({
                "verdict": "blocked",
                "summary": "retry reviewer found an upstream plan defect",
                "findings": [{
                    "severity": "error",
                    "message": "upstream contract lacks required capability",
                    "source_stage": "code_review",
                    "evidence": [{
                        "kind": "review",
                        "source_ref": "recovered_retry_review",
                        "message": "missing capability"
                    }],
                    "defect_class": "upstream_contract_invalid",
                    "reason_code": "upstream_contract_capability_missing",
                    "contract_refs": ["contract_upstream"],
                    "capability_refs": ["capability_missing"],
                    "repair_target": {
                        "kind": "upstream_work_item",
                        "logical_work_item_ids": ["wi_upstream"],
                        "work_item_revision_ids": ["work_item_revision_upstream"]
                    },
                    "recommended_route": "plan_repair",
                    "confidence": "high"
                }]
            })
            .to_string();
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output,
                    Some("recovered-reviewer-session".to_string()),
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
async fn coding_plan_repair_recovered_reviewer_retry_can_start_plan_repair() {
    let fixture = plan_repair_fixture_with_dependency(false);
    fs::create_dir_all(fixture.attempt.worktree_path.as_ref().expect("worktree"))
        .expect("create worktree");
    let failed_node_id = "coding_node_0009";
    fixture
        .store
        .save_timeline_node(
            &fixture.attempt,
            CodingTimelineNode {
                id: failed_node_id.to_string(),
                attempt_id: fixture.attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                title: "代码审查".to_string(),
                status: CodingTimelineNodeStatus::Running,
                agent_role: Some(CodingAgentRole::Reviewer),
                summary: None,
                started_at: "2026-07-19T00:00:00Z".to_string(),
                completed_at: None,
                artifact_refs: Vec::new(),
            },
        )
        .expect("seed interrupted review node");
    fixture
        .store
        .create_role_run(
            &fixture.attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some(failed_node_id.to_string()),
        )
        .expect("seed interrupted reviewer run");
    let interrupted: Result<(), _> = fixture
        .engine
        .fail_provider_stream_ended(&fixture.attempt, failed_node_id)
        .await;
    assert!(
        interrupted.is_err(),
        "review interruption must fail the run"
    );
    let blocked = fixture
        .store
        .get_attempt_by_id(&fixture.attempt.id)
        .expect("blocked attempt");
    let gate = fixture
        .store
        .list_open_blocked_gates(&blocked.project_id, &blocked.issue_id, &blocked.id)
        .expect("failed review gates")
        .into_iter()
        .find(|gate| gate.reason_code.as_deref() == Some("code_review_provider_interrupted"))
        .expect("failed review recovery gate");
    let recovered = fixture
        .engine
        .recover_failed_code_review(&gate.gate_id)
        .await
        .expect("recover failed review");

    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(RecoveredReviewerPlanDefectProvider),
    );
    let mut state = WebAppState::new(
        fixture._tmp.path().to_path_buf(),
        WebRuntime::new_fake(fixture._tmp.path().to_path_buf()),
    );
    state.provider_registry = Arc::new(registry);
    state.test_provider_enabled = true;
    let attempt_key = CodingAttemptRunKey::from_attempt(&recovered);
    let reservation = state
        .coding_runs
        .try_reserve_attempt(&attempt_key)
        .expect("reserve recovered runner");
    let (event_tx, mut event_rx) = mpsc::channel(256);
    let command_tx = spawn_coding_runner_reserved(
        state.clone(),
        fixture.store.clone(),
        event_tx,
        recovered.clone(),
        &gate.gate_id,
        reservation,
    )
    .expect("spawn recovered runner");
    let premature = plan_defect_report(plan_defect_finding("premature_plan_repair"));
    let error = fixture
        .engine
        .start_plan_repair_from_review(
            &recovered,
            &premature.id,
            "premature_plan_repair_finding",
            &premature.findings[0],
            &fixture.projection,
        )
        .await
        .expect_err("completed recovery must still protect a provider that has not started");
    assert!(
        error
            .to_string()
            .contains("coding_failed_review_recovery_state_changed"),
        "unexpected premature Plan Repair error: {error}"
    );
    assert!(
        fixture
            .revision_store
            .list_open_repair_requests(&fixture.plan)
            .expect("premature repair requests")
            .is_empty()
    );
    command_tx
        .send(CodingRunnerCommand::StageGateConfirm {
            stage: CodingExecutionStage::CodeReview,
        })
        .await
        .expect("confirm retry review stage");

    tokio::time::timeout(Duration::from_secs(5), async {
        while state.coding_runs.runner_count(&attempt_key) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("recovered runner completion");

    let mut runner_events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        runner_events.push(event);
    }
    let journal_after_runner = fixture
        .store
        .get_failed_code_review_recovery_journal(
            &recovered.project_id,
            &recovered.issue_id,
            &recovered.id,
        )
        .expect("journal after runner");
    let role_runs_after_runner = fixture
        .store
        .list_role_runs(&recovered.project_id, &recovered.issue_id, &recovered.id)
        .expect("role runs after runner");
    let reports = fixture
        .store
        .list_code_review_reports(&recovered.project_id, &recovered.issue_id, &recovered.id)
        .expect("retry review reports");
    assert_eq!(
        reports.len(),
        1,
        "retry reviewer report must persist; journal={journal_after_runner:?}, role_runs={role_runs_after_runner:?}, events={runner_events:?}"
    );
    assert_eq!(
        code_review_flow_decision(&reports[0], &fixture.projection),
        CodeReviewFlowDecision::StartPlanRepair,
        "retry reviewer must return the exact typed Plan Repair decision"
    );
    let current_journal = fixture
        .store
        .get_failed_code_review_recovery_journal(
            &recovered.project_id,
            &recovered.issue_id,
            &recovered.id,
        )
        .expect("current journal diagnostics");
    let retry_role_run_id = current_journal
        .as_ref()
        .and_then(|journal| journal.retry_role_run_id.as_deref())
        .unwrap_or("missing_retry_role_run");
    let retry_events = fixture
        .store
        .list_role_run_events(
            &recovered.project_id,
            &recovered.issue_id,
            &recovered.id,
            retry_role_run_id,
        )
        .unwrap_or_default();
    let requests = fixture
        .revision_store
        .list_open_repair_requests(&fixture.plan)
        .expect("repair request diagnostics");
    let paused = fixture
        .store
        .get_attempt_by_id(&recovered.id)
        .expect("attempt after recovered review");
    assert_eq!(
        paused.status,
        CodingAttemptStatus::AwaitingPlanAmendment,
        "journal={current_journal:?}, retry_events={retry_events:?}, requests={requests:?}"
    );
    assert!(
        fixture
            .store
            .get_failed_code_review_recovery_journal(
                &paused.project_id,
                &paused.issue_id,
                &paused.id,
            )
            .expect("current recovery journal")
            .is_none()
    );
    assert!(
        fixture
            .store
            .get_archived_failed_code_review_recovery_journal(
                &paused.project_id,
                &paused.issue_id,
                &paused.id,
                &gate.gate_id,
            )
            .expect("archived recovery journal")
            .is_some()
    );

    let request = fixture
        .revision_store
        .list_open_repair_requests(&fixture.plan)
        .expect("repair requests")
        .into_iter()
        .next()
        .expect("durable repair request");
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let link = lifecycle
        .list_session_links(&paused.project_id, &paused.issue_id)
        .expect("repair session links")
        .into_iter()
        .find(|link| link.trigger.repair_request_id == request.id)
        .expect("durable repair session link");
    assert!(
        lifecycle
            .load_plan_repair_session_state(
                &paused.project_id,
                &paused.issue_id,
                &link.child_session_id,
            )
            .expect("repair session snapshot")
            .is_some()
    );
    assert!(
        fixture
            .store
            .get_timeline_nodes(&paused.project_id, &paused.issue_id, &paused.id)
            .expect("timeline")
            .iter()
            .any(|node| {
                node.title == "Plan Repair"
                    && node.status == CodingTimelineNodeStatus::Blocked
                    && node.artifact_refs.contains(&request.id)
            })
    );
    let unit_run = fixture
        .store
        .get_active_unit_run(&paused)
        .expect("active unit run");
    assert_eq!(unit_run.status, CodingUnitRunStatus::BlockedByPlanDefect);
    assert_eq!(unit_run.plan_repair_count, 1);
}
