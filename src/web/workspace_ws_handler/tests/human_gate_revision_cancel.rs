//! 蒸发链修法（顺手项·取消臂回显化，诊断 local://diag-manual-revision-evaporate.md
//! §a-①）：provider_run.rs 的 `tokio::select!` 取消臂 drop drive 后，SC 修订
//! turn 此前永停 Reserved/Running——无失败落盘、无回显帧，后续 feedback 全被
//! gate_busy 卡死。`fail_cancelled_human_gate_revision_turn` 是取消臂的收口
//! 助手：durable Failed(cancelled) + HumanGateTurnFailed 回显 + eprintln 打点；
//! 刚完成的 turn 幂等跳过（不回显误导帧）。

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::json_store;
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
};
use crate::product::models::{
    HumanGateTurnFailureClass, HumanGateTurnStatus, ProviderName, SingleCandidatePhase,
    WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_policy::{
    HumanGateSnapshot, HumanReason, RunPolicy, WorkItemPlanFlowKind,
};
use crate::product::workspace_engine::{HumanGateFeedbackInput, WorkspaceEngine, WorkspaceSession};

fn revision_cancel_fixture() -> (tempfile::TempDir, LifecycleStore, WorkspaceEngine) {
    let root = tempfile::TempDir::new().expect("tempdir");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    let mut record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "plan_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Fake),
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
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
    });
    json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist fixture session");
    let (event_tx, _event_rx) = mpsc::channel(8);
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

async fn open_running_revision_turn(engine: &mut WorkspaceEngine, command_id: &str) -> String {
    let outcome = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: command_id.to_string(),
            feedback: "修订当前候选标题".to_string(),
        })
        .await
        .expect("open human gate turn");
    let turn_id = match outcome {
        crate::product::workspace_engine::HumanGateCommandOutcome::TurnOpened { turn, .. } => {
            turn.turn_id
        }
        other => panic!("expected opened turn, got {other:?}"),
    };
    engine
        .mark_human_gate_turn_running(&turn_id)
        .expect("mark turn running");
    turn_id
}

#[tokio::test]
async fn cancelled_revision_turn_fails_durably_and_echoes_human_gate_turn_failed() {
    let (_root, lifecycle, mut engine) = revision_cancel_fixture();
    let session_id = engine.session().session_id.to_string();
    let turn_id = open_running_revision_turn(&mut engine, "cmd_cancel_echo").await;
    let author_node_id = engine.begin_work_item_plan_human_gate_revision_run().await;
    let engine = Arc::new(Mutex::new(engine));
    let (outbound_tx, mut outbound_rx) = mpsc::channel(8);

    let transitioned =
        fail_cancelled_human_gate_revision_turn(&engine, &outbound_tx, &session_id, &turn_id).await;

    assert!(
        transitioned,
        "cancelled running turn must transition to Failed"
    );
    let durable_turn = lifecycle
        .get_human_gate_turn(&session_id, &turn_id)
        .expect("durable turn after cancel");
    assert_eq!(durable_turn.status, HumanGateTurnStatus::Failed);
    assert_eq!(
        durable_turn.failure_class,
        Some(HumanGateTurnFailureClass::Cancelled)
    );
    // 顺手项：取消同样收口修订 author 载体节点。
    {
        let engine = engine.lock().await;
        let node = engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == author_node_id)
            .expect("revision author node after cancel");
        assert_eq!(
            node.status,
            crate::web::workspace_ws_types::TimelineNodeStatus::Failed
        );
    }
    // 回显化：取消必须发出 HumanGateTurnFailed 帧而非静默吞掉。
    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), outbound_rx.recv())
        .await
        .expect("cancel echo frame within timeout")
        .expect("outbound channel alive");
    match frame {
        OutboundControl::Text(json) => {
            let value: serde_json::Value = serde_json::from_str(&json).expect("echo frame json");
            assert_eq!(
                value["type"].as_str(),
                Some("human_gate_turn_failed"),
                "cancel echo frame: {json}"
            );
            assert_eq!(value["turn_id"].as_str(), Some(turn_id.as_str()));
            assert_eq!(value["failure_class"].as_str(), Some("cancelled"));
        }
        other => panic!("expected text echo frame, got {other:?}"),
    }
}

#[tokio::test]
async fn cancelled_revision_turn_is_idempotent_on_terminal_turns() {
    let (_root, lifecycle, mut engine) = revision_cancel_fixture();
    let session_id = engine.session().session_id.to_string();
    let turn_id = open_running_revision_turn(&mut engine, "cmd_cancel_idempotent").await;
    engine
        .fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
        .await
        .expect("pre-fail turn");
    let engine = Arc::new(Mutex::new(engine));
    let (outbound_tx, mut outbound_rx) = mpsc::channel(8);

    let transitioned =
        fail_cancelled_human_gate_revision_turn(&engine, &outbound_tx, &session_id, &turn_id).await;

    assert!(
        !transitioned,
        "terminal turn must be an idempotent no-op, not re-echoed"
    );
    assert!(
        outbound_rx.try_recv().is_err(),
        "no echo frame may be sent for an already-terminal turn"
    );
    let durable_turn = lifecycle
        .get_human_gate_turn(&session_id, &turn_id)
        .expect("durable turn");
    assert_eq!(
        durable_turn.failure_class,
        Some(HumanGateTurnFailureClass::ProviderErr),
        "cancel no-op must not rewrite the original failure class"
    );
}
