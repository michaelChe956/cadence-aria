use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
};
use crate::product::models::{
    HumanGateTurnStatus, ProviderName, SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_policy::{
    HumanGateSnapshot, HumanReason, RunBudgets, RunHistory, RunPolicy, WorkItemPlanFlowKind,
};
use tempfile::TempDir;

pub(super) fn gate_fixture(budget: u32) -> (TempDir, LifecycleStore, WorkspaceEngine) {
    let (root, lifecycle, engine, _event_rx) = gate_fixture_with_event_rx(budget);
    (root, lifecycle, engine)
}

pub(super) fn gate_fixture_with_event_rx(
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

#[tokio::test]
async fn enter_human_confirm_emits_human_gate_opened_after_gate_state_is_current() {
    let (_root, _lifecycle, mut engine, mut event_rx) = gate_fixture_with_event_rx(1);
    engine
        .enter_human_confirm(Some("门已打开".to_string()))
        .await;

    let mut opened_stage = None;
    while let Ok(event) = event_rx.try_recv() {
        if let EngineEvent::HumanGateOpened { stage } = event {
            opened_stage = Some(stage);
        }
    }
    assert_eq!(
        opened_stage.as_deref(),
        Some("human_confirm"),
        "门开启事件必须在状态与时间线节点写入后通知 runtime 构建全量快照"
    );
}

pub(super) fn feedback(command_id: &str) -> HumanGateFeedbackInput {
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
        HumanGateCloseDecision::Approve,
        HumanGateCloseDecision::Abandon,
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
            .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
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

/// L0 typed 重承载（REQ-RET-02/REQ-CG-04）红绿锚：SC 门关门决策脱离 legacy
/// human-confirm 决策枚举——`HumanGateCloseDecision::Abandon` 经关门链路的
/// 行为与旧 Terminate 路径逐项等价（终态/durable/事件单条、in-flight Busy、
/// 预算耗尽仍可 abandon），且类型面即证明全程不经 legacy 枚举
/// （本函数体内不出现该旧枚举的类型名）。
#[tokio::test]
async fn abandon_human_gate_command_closes_gate_without_legacy_enum() {
    // 1) 开门态 typed abandon：终态 Abandoned + durable Terminated + 单条 close 事件。
    let (_root, lifecycle, mut engine, mut event_rx) = gate_fixture_with_event_rx(2);
    let session_id = engine.session().session_id.clone();
    assert_eq!(
        engine
            .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
            .await
            .expect("typed abandon must close the gate"),
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
            assert_eq!(
                decision, "terminate",
                "durable 事件 payload 字面量与旧路径一致"
            );
            assert_eq!(stage, "completed");
        }
    }
    assert_eq!(close_events, 1);

    // 2) in-flight turn：typed abandon 与旧 Terminate 同样 busy。
    let (_root, _lifecycle, mut engine) = gate_fixture(1);
    let opened = engine
        .handle_human_gate_feedback(feedback("cmd_typed_abandon_busy"))
        .await
        .expect("initial reservation");
    let turn_id = match opened {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    assert_eq!(
        engine
            .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
            .await
            .expect("busy result"),
        HumanGateCloseOutcome::Busy { turn_id }
    );

    // 3) 预算耗尽：typed abandon 仍可用（不退还也不扣减预算，直接终态）。
    let (_root, _lifecycle, mut engine) = gate_fixture(0);
    assert_eq!(
        engine
            .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
            .await
            .expect("typed abandon remains admissible after budget exhaustion"),
        HumanGateCloseOutcome::Abandoned
    );
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
async fn conversational_gate_budget_exhausted_rejects_feedback_but_accepts_termination() {
    let (_root, lifecycle, mut engine) = gate_fixture(0);
    let session_id = engine.session().session_id.clone();
    let before = lifecycle
        .get_workspace_session(&session_id)
        .expect("durable session before rejection");

    assert_eq!(
        engine
            .handle_human_gate_feedback(feedback("cmd_exhausted_accept_close"))
            .await
            .expect("budget rejection"),
        HumanGateCommandOutcome::Rejected {
            code: "HUMAN_GATE_BUDGET_EXHAUSTED".to_string(),
            reason: "manual repair budget is exhausted".to_string(),
        }
    );
    assert_eq!(
        lifecycle
            .get_workspace_session(&session_id)
            .expect("durable session after rejection"),
        before,
        "budget rejection must preserve the open gate"
    );
    assert!(
        lifecycle
            .list_human_gate_turns(&session_id)
            .expect("list turns")
            .is_empty()
    );

    assert_eq!(
        engine
            .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
            .await
            .expect("termination remains admissible"),
        HumanGateCloseOutcome::Abandoned
    );
}

#[tokio::test]
async fn conversational_gate_budget_exhaustion_emits_no_advance_event() {
    let (_root, _lifecycle, mut engine, mut event_rx) = gate_fixture_with_event_rx(0);

    assert!(matches!(
        engine
            .handle_human_gate_feedback(feedback("cmd_exhausted_no_advance"))
            .await
            .expect("budget rejection"),
        HumanGateCommandOutcome::Rejected { code, .. }
            if code == "HUMAN_GATE_BUDGET_EXHAUSTED"
    ));
    assert!(
        event_rx.try_recv().is_err(),
        "budget rejection must not emit an advance or provider event"
    );
}

#[tokio::test]
async fn conversational_gate_closed_event_never_starts_advance() {
    let (_root, _lifecycle, mut engine, mut event_rx) = gate_fixture_with_event_rx(0);

    assert_eq!(
        engine
            .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
            .await
            .expect("termination remains admissible after budget exhaustion"),
        HumanGateCloseOutcome::Abandoned
    );

    let events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, EngineEvent::HumanGateClosed { .. }))
            .count(),
        1,
        "closing emits exactly one close event"
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, EngineEvent::ProviderRunRequested { .. })),
        "HumanGateClosed must not start an advance/provider run"
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

/// R3（oracle 裁决）夹具：SC 会话停在 Final Compile 失败落门前的形态——
/// phase=Approval、WaitingForHuman、**无** human_gate_snapshot（AutoIfValid 链
/// ContinueToCompleted 路由只落 (Running, None)，见 policy_route_record_values）、
/// run_history 已有 repairs_used=2 / manual_repairs_used=1 计数，用于断言补建
/// 预算「接续而非重置」（默认 max_manual_repairs=3 − 1 = 2）。
fn compile_failure_gate_fixture(
    flow_kind: WorkItemPlanFlowKind,
    run_policy: RunPolicy,
) -> (TempDir, LifecycleStore, WorkspaceEngine) {
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
                flow_kind,
                run_policy,
                rollout_snapshot: true,
            }),
        })
        .expect("create session");
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    if flow_kind == WorkItemPlanFlowKind::SingleCandidate {
        record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    }
    record.human_gate_snapshot = None;
    record.run_history = RunHistory {
        repairs_used: 2,
        manual_repairs_used: 1,
        ..RunHistory::default()
    };
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist compile failure fixture");
    let (event_tx, _event_rx) = mpsc::channel(32);
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
    (root, lifecycle, engine)
}

/// R3（oracle 裁决）：SC plan 因 Final Compile 失败落 HumanConfirm 时必须补建
/// human gate snapshot——否则 `handle_human_gate_feedback` 的硬前置
/// （conversational_gate「human gate snapshot is missing」拒收）令该门上的
/// 反馈入口形同虚设。预算口径沿用既有来源（routing_scope.rs
/// `single_candidate_approval_gate` 等价构造）：
/// `RunBudgets::default().max_manual_repairs − run_history.manual_repairs_used`，
/// 从既有 durable 计数接续扣减，不臆造新预算常量。
#[tokio::test]
async fn compile_failure_human_confirm_rebuilds_gate_snapshot_and_accepts_feedback() {
    let (_root, lifecycle, mut engine) = compile_failure_gate_fixture(
        WorkItemPlanFlowKind::SingleCandidate,
        RunPolicy::AutoIfValid,
    );
    // 无 source revision ref → SC compile 确定性失败，走 Final Compile 失败
    // 第三分支（非 recovery、非 batch）落 HumanConfirm。
    engine.enter_policy_valid_work_item_plan_compile().await;

    assert_eq!(
        engine.session().stage,
        WorkspaceStage::HumanConfirm,
        "Final Compile 失败必须落 HumanConfirm"
    );
    let durable = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session");
    assert_eq!(durable.status, WorkspaceSessionStatus::WaitingForHuman);
    let snapshot = durable
        .human_gate_snapshot
        .as_ref()
        .expect("R3: compile 失败落门必须补建 human gate snapshot");
    assert_eq!(
        snapshot.manual_repairs_remaining,
        RunBudgets::default().max_manual_repairs - 1,
        "预算接续而非重置：默认 max_manual_repairs(3) − manual_repairs_used(1)"
    );
    assert_eq!(
        snapshot.attempts_used, 3,
        "attempts_used = repairs_used(2) + manual_repairs_used(1)，与 policy_routing 口径一致"
    );
    assert_eq!(snapshot.trigger, HumanReason::NativeHumanRequired);
    assert_eq!(
        engine.session().human_gate_snapshot, durable.human_gate_snapshot,
        "内存会话必须与 durable 快照同步（handle_human_gate_feedback 读内存快照）"
    );

    // 反馈入口真可用：不再被 "human gate snapshot is missing" 拒收。
    let opened = engine
        .handle_human_gate_feedback(feedback("cmd_r3_compile_failure_feedback"))
        .await
        .expect("compile 失败门上的 HumanGateFeedback 必须被接受");
    match opened {
        HumanGateCommandOutcome::TurnOpened {
            remaining_budget, ..
        } => assert_eq!(remaining_budget, 1, "开门 remaining=2，turn 预留恰扣 1"),
        other => panic!("expected TurnOpened, got {other:?}"),
    }
    let after = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session after turn");
    assert_eq!(
        after
            .human_gate_snapshot
            .as_ref()
            .expect("gate snapshot")
            .manual_repairs_remaining,
        1,
        "turn 预留后 durable 预算恰扣 1"
    );
}

/// 对照（R3）：非 SC（Legacy）plan compile 失败同样落 HumanConfirm，但不补建
/// 快照，反馈仍按既有语义拒收——补建不放宽 SingleCandidate 前置。
#[tokio::test]
async fn compile_failure_human_confirm_keeps_legacy_rejection_without_snapshot() {
    let (_root, lifecycle, mut engine) = compile_failure_gate_fixture(
        WorkItemPlanFlowKind::Legacy,
        RunPolicy::Interactive,
    );
    engine.enter_policy_valid_work_item_plan_compile().await;
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
    let durable = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session");
    assert_eq!(
        durable.human_gate_snapshot, None,
        "非 SC 不补建快照（反馈入口本就限定 SingleCandidate）"
    );
    assert_eq!(
        engine
            .handle_human_gate_feedback(feedback("cmd_r3_legacy_feedback"))
            .await
            .expect("structured rejection"),
        HumanGateCommandOutcome::Rejected {
            code: "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID".to_string(),
            reason: "human gate feedback is only available for a single-candidate work-item plan in human_confirm"
                .to_string(),
        }
    );
}

/// 对照（R3）：非法 stage（非 HumanConfirm）的反馈仍拒收——补建快照不放宽
/// stage 门。
#[tokio::test]
async fn compile_failure_rebuilt_gate_keeps_stage_rejection() {
    let (_root, _lifecycle, mut engine) = compile_failure_gate_fixture(
        WorkItemPlanFlowKind::SingleCandidate,
        RunPolicy::AutoIfValid,
    );
    engine.enter_policy_valid_work_item_plan_compile().await;
    assert!(engine.session().human_gate_snapshot.is_some());
    engine.session.stage = WorkspaceStage::Running; // 模拟门已推进后的迟到反馈
    assert_eq!(
        engine
            .handle_human_gate_feedback(feedback("cmd_r3_stage_invalid"))
            .await
            .expect("structured rejection"),
        HumanGateCommandOutcome::Rejected {
            code: "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID".to_string(),
            reason: "human gate feedback is only available for a single-candidate work-item plan in human_confirm"
                .to_string(),
        }
    );
}
