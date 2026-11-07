// REQ-CG-10（gate-card-resolved-and-inflight-state）：owner 在人工修订 turn 的
// provider run 进行中点击终止面（abort），该失败 SHALL 记为中止而非 provider
// 故障——turn 终态 Failed(cancelled)、修订 author 载体节点 Aborted，并明说
// 「你的选择未生效」。现场（06:11:00）该失败被记成 failed/provider_err，且节点
// 文案「人工修订失败，本轮反馈未采纳」，对话流零提示。
//
// 与 harness 其它用例的区别：abort 必须作用于**同一个** manager（run 登记在
// 其 active_run），故用 `build_inbound_context` 取 context 并保留其 manager 句柄。

#[tokio::test]
async fn campaign_stage3_revision_abort_records_cancelled_turn_and_aborted_node() {
    let harness = campaign_stage3_fixture(2, vec![RevisionScriptStep::Hang]).await;
    let context = harness.build_inbound_context().await;
    let manager = context.run_context.manager.clone();

    handle_workspace_inbound_message(
        context,
        WsInMessage::HumanGateFeedback {
            command_id: "cmd-revision-abort".to_string(),
            feedback: "请把标题改短一点".to_string(),
            excluded_finding_ids: vec![],
        },
    )
    .await;
    let open = harness.await_gate_event("human_gate_turn_open").await;
    let WsOutMessage::HumanGateTurnOpen { turn_id, .. } = open else {
        panic!("expected turn open, got {open:?}");
    };
    timeout(
        Duration::from_secs(10),
        harness.provider.hang_entered.notified(),
    )
    .await
    .expect("provider hang entered");

    // owner 在终止面点击终止：取消在途 run。
    assert!(
        manager.abort_active_run().await,
        "revision run must be the active run"
    );

    let failed = harness.await_gate_event("human_gate_turn_failed").await;
    let WsOutMessage::HumanGateTurnFailed {
        turn_id: failed_turn,
        failure_class,
        message,
    } = failed
    else {
        panic!("expected turn failed, got {failed:?}");
    };
    assert_eq!(failed_turn, turn_id);
    assert_eq!(
        failure_class, "cancelled",
        "abort must not be reported as provider_err"
    );
    assert!(
        message.contains("你的选择未生效"),
        "failure echo must say the choice did not take effect: {message}"
    );

    let durable_turn = harness
        .durable_turns()
        .into_iter()
        .find(|turn| turn.turn_id == turn_id)
        .expect("durable revision turn");
    assert_eq!(durable_turn.status, HumanGateTurnStatus::Failed);
    assert_eq!(
        durable_turn.failure_class,
        Some(crate::product::models::HumanGateTurnFailureClass::Cancelled)
    );

    let aborted_nodes: Vec<_> = harness
        .durable_timeline_nodes()
        .into_iter()
        .filter(|node| {
            node.status == crate::web::workspace_ws_types::TimelineNodeStatus::Aborted
        })
        .collect();
    assert!(
        aborted_nodes.iter().any(|node| node.summary.as_deref()
            == Some("运行已中止：修订被中断，你的选择未生效")),
        "the revision author node must be Aborted with the explicit sentence: {aborted_nodes:?}"
    );
    assert!(
        harness
            .durable_timeline_nodes()
            .iter()
            .all(|node| node.summary.as_deref() != Some("人工修订失败，本轮反馈未采纳")),
        "an abort must never be recorded as a failed revision"
    );
}
