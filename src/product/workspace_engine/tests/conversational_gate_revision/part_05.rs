// —— C2 human-gate-convergence（REQ-HGC-01 / REQ-CG-02 预算重置边界修订）——
//
// 现场根因（F-52 §三，issue_0002/workspace_session_0009）：3 次门内 SC 修订后
// `manual_repairs_used=0`（policy 计数从不记人工轮次），重建快照的重置公式
// `max_manual_repairs − manual_repairs_used` 恒等于默认 3——门卡「剩余修复轮次」
// 纹丝不动，预算永不耗尽。
//
// 新契约：同一 logical gate 内的修订轮次 MUST 从 durable 快照 carry-forward
// 剩余预算（修订成功重建快照时接续，不回填默认）；gate-local
// `accepted_feedback_turns` 与预算预留同一 CAS 原子递增，且不得写入 run_history
// 的 repairs_used/manual_repairs_used（policy 计数仅自动返修）。

#[tokio::test]
async fn hgc_same_gate_revision_rounds_decrement_budget_to_zero() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine) =
        evaluate_gate_revision_fixture("hgc_budget_truth", 3, 0);

    let round_one = handoff_clean_rep4().replace(
        "## Work Item WI-001: Backend levels API",
        "## Work Item WI-001: Backend levels API round-2",
    );
    let round_two = round_one.replace(
        "## Work Item WI-001: Backend levels API round-2",
        "## Work Item WI-001: Backend levels API round-3",
    );
    let round_three = round_two.replace(
        "## Work Item WI-001: Backend levels API round-3",
        "## Work Item WI-001: Backend levels API round-4",
    );

    let rounds: [(&str, &str, u32, u32); 3] = [
        ("hgc_round_1", round_one.as_str(), 2, 1),
        ("hgc_round_2", round_two.as_str(), 1, 2),
        ("hgc_round_3", round_three.as_str(), 0, 3),
    ];
    for (command, markdown, expected_remaining, expected_turns) in rounds {
        let turn_id = open_running_revision_turn(&mut engine, command).await;
        engine
            .run_sc_manual_revision_turn(&turn_id, markdown.to_string())
            .await
            .unwrap_or_else(|error| panic!("round {command} must complete: {error}"));
        let routed = lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("durable session after gate rebuild");
        let snapshot = routed.human_gate_snapshot.as_ref().expect("gate snapshot");
        assert_eq!(
            snapshot.manual_repairs_remaining, expected_remaining,
            "round {command}: 同一 logical gate 重建快照必须接续 durable 剩余预算"
        );
        assert_eq!(
            snapshot.accepted_feedback_turns,
            Some(expected_turns),
            "round {command}: gate-local accepted_feedback_turns 与预留同 CAS 原子递增"
        );
        assert_eq!(
            routed.run_history.manual_repairs_used, 0,
            "人工修订轮次不得污染 policy manual_repairs_used"
        );
        assert_eq!(
            routed.run_history.repairs_used, 0,
            "人工修订轮次不得计入自动返修 repairs_used"
        );
    }

    // 预算耗尽：门保持开启（仅 approve/abandon 可用），反馈按既有语义拒收。
    let rejected = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "hgc_exhausted".to_string(),
            feedback: "预算耗尽后的反馈".to_string(),
        })
        .await
        .expect("budget exhausted outcome");
    assert!(matches!(
        &rejected,
        HumanGateCommandOutcome::Rejected { code, .. }
            if code == "HUMAN_GATE_BUDGET_EXHAUSTED"
    ));
    let exhausted = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session at exhaustion");
    assert_eq!(
        exhausted.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
        "预算耗尽不关门"
    );
    assert_eq!(
        exhausted
            .human_gate_snapshot
            .as_ref()
            .expect("snapshot")
            .manual_repairs_remaining,
        0,
        "耗尽态如实显示 0"
    );
    assert_eq!(
        exhausted.human_gate_snapshot.as_ref().unwrap().accepted_feedback_turns,
        Some(3)
    );
}

#[tokio::test]
async fn hgc_reviewer_pass_rebuild_carries_gate_budget_and_turns() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine) =
        evaluate_gate_revision_fixture("hgc_reviewer_truth", 3, 1);
    let turn_id = open_running_revision_turn(&mut engine, "hgc_reviewer_round").await;
    engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("revision must complete");

    // reviewer pass → ContinueToCompleted → Interactive 重建审批门：普通门臂
    // 必须接续 durable 快照预算，而不是重置公式回填默认 3。
    let verdict = crate::web::workspace_ws_types::ReviewVerdict {
        verdict: crate::web::workspace_ws_types::ReviewVerdictType::Pass,
        comments: "review pass".to_string(),
        summary: "review pass".to_string(),
        findings: Vec::new(),
        review_gate: crate::web::workspace_ws_types::ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "review".to_string(),
                None,
            ),
            verdict,
        )
        .await;
    let routed = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("routed session");
    let snapshot = routed.human_gate_snapshot.as_ref().expect("approval gate snapshot");
    assert_eq!(
        snapshot.manual_repairs_remaining, 2,
        "审批门重建必须 carry-forward（3→2 后仍为 2）"
    );
    assert_eq!(snapshot.accepted_feedback_turns, Some(1));
}

#[tokio::test]
async fn hgc_needs_human_rebuild_carries_gate_budget() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine) =
        evaluate_gate_revision_fixture("hgc_needs_human_truth", 3, 1);
    let turn_id = open_running_revision_turn(&mut engine, "hgc_needs_human_round").await;
    engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("revision must complete");

    // reviewer needs_human（invalid_json 降级同形态）→ EnterHumanGate 重建：
    // HumanRequired 臂的快照同样必须接续 durable 预算。
    let verdict = crate::web::workspace_ws_types::ReviewVerdict {
        verdict: crate::web::workspace_ws_types::ReviewVerdictType::NeedsHuman,
        comments: "needs human decision".to_string(),
        summary: "needs human decision".to_string(),
        findings: Vec::new(),
        review_gate: crate::web::workspace_ws_types::ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "review".to_string(),
                None,
            ),
            verdict,
        )
        .await;
    let routed = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("routed session");
    assert_eq!(
        routed.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman
    );
    let snapshot = routed.human_gate_snapshot.as_ref().expect("gate snapshot");
    assert_eq!(
        snapshot.manual_repairs_remaining, 2,
        "HumanRequired 重建必须 carry-forward durable 剩余预算"
    );
    assert_eq!(snapshot.accepted_feedback_turns, Some(1));
}

#[tokio::test]
async fn hgc_reconnect_rebuild_does_not_refill_budget() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (root, lifecycle, mut engine) =
        evaluate_gate_revision_fixture("hgc_reconnect_truth", 3, 0);
    let turn_id = open_running_revision_turn(&mut engine, "hgc_reconnect_round").await;
    engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("revision must complete");
    let routed = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable after round one");
    assert_eq!(
        routed
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
        2
    );

    // 刷新/重连：仅从 durable 记录重建引擎，快照真值不得回填默认满额。
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
    let mut recovered = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
        std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            root.path().join("hgc-reconnect-checkpoints"),
        )),
        lifecycle.clone(),
        event_tx,
        crate::product::workspace_engine::WorkspaceSession::from_record(routed),
    );
    assert_eq!(
        recovered
            .session()
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
        2,
        "重连重建必须取 durable 快照真值"
    );

    // 重连后的第二轮反馈从真值继续递减（2→1），而非从默认 3 重新起算。
    let outcome = recovered
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "hgc_reconnect_round_two".to_string(),
            feedback: "重连后的第二轮反馈".to_string(),
        })
        .await
        .expect("second round after reconnect");
    match outcome {
        HumanGateCommandOutcome::TurnOpened {
            remaining_budget, ..
        } => assert_eq!(remaining_budget, 1),
        other => panic!("expected opened turn, got {other:?}"),
    }
    let after = lifecycle
        .get_workspace_session(recovered.session().session_id.as_str())
        .expect("durable after reconnect round two");
    assert_eq!(
        after
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
        1
    );
}
