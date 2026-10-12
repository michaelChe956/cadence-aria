// 蒸发链修法（诊断 local://diag-manual-revision-evaporate.md）：
// 修法②——command_id 命中**终态** turn 必须以 typed Rejected 回显（稳定码
// HUMAN_GATE_COMMAND_REPLAYED_AFTER_TERMINAL_TURN），重连幂等只覆盖
// Reserved/Running。现场（workspace_session_0003 / turn 71b47478）：第二次
// 提交复用死 turn 的 command_id 被无状态过滤的查重判为 Replayed——零 durable
// 写 + 客户端对同 turn_id 幂等吞帧 = 无声蒸发。
// 顺手项——turn 失败收口修订 author 载体节点（否则永停 Active 时间线幻影）。

/// 修法②红绿锚：Failed 终态 turn 的 command_id 重放 → typed Rejected，
/// 且拒收零副作用（会话与 turn 集合原样）。
#[tokio::test]
async fn conversational_gate_feedback_replay_after_failed_turn_is_typed_rejected() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    let turn_id = match engine
        .handle_human_gate_feedback(super::conversational_gate::feedback(
            "cmd_terminal_replay_failed",
        ))
        .await
        .expect("open turn")
    {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    engine
        .fail_human_gate_turn(
            &turn_id,
            crate::product::models::HumanGateTurnFailureClass::ValidationReject,
        )
        .await
        .expect("fail turn");
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session before terminal replay");

    let outcome = engine
        .handle_human_gate_feedback(super::conversational_gate::feedback(
            "cmd_terminal_replay_failed",
        ))
        .await
        .expect("terminal replay outcome");

    match outcome {
        HumanGateCommandOutcome::Rejected { code, reason } => {
            assert_eq!(
                code, "HUMAN_GATE_COMMAND_REPLAYED_AFTER_TERMINAL_TURN",
                "terminal replay must carry the stable typed code"
            );
            assert!(
                reason.contains(&turn_id),
                "reason must identify the terminal turn: {reason}"
            );
        }
        other => panic!("terminal replay must be typed rejected, got {other:?}"),
    }
    // 拒收是可见回显而非吞帧：不得有任何 durable 写。
    assert_eq!(
        lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("durable session after terminal replay"),
        before,
        "terminal replay rejection must not mutate durable session"
    );
    assert_eq!(
        lifecycle
            .list_human_gate_turns(engine.session().session_id.as_str())
            .expect("list turns")
            .len(),
        1
    );
}

/// 对照锚：Running（非终态）turn 的 command_id 重放保持 Replayed 幂等——
/// 修法②不得放宽重连重放语义。
#[tokio::test]
async fn conversational_gate_feedback_replay_stays_idempotent_while_turn_running() {
    let (_root, _lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    let turn_id = match engine
        .handle_human_gate_feedback(super::conversational_gate::feedback("cmd_running_replay"))
        .await
        .expect("open turn")
    {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    engine
        .mark_human_gate_turn_running(&turn_id)
        .expect("mark running");

    let replayed = engine
        .handle_human_gate_feedback(super::conversational_gate::feedback("cmd_running_replay"))
        .await
        .expect("running replay outcome");

    match replayed {
        HumanGateCommandOutcome::Replayed { turn } => assert_eq!(turn.turn_id, turn_id),
        other => panic!("running replay must stay Replayed, got {other:?}"),
    }
}

/// 修法②红绿锚：Completed 终态 turn（awaiting_confirm 后的重复点击形态）
/// 的 command_id 重放同样 typed Rejected。
#[tokio::test]
async fn conversational_gate_feedback_replay_after_completed_turn_is_typed_rejected() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    let turn_id = match engine
        .handle_human_gate_feedback(super::conversational_gate::feedback(
            "cmd_terminal_replay_completed",
        ))
        .await
        .expect("open turn")
    {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    // 直接经 durable store 推进 Reserved→Completed（progression 合法路径），
    // 模拟修订成功后 awaiting_confirm 期间的重复提交。
    let expected = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session");
    let mut turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("durable turn");
    turn.status = crate::product::models::HumanGateTurnStatus::Completed;
    turn.result_artifact_ref = Some("artifact_terminal_replay".to_string());
    turn.updated_at = chrono::Utc::now().to_rfc3339();
    lifecycle
        .update_human_gate_turn(&expected, turn)
        .expect("complete turn");

    let outcome = engine
        .handle_human_gate_feedback(super::conversational_gate::feedback(
            "cmd_terminal_replay_completed",
        ))
        .await
        .expect("completed replay outcome");

    match outcome {
        HumanGateCommandOutcome::Rejected { code, .. } => assert_eq!(
            code, "HUMAN_GATE_COMMAND_REPLAYED_AFTER_TERMINAL_TURN",
            "completed turn replay must carry the stable typed code"
        ),
        other => panic!("completed replay must be typed rejected, got {other:?}"),
    }
}

/// 顺手项红绿锚：turn 失败必须收口修订 author 载体节点——与成功路径
/// （revision_turn.inc.rs 的 complete_active_node）同构；不收口会留下永久
/// Active 的「修订中」author 节点（现场 workspace_session_0003 node_005）。
#[tokio::test]
async fn failing_human_gate_turn_closes_revision_author_node() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    let turn_id = match engine
        .handle_human_gate_feedback(super::conversational_gate::feedback(
            "cmd_fail_closes_author_node",
        ))
        .await
        .expect("open turn")
    {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    engine
        .mark_human_gate_turn_running(&turn_id)
        .expect("mark running");
    let author_node_id = engine.begin_work_item_plan_human_gate_revision_run().await;
    assert_eq!(
        engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == author_node_id)
            .expect("revision author node")
            .status,
        crate::web::workspace_ws_types::TimelineNodeStatus::Active
    );

    engine
        .fail_human_gate_turn(
            &turn_id,
            crate::product::models::HumanGateTurnFailureClass::ProviderErr,
        )
        .await
        .expect("fail turn");

    let failed_node = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_id == author_node_id)
        .expect("revision author node after failure");
    assert_eq!(
        failed_node.status,
        crate::web::workspace_ws_types::TimelineNodeStatus::Failed,
        "failing the turn must close the revision author node (timeline phantom)"
    );
    assert!(
        failed_node.completed_at.is_some(),
        "closed author node must carry completed_at"
    );
    // durable turn 侧同步终态化。
    let durable_turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("durable turn after failure");
    assert_eq!(
        durable_turn.status,
        crate::product::models::HumanGateTurnStatus::Failed
    );
}
