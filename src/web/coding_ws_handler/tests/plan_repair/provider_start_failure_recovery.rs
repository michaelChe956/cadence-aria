use std::fs;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, StreamChunk, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::product::coding_attempt_store::FailedCodeReviewRecoveryPhase;
use crate::product::coding_models::{
    CodingAgentRole, CodingProviderRole, CodingRoleRunStatus, CodingRoleRunTrigger,
    CodingTimelineNode, CodingTimelineNodeStatus, CodingUnitRunStatus,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::protocol::contracts::AdapterInput;
use crate::web::coding_ws_handler::{
    should_resume_runner_after_gate_response, spawn_coding_runner_reserved,
};
use crate::web::runtime::WebRuntime;
use crate::web::state::{CodingAttemptRunKey, WebAppState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProviderStreamMode {
    Modern,
    Legacy,
}

#[test]
fn permission_or_choice_wait_timeout_does_not_enqueue_automatic_retry() {
    let waiting = CodingAttemptStatus::WaitingForHuman;

    assert!(!should_resume_runner_after_gate_response(
        "permission_timeout",
        &CodingExecutionAttempt {
            status: waiting.clone(),
            version: 0,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            ..plan_repair_fixture_with_dependency(false).attempt
        },
    ));
    assert!(!should_resume_runner_after_gate_response(
        "choice_timeout",
        &CodingExecutionAttempt {
            status: waiting,
            version: 0,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            ..plan_repair_fixture_with_dependency(false).attempt
        },
    ));
}

struct RecoveryReviewerProvider {
    mode: ProviderStreamMode,
    poison: Option<(CodingAttemptStore, CodingExecutionAttempt)>,
}

impl RecoveryReviewerProvider {
    fn failing(
        mode: ProviderStreamMode,
        store: CodingAttemptStore,
        attempt: CodingExecutionAttempt,
    ) -> Self {
        Self {
            mode,
            poison: Some((store, attempt)),
        }
    }

    fn successful(mode: ProviderStreamMode) -> Self {
        Self { mode, poison: None }
    }

    fn poison_provider_start_event(&self) {
        let Some((store, attempt)) = &self.poison else {
            return;
        };
        let retry = store
            .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("role runs before ProviderStart")
            .into_iter()
            .find(|run| {
                run.role == CodingProviderRole::CodeReviewer
                    && run.status == CodingRoleRunStatus::Running
                    && run.trigger == CodingRoleRunTrigger::ManualRetry
                    && run.node_id.is_some()
            })
            .expect("bound running retry reviewer role run");
        let path = store.role_run_event_log_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &retry.id,
        );
        fs::remove_file(&path).expect("remove retry provider prompt event log");
        fs::create_dir(&path).expect("poison retry ProviderStart event path");
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RecoveryReviewerProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        if self.mode == ProviderStreamMode::Legacy {
            return Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "streaming provider start is not implemented",
                0,
            ));
        }
        self.poison_provider_start_event();
        let output = recovered_reviewer_plan_defect_output();
        let (event_tx, event_rx) = mpsc::channel(2);
        event_tx
            .try_send(ProviderEvent::TextDelta {
                content: output.clone(),
            })
            .expect("queue modern provider text");
        event_tx
            .try_send(ProviderEvent::Completed(ProviderCompletion::plain(
                output,
                Some("provider-start-recovery-modern".to_string()),
            )))
            .expect("queue modern provider completion");
        let (command_tx, _command_rx) = mpsc::channel(2);
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.poison_provider_start_event();
        let output = recovered_reviewer_plan_defect_output();
        let (chunk_tx, chunk_rx) = mpsc::channel(2);
        chunk_tx
            .try_send(StreamChunk::Text(output.clone()))
            .expect("queue legacy provider text");
        chunk_tx
            .try_send(StreamChunk::Done {
                full_output: output,
            })
            .expect("queue legacy provider completion");
        Ok(chunk_rx)
    }
}

#[tokio::test]
async fn coding_plan_repair_provider_start_failure_recovery_modern_can_retry_again() {
    assert_provider_start_failure_can_recover_again(ProviderStreamMode::Modern).await;
}

#[tokio::test]
async fn coding_plan_repair_provider_start_failure_recovery_legacy_can_retry_again() {
    assert_provider_start_failure_can_recover_again(ProviderStreamMode::Legacy).await;
}

async fn assert_provider_start_failure_can_recover_again(mode: ProviderStreamMode) {
    let fixture = plan_repair_fixture_with_dependency(false);
    fs::create_dir_all(fixture.attempt.worktree_path.as_ref().expect("worktree"))
        .expect("create worktree");
    let (recovered, initial_gate_id) = seed_completed_recovery_candidate(&fixture).await;

    run_recovered_reviewer(
        &fixture,
        recovered.clone(),
        &initial_gate_id,
        Arc::new(RecoveryReviewerProvider::failing(
            mode,
            fixture.store.clone(),
            recovered.clone(),
        )),
    )
    .await;

    let journal = fixture
        .store
        .get_failed_code_review_recovery_journal(
            &recovered.project_id,
            &recovered.issue_id,
            &recovered.id,
        )
        .expect("completed recovery journal")
        .expect("current recovery journal");
    assert_eq!(journal.phase, FailedCodeReviewRecoveryPhase::Completed);
    let failed_retry_id = journal
        .retry_role_run_id
        .as_deref()
        .expect("failed retry role-run id");
    let failed_retry = fixture
        .store
        .get_role_run(
            &recovered.project_id,
            &recovered.issue_id,
            &recovered.id,
            failed_retry_id,
        )
        .expect("failed retry role run");
    assert_eq!(
        failed_retry.status,
        CodingRoleRunStatus::Failed,
        "{mode:?} ProviderStart failure must compensate the bound retry role run"
    );
    assert_eq!(
        failed_retry.reason_code.as_deref(),
        Some("code_review_provider_interrupted")
    );
    let failed_retry_node_id = failed_retry
        .node_id
        .as_deref()
        .expect("failed retry node id");
    let failed_retry_node = fixture
        .store
        .get_timeline_nodes(&recovered.project_id, &recovered.issue_id, &recovered.id)
        .expect("timeline after ProviderStart failure")
        .into_iter()
        .find(|node| node.id == failed_retry_node_id)
        .expect("failed retry timeline node");
    assert_eq!(failed_retry_node.status, CodingTimelineNodeStatus::Failed);
    let blocked = fixture
        .store
        .get_attempt_by_id(&recovered.id)
        .expect("attempt after ProviderStart failure");
    assert_eq!(blocked.status, CodingAttemptStatus::Blocked);
    let recovery_gate = fixture
        .store
        .list_open_blocked_gates(&blocked.project_id, &blocked.issue_id, &blocked.id)
        .expect("open gates after ProviderStart failure")
        .into_iter()
        .find(|gate| gate.reason_code.as_deref() == Some("code_review_provider_interrupted"))
        .expect("new ProviderStart recovery gate");
    assert_ne!(recovery_gate.gate_id, initial_gate_id);
    fs::remove_dir(fixture.store.role_run_event_log_path(
        &recovered.project_id,
        &recovered.issue_id,
        &recovered.id,
        failed_retry_id,
    ))
    .expect("clear ProviderStart failure injection");

    let recovered_again = fixture
        .engine
        .recover_failed_code_review(&recovery_gate.gate_id)
        .await
        .expect("recover compensated ProviderStart failure");
    assert!(
        fixture
            .store
            .get_archived_failed_code_review_recovery_journal(
                &recovered.project_id,
                &recovered.issue_id,
                &recovered.id,
                &initial_gate_id,
            )
            .expect("archived first recovery journal")
            .is_some()
    );
    let next_journal = fixture
        .store
        .get_failed_code_review_recovery_journal(
            &recovered.project_id,
            &recovered.issue_id,
            &recovered.id,
        )
        .expect("second recovery journal")
        .expect("current second recovery journal");
    assert_eq!(next_journal.expected_gate_id, recovery_gate.gate_id);
    assert_ne!(
        next_journal.retry_role_run_id.as_deref(),
        Some(failed_retry_id)
    );

    run_recovered_reviewer(
        &fixture,
        recovered_again,
        &recovery_gate.gate_id,
        Arc::new(RecoveryReviewerProvider::successful(mode)),
    )
    .await;

    assert_plan_repair_started_after_second_recovery(
        &fixture,
        &recovery_gate.gate_id,
        &initial_gate_id,
    );
}

async fn seed_completed_recovery_candidate(
    fixture: &PlanRepairFixture,
) -> (CodingExecutionAttempt, String) {
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
    (recovered, gate.gate_id)
}

async fn run_recovered_reviewer(
    fixture: &PlanRepairFixture,
    attempt: CodingExecutionAttempt,
    recovery_gate_id: &str,
    provider: Arc<dyn StreamingProviderAdapter>,
) {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let mut state = WebAppState::new(
        fixture._tmp.path().to_path_buf(),
        WebRuntime::new_fake(fixture._tmp.path().to_path_buf()),
    );
    state.provider_registry = Arc::new(registry);
    state.test_provider_enabled = true;
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let reservation = state
        .coding_runs
        .try_reserve_attempt(&attempt_key)
        .expect("reserve recovered runner");
    let (event_tx, mut event_rx) = mpsc::channel(256);
    let command_tx = spawn_coding_runner_reserved(
        state.clone(),
        fixture.store.clone(),
        event_tx,
        attempt,
        recovery_gate_id,
        reservation,
    )
    .expect("spawn recovered runner");
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
    while event_rx.try_recv().is_ok() {}
}

fn assert_plan_repair_started_after_second_recovery(
    fixture: &PlanRepairFixture,
    second_gate_id: &str,
    first_gate_id: &str,
) {
    let paused = fixture
        .store
        .get_attempt_by_id(&fixture.attempt.id)
        .expect("attempt after successful second retry");
    let current_journal = fixture
        .store
        .get_failed_code_review_recovery_journal(&paused.project_id, &paused.issue_id, &paused.id)
        .expect("current recovery journal diagnostics");
    let role_runs = fixture
        .store
        .list_role_runs(&paused.project_id, &paused.issue_id, &paused.id)
        .expect("role run diagnostics");
    let diagnostic_reports = fixture
        .store
        .list_code_review_reports(&paused.project_id, &paused.issue_id, &paused.id)
        .expect("review report diagnostics");
    let gates = fixture
        .store
        .list_open_blocked_gates(&paused.project_id, &paused.issue_id, &paused.id)
        .expect("gate diagnostics");
    assert_eq!(
        paused.status,
        CodingAttemptStatus::AwaitingPlanAmendment,
        "journal={current_journal:?}, role_runs={role_runs:?}, reports={diagnostic_reports:?}, gates={gates:?}"
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
    for gate_id in [first_gate_id, second_gate_id] {
        assert!(
            fixture
                .store
                .get_archived_failed_code_review_recovery_journal(
                    &paused.project_id,
                    &paused.issue_id,
                    &paused.id,
                    gate_id,
                )
                .expect("archived recovery journal")
                .is_some(),
            "missing archived journal for {gate_id}"
        );
    }
    let reports = fixture
        .store
        .list_code_review_reports(&paused.project_id, &paused.issue_id, &paused.id)
        .expect("retry review reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(
        code_review_flow_decision(&reports[0], &fixture.projection),
        CodeReviewFlowDecision::StartPlanRepair
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

fn recovered_reviewer_plan_defect_output() -> String {
    serde_json::json!({
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
    .to_string()
}
