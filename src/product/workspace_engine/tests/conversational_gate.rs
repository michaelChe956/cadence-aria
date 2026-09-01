use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
};
use crate::product::models::{
    HumanGateTurnStatus, ProviderName, SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_policy::{
    HumanGateSnapshot, HumanReason, RunPolicy, WorkItemPlanFlowKind,
};
use tempfile::TempDir;

pub(super) fn gate_fixture(budget: u32) -> (TempDir, LifecycleStore, WorkspaceEngine) {
    let (root, lifecycle, engine, _event_rx) = gate_fixture_with_event_rx(budget);
    (root, lifecycle, engine)
}

fn gate_fixture_with_event_rx(
    budget: u32,
) -> (
    TempDir,
    LifecycleStore,
    WorkspaceEngine,
    mpsc::Receiver<EngineEvent>,
) {
    let root = TempDir::new().expect("tempdir");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    let mut record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "plan_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy: RunPolicy::Interactive,
                rollout_snapshot: true,
            }),
        })
        .expect("create session");
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: budget,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
    });
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist human gate fixture");
    let (event_tx, event_rx) = mpsc::channel(8);
    let mut session = WorkspaceSession::from_record(record);
    session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: "# Work Item Plan\n".to_string(),
        diff: None,
    });
    let engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    );
    (root, lifecycle, engine, event_rx)
}

fn feedback(command_id: &str) -> HumanGateFeedbackInput {
    HumanGateFeedbackInput {
        command_id: command_id.to_string(),
        feedback: "只修正这个字段".to_string(),
    }
}

#[tokio::test]
async fn conversational_gate_feedback_replay_returns_same_turn_without_second_start() {
    let (_root, lifecycle, mut engine) = gate_fixture(2);
    let opened = engine
        .handle_human_gate_feedback(feedback("cmd_replay"))
        .await
        .expect("initial reservation");
    let (turn_id, command_id) = match opened {
        HumanGateCommandOutcome::TurnOpened {
            turn,
            remaining_budget,
            ..
        } => {
            assert_eq!(remaining_budget, 1);
            (turn.turn_id, turn.command_id)
        }
        other => panic!("expected opened turn, got {other:?}"),
    };
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session before replay");

    let replayed = engine
        .handle_human_gate_feedback(feedback("cmd_replay"))
        .await
        .expect("replay");
    match replayed {
        HumanGateCommandOutcome::Replayed { turn } => {
            assert_eq!(turn.turn_id, turn_id);
            assert_eq!(turn.command_id, command_id);
        }
        other => panic!("expected replay, got {other:?}"),
    }
    assert_eq!(
        lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("durable session after replay"),
        before,
        "replay cannot alter budget or provider ledger"
    );
    assert_eq!(
        lifecycle
            .list_human_gate_turns(engine.session().session_id.as_str())
            .expect("list turns")
            .len(),
        1
    );
}

#[tokio::test]
async fn conversational_gate_feedback_conflicts_with_inflight_turn() {
    let (_root, lifecycle, mut engine) = gate_fixture(2);
    let opened = engine
        .handle_human_gate_feedback(feedback("cmd_first"))
        .await
        .expect("initial reservation");
    let turn_id = match opened {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session before busy");

    assert_eq!(
        engine
            .handle_human_gate_feedback(feedback("cmd_second"))
            .await
            .expect("busy result"),
        HumanGateCommandOutcome::Busy { turn_id }
    );
    assert_eq!(
        lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("durable session after busy"),
        before,
        "busy path must not write any durable state"
    );
}

#[tokio::test]
async fn conversational_gate_termination_conflicts_with_inflight_turn() {
    let (_root, _lifecycle, mut engine) = gate_fixture(1);
    let opened = engine
        .handle_human_gate_feedback(feedback("cmd_first"))
        .await
        .expect("initial reservation");
    let turn_id = match opened {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };

    for decision in [
        HumanConfirmDecision::Confirm,
        HumanConfirmDecision::Terminate,
    ] {
        assert_eq!(
            engine
                .handle_human_gate_termination(decision)
                .await
                .expect("busy result"),
            HumanGateCloseOutcome::Busy {
                turn_id: turn_id.clone()
            }
        );
    }
}

#[tokio::test]
async fn conversational_gate_terminate_is_durable_and_emits_one_terminal_close_event() {
    let (_root, lifecycle, mut engine, mut event_rx) = gate_fixture_with_event_rx(1);
    let session_id = engine.session().session_id.clone();

    assert_eq!(
        engine
            .handle_human_gate_termination(HumanConfirmDecision::Terminate)
            .await
            .expect("terminate human gate"),
        HumanGateCloseOutcome::Abandoned
    );

    let durable = lifecycle
        .get_workspace_session(&session_id)
        .expect("durable terminated session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
    assert_eq!(durable.human_gate_snapshot, None);
    assert_eq!(durable.human_gate_reservation, None);
    assert_eq!(engine.session().stage, WorkspaceStage::Completed);
    let mut close_events = 0;
    while let Ok(event) = event_rx.try_recv() {
        if let EngineEvent::HumanGateClosed { decision, stage } = event {
            close_events += 1;
            assert_eq!(decision, "terminate");
            assert_eq!(stage, "completed");
        }
    }
    assert_eq!(close_events, 1, "close event must not be duplicated");
}

#[tokio::test]
async fn conversational_gate_budget_exhaustion_rejects_before_reservation() {
    let (_root, lifecycle, mut engine) = gate_fixture(0);
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session before rejection");

    assert_eq!(
        engine
            .handle_human_gate_feedback(feedback("cmd_exhausted"))
            .await
            .expect("budget rejection"),
        HumanGateCommandOutcome::Rejected {
            code: "HUMAN_GATE_BUDGET_EXHAUSTED".to_string(),
            reason: "manual repair budget is exhausted".to_string(),
        }
    );
    assert_eq!(
        lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("durable session after rejection"),
        before
    );
    assert!(
        lifecycle
            .list_human_gate_turns(engine.session().session_id.as_str())
            .expect("list turns")
            .is_empty()
    );
}

#[tokio::test]
async fn conversational_gate_concurrent_feedback_reserves_exactly_one_turn() {
    let (_root, lifecycle, engine) = gate_fixture(2);
    let engine = Arc::new(tokio::sync::Mutex::new(engine));
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut tasks = Vec::new();
    for command_id in ["cmd_parallel_one", "cmd_parallel_two"] {
        let engine = engine.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            engine
                .lock()
                .await
                .handle_human_gate_feedback(feedback(command_id))
                .await
                .expect("gate command")
        }));
    }
    let outcomes = futures_util::future::join_all(tasks)
        .await
        .into_iter()
        .map(|result| result.expect("join"))
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, HumanGateCommandOutcome::TurnOpened { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, HumanGateCommandOutcome::Busy { .. }))
            .count(),
        1
    );
    let session_id = engine.lock().await.session().session_id.clone();
    let session = lifecycle
        .get_workspace_session(&session_id)
        .expect("durable session");
    assert_eq!(
        session
            .human_gate_snapshot
            .as_ref()
            .expect("gate snapshot")
            .manual_repairs_remaining,
        1
    );
    let turns = lifecycle.list_human_gate_turns(&session_id).expect("turns");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].status, HumanGateTurnStatus::Reserved);
    assert_eq!(session.provider_start_ledger.len(), 1);
}
