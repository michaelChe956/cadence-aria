//! F-21 fix round（controller 裁定=引擎侧补建关门效力）：plan 会话（SC 流）
//! 停在 human_confirm 的 context blocker 门（0449 现场形态）与 author 连续
//! validate 失败门——两入口只置 stage+WaitingForHuman，不写 human_gate_snapshot
//! 不提相位，typed `abandon_human_gate` 此前被 close CAS 前置拒收（Conflict），
//! 终止零通路。本组用例钉 terminate 在这些形态的引擎关门语义：durable
//! WaitingForHuman → Terminated + Completed 阶段 + 恰一条 terminate close 事件；
//! confirm 前置不动（store 层 ⑨ 钉 Conflict）。
use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
use crate::product::models::{
    ProviderName, SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use tempfile::TempDir;

/// plan 会话（SC 流）非终审门夹具：durable 记录按传入相位铺好（context
/// blocker=Prepare、author 失败=Generate、缺相位=None），引擎经真实入口
/// `enter_work_item_plan_context_blocker` 停到 human_confirm（入口自身把
/// durable 置 WaitingForHuman，不写快照——与生产 0449 形态同构）。
async fn plan_non_approval_gate_fixture(
    phase: Option<SingleCandidatePhase>,
) -> (
    TempDir,
    LifecycleStore,
    WorkspaceEngine,
    mpsc::Receiver<EngineEvent>,
) {
    let root = TempDir::new().expect("tempdir");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    // session id 进程内唯一：drift-hook 注册表以 session id 为全局键（同
    // story_author_gate 纪律）。
    static F21_SESSION_SEQUENCE: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(1);
    let session_id = format!(
        "workspace_session_f21_{}",
        F21_SESSION_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let mut record = lifecycle
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "work_item_plan_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: ProviderName::Fake,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(
                    crate::product::lifecycle_store::WorkItemPlanSessionOptions {
                        flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                        run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
                        rollout_snapshot: false,
                    },
                ),
            },
            session_id,
        )
        .expect("create plan session");
    record.single_candidate_phase = phase;
    record.human_gate_snapshot = None;
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist plan gate session");
    let (event_tx, event_rx) = mpsc::channel(16);
    let session = WorkspaceSession::from_record(record.clone());
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    );
    engine
        .enter_work_item_plan_context_blocker(Some("请补充 WorkItemPlan Outline 所需上下文".to_string()))
        .await;
    (root, lifecycle, engine, event_rx)
}

fn collect_close_events(event_rx: &mut mpsc::Receiver<EngineEvent>) -> Vec<(String, String)> {
    let mut events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        if let EngineEvent::HumanGateClosed { decision, stage } = event {
            events.push((decision, stage));
        }
    }
    events
}

/// 0449 现场形态：context blocker 门（prepare 相位、无快照）终止 → CAS 通过 →
/// durable Terminated + 恰一条 terminate close 事件。
#[tokio::test]
async fn plan_context_blocker_gate_abandon_terminates_waiting_for_human_session() {
    let (_root, lifecycle, mut engine, mut event_rx) =
        plan_non_approval_gate_fixture(Some(SingleCandidatePhase::Prepare)).await;
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::HumanConfirm,
        "夹具必须停在 human_confirm 门"
    );

    let outcome = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
        .await
        .expect("context blocker gate abandon must close the gate");
    assert_eq!(outcome, HumanGateCloseOutcome::Abandoned);

    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable plan session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare),
        "terminate 不改相位"
    );
    assert_eq!(durable.human_gate_snapshot, None);
    assert_eq!(engine.session().stage, WorkspaceStage::Completed);
    assert_eq!(
        engine.session().session_status,
        WorkspaceSessionStatus::Terminated
    );

    let close_events = collect_close_events(&mut event_rx);
    assert_eq!(
        close_events,
        vec![("terminate".to_string(), "completed".to_string())],
        "恰好一条 terminate close 事件（WS 权威关门通知）"
    );
}

/// author 连续 validate 失败门（generate 相位）与旧会话缺相位门（None）同款。
#[tokio::test]
async fn plan_author_failure_and_missing_phase_gate_abandon_terminate() {
    for phase in [Some(SingleCandidatePhase::Generate), None] {
        let phase_label = format!("{phase:?}");
        let expected_phase = phase.clone();
        let (_root, lifecycle, mut engine, mut event_rx) =
            plan_non_approval_gate_fixture(phase).await;
        let outcome = engine
            .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
            .await
            .unwrap_or_else(|error| panic!("phase {phase_label} abandon must close: {error}"));
        assert_eq!(outcome, HumanGateCloseOutcome::Abandoned);

        let durable = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("durable plan session");
        assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
        assert_eq!(
            durable.single_candidate_phase, expected_phase,
            "terminate 不改相位（{phase_label}）"
        );
        assert_eq!(collect_close_events(&mut event_rx).len(), 1);
    }
}
