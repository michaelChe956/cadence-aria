use super::conversational_gate::gate_fixture;
use super::*;
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus};
use crate::product::work_item_plan_policy::RunPolicy;
use crate::product::workspace_engine::compile::SingleCandidateCompileCheckpoint;

fn approval_fixture() -> (tempfile::TempDir, LifecycleStore, WorkspaceEngine) {
    let (tmp, lifecycle, _plan_id, mut engine) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    super::single_candidate_recovery::single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        RunPolicy::Interactive,
    );
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.run_policy = RunPolicy::Interactive;
    record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.human_gate_snapshot = Some(crate::product::work_item_plan_policy::HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: false,
    });
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist approval session");
    let artifact = engine.session.artifact.clone();
    engine.session = WorkspaceSession::from_record(record);
    engine.session.stage = WorkspaceStage::HumanConfirm;
    engine.session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    engine.session.artifact = artifact;
    (tmp, lifecycle, engine)
}

#[tokio::test]
async fn conversational_gate_approve_does_not_confirm_before_compile_success() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(1);
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session before confirm");
    let error = engine
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
        .await
        .expect_err("incomplete fixture must fail closed before confirming");
    assert!(error.contains("compile failed"));
    let after = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session after failed confirm");
    assert_ne!(before.status, WorkspaceSessionStatus::Confirmed);
    assert_ne!(after.status, WorkspaceSessionStatus::Confirmed);
    assert_ne!(
        after.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    assert!(
        after.human_gate_snapshot.is_none() || after.status != WorkspaceSessionStatus::Confirmed
    );
}

#[tokio::test]
async fn conversational_gate_approve_fails_closed_at_compile_finalizer_failpoint() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, mut engine) = approval_fixture();
    let (event_tx, mut event_rx) = mpsc::channel(32);
    engine.event_tx = event_tx;
    let _guard = engine.register_single_candidate_compile_failpoint(
        SingleCandidateCompileCheckpoint::ProvenancePersisted,
    );
    let error = engine
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
        .await
        .expect_err("compile failpoint must fail closed");
    assert!(error.contains("human gate remains open"), "{error}");
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session");
    assert_ne!(durable.status, WorkspaceSessionStatus::Confirmed);
    assert_ne!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
    assert!(
        lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("durable session")
            .provider_start_ledger
            .is_empty()
    );
    assert!(
        !event_rx
            .try_recv()
            .is_ok_and(|event| matches!(event, EngineEvent::HumanGateClosed { .. }))
    );
}

#[tokio::test]
async fn conversational_gate_approve_confirms_only_after_durable_compile() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, mut engine) = approval_fixture();
    let (event_tx, mut event_rx) = mpsc::channel(32);
    engine.event_tx = event_tx;
    let result = engine
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
        .await;
    assert_eq!(result, Ok(HumanGateCloseOutcome::Confirmed), "{result:?}");
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Confirmed);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    let mut close_events = 0;
    while let Ok(event) = event_rx.try_recv() {
        if matches!(event, EngineEvent::HumanGateClosed { .. }) {
            close_events += 1;
        }
    }
    assert_eq!(close_events, 1);
}

#[tokio::test]
async fn conversational_gate_abandon_is_terminal_without_compile() {
    let (_root, lifecycle, mut engine, mut event_rx) =
        super::conversational_gate::gate_fixture_with_event_rx(1);
    let session_id = engine.session().session_id.clone();
    assert_eq!(
        engine
            .handle_human_gate_termination(
                crate::web::workspace_ws_types::HumanConfirmDecision::Terminate
            )
            .await
            .expect("terminate"),
        HumanGateCloseOutcome::Abandoned
    );
    let durable = lifecycle
        .get_workspace_session(&session_id)
        .expect("session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
    let plan_store = WorkItemPlanStore::new(lifecycle.app_paths());
    assert!(
        plan_store
            .list_compile_transactions("project_0001", "issue_0001", "plan_0001")
            .expect("compile transactions")
            .is_empty()
    );
    assert!(
        matches!(event_rx.try_recv(), Ok(EngineEvent::HumanGateClosed { decision, stage }) if decision == "terminate" && stage == "completed")
    );
}

#[tokio::test]
async fn conversational_gate_close_is_busy_during_inflight_turn() {
    let (_root, _lifecycle, mut engine) = gate_fixture(1);
    let opened = engine
        .handle_human_gate_feedback(super::conversational_gate::feedback("busy-close"))
        .await
        .expect("feedback");
    let turn_id = match opened {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    for decision in [
        crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        crate::web::workspace_ws_types::HumanConfirmDecision::Terminate,
    ] {
        assert_eq!(
            engine
                .handle_human_gate_termination(decision)
                .await
                .expect("busy"),
            HumanGateCloseOutcome::Busy {
                turn_id: turn_id.clone()
            }
        );
    }
}
