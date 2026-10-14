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
    let (_root, lifecycle, mut engine) = evaluate_gate_revision_fixture("hgc_budget_truth", 3, 0);

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
        exhausted
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .accepted_feedback_turns,
        Some(3)
    );
}

#[tokio::test]
async fn hgc_reviewer_pass_rebuild_carries_gate_budget_and_turns() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine) = evaluate_gate_revision_fixture("hgc_reviewer_truth", 3, 1);
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
    let snapshot = routed
        .human_gate_snapshot
        .as_ref()
        .expect("approval gate snapshot");
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
    let (root, lifecycle, mut engine) = evaluate_gate_revision_fixture("hgc_reconnect_truth", 3, 0);
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

// ============================================================================
// SG6a.2：handler 实际 author 与初轮机会融合——唯一 claim/冻结 prompt 消费
// 链（REQ-CG-03「修订不过校验」attempt2 机器指令驱动 × REQ-SG-05 claim 与
// 真实 start 分账）。测试以 handler 将消费的同款引擎/store 接口驱动并断言
// durable 事实（refs/状态/实际 start/账本），不匹配文案。
// ============================================================================

/// fixture 微调：把 gate-local accepted 计数落为 Some(0) 起点（真实会话
/// 由 Evaluate 重建物化；本 fixture 直接写 durable + 内存同款形状），使
/// 「accepted 增 1 非 2」可观察。
fn sg62_seed_accepted_turns_zero(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    engine: &mut crate::product::workspace_engine::WorkspaceEngine,
) {
    let mut record = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session before accepted-turns seed");
    if let Some(snapshot) = record.human_gate_snapshot.as_mut() {
        snapshot.accepted_feedback_turns = Some(0);
    }
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist accepted-turns seed");
    engine.session.human_gate_snapshot = record.human_gate_snapshot.clone();
}

/// turn 专属 provider-start ledger 条目（human_gate:{turn_id}:attempt:*）。
fn sg62_turn_ledger_entries(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    session_id: &str,
    turn_id: &str,
) -> Vec<crate::product::work_item_plan_policy::ProviderStartLedgerEntry> {
    let prefix = format!("human_gate:{turn_id}:attempt:");
    lifecycle
        .get_workspace_session(session_id)
        .expect("session for ledger")
        .provider_start_ledger
        .into_iter()
        .filter(|entry| entry.provider_start_idempotency_key.starts_with(&prefix))
        .collect()
}

/// SG6a.2 主验收：第414行 typed 结构拒收 → 同 turn claim attempt2（冻结
/// 机器指令 prompt 携原反馈/指令/失败候选全文随案返回，handler 消费唯一
/// claim/冻结 prompt）→ 冻结交付精确 join → 实际 launch observation 落
/// ledger（claim 与 start 分账）→ attempt2 合法输出通过全量结构套件。
/// actual starts 恰 2、turn/command 同、accepted 增 1 非 2、review 正常
/// 不自动 confirm、无 RepairOnce 初轮 0 额外（policy 计数零污染）。
#[tokio::test]
async fn sg_06a_2_heading_attempt2_gateway() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06a2_gateway", 3);
    sg62_seed_accepted_turns_zero(&lifecycle, &mut engine);
    let turn_id = open_running_revision_turn(&mut engine, "sg06a2_gateway_command").await;
    let turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after open");
    // 真实 L1 开窗后的耐久形状：活动 grant 绑定本 turn（attempt 1）。
    sg61_seed_structural_grant(&lifecycle, &engine, &turn, 1);
    let run_history_before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session before")
        .run_history;
    let output = sg61_line414_illegal_heading_candidate();

    // —— attempt1：typed heading 拒收 in-band，claim 携冻结件返回 ——
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, output.clone())
        .await
        .expect("typed structural rejection is in-band");
    let crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
        claim, ..
    } = &result
    else {
        panic!("expected in-band rejection for the line-414 candidate, got {result:?}")
    };
    let carry = claim
        .clone()
        .expect("L1 heading rejection must claim attempt2 and carry the frozen prompt");
    assert_eq!(carry.attempt_no, 2);
    assert_eq!(
        carry.provider_start_key,
        format!("human_gate:{turn_id}:attempt:2"),
        "claim carries the provider-start key the real launch path must register"
    );
    // claim 绑定身份与 durable grant 逐字段一致（同一授权窗口）。
    let grant = sg61_current_grant(&lifecycle, engine.session().session_id.as_str())
        .expect("grant survives the claim");
    assert_eq!(grant.attempt_no, 2);
    assert_eq!(
        grant.phase,
        Some(crate::product::models::workspace::StructuralGrantPhase::Attempt2Claimed)
    );
    assert_eq!(
        grant.prompt_ref.as_deref(),
        Some(carry.frozen.prompt_ref.as_str())
    );
    assert_eq!(
        grant.prompt_digest.as_deref(),
        Some(carry.frozen.prompt_digest.as_str())
    );
    assert_eq!(
        grant.failing_source_hash.as_deref(),
        Some(sg61_source_hash(&output).as_str())
    );

    // —— 冻结 prompt 内容：原反馈恰一次 + 机器指令段 + 失败候选全文 ——
    let frozen_text = carry.frozen.full_prompt.as_str();
    assert_eq!(
        frozen_text.matches(turn.feedback_text.as_str()).count(),
        1,
        "attempt2 prompt must carry the original business feedback verbatim exactly once"
    );
    assert!(
        frozen_text.contains("[structural_repair_instruction]"),
        "attempt2 prompt must carry the machine repair instruction section"
    );
    assert!(
        frozen_text.contains(output.as_str()),
        "attempt2 prompt must embed the full failed candidate"
    );

    // 交付 join 的候选对账口径（SG5.3）：claim 绑定的失败候选=合法中间
    // hash——「当前修订候选」在 attempt2 交付窗口即该中间候选（arm 消费
    // 面同款布置；refs/版本/评审不提升）。
    engine.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: output.clone(),
        diff: None,
    });

    // —— handler 消费链①：SG5.3 冻结交付精确 join（同 author/gateway）——
    let delivered = engine
        .build_sc_revision_delivery_for_resume(&turn.command_id, &turn, Some(&carry.frozen))
        .expect("frozen delivery must join the claimed prompt identity exactly");
    assert_eq!(
        delivered, frozen_text,
        "inline delivery must be byte-identical to the frozen full prompt"
    );

    // —— handler 消费链②：实际 launch observation 落 ledger（分账登记）——
    let author = serde_json::to_value(&engine.session().author_provider)
        .ok()
        .and_then(|value| value.as_str().map(ToString::to_string))
        .expect("author provider serde text");
    let registered = lifecycle
        .claim_provider_start_with_details(
            engine.session().session_id.as_str(),
            &carry.provider_start_key,
            Some(author.as_str()),
            Some("2026-10-14T00:00:00Z".to_string()),
        )
        .expect("actual launch registration succeeds (claim guaranteed the key absent)");
    assert!(registered, "first registration of the claimed key must win");

    // durable 账本：actual starts 恰 2（attempt1 预留占位 + attempt2 实际
    // launch 观察），turn/command 同，L1 不 bump turn attempt key。
    let session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session after launch registration");
    let entries =
        sg62_turn_ledger_entries(&lifecycle, engine.session().session_id.as_str(), &turn_id);
    assert_eq!(entries.len(), 2, "exactly two physical starts for the turn");
    let attempt1 = entries
        .iter()
        .find(|entry| entry.provider_start_idempotency_key.ends_with(":attempt:1"))
        .expect("attempt1 reserve placeholder");
    assert!(
        !attempt1.is_actual_launch_observation(),
        "reserve placeholder is not an actual launch observation"
    );
    let attempt2 = entries
        .iter()
        .find(|entry| entry.provider_start_idempotency_key.ends_with(":attempt:2"))
        .expect("attempt2 launch entry");
    assert!(attempt2.is_actual_launch_observation());
    assert_eq!(attempt2.provider.as_deref(), Some(author.as_str()));
    let turn_after_claim = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after claim");
    assert_eq!(
        turn_after_claim.status,
        crate::product::models::HumanGateTurnStatus::Running
    );
    assert_eq!(turn_after_claim.turn_id, turn.turn_id);
    assert_eq!(turn_after_claim.command_id, turn.command_id);
    assert_eq!(
        turn_after_claim.attempt_no, 1,
        "L1 attempt2 is accounted by grant.attempt_no; the turn attempt key is not bumped"
    );

    // accepted 增 1 非 2：attempt2 是结构 attempt，不是新用户反馈轮。
    let accepted = session
        .human_gate_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.accepted_feedback_turns);
    assert_eq!(accepted, Some(1));

    // —— attempt2：合法修复候选通过全量结构套件 → Accepted ——
    let remaining_before = session
        .human_gate_snapshot
        .as_ref()
        .map(|snapshot| snapshot.manual_repairs_remaining);
    let fixed = handoff_clean_rep4();
    let result2 = engine
        .run_sc_manual_revision_turn(&turn_id, fixed)
        .await
        .expect("attempt2 legal delivery must be assessed, not errored");
    assert!(matches!(
        result2,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));
    let turn_done = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after attempt2 acceptance");
    assert_eq!(
        turn_done.status,
        crate::product::models::HumanGateTurnStatus::Completed,
        "same turn completes on the attempt2 delivery"
    );
    assert_eq!(turn_done.turn_id, turn.turn_id);
    assert_eq!(turn_done.command_id, turn.command_id);

    // review 正常不自动 confirm：回原人工确认门，会话不落 Confirmed。
    assert_eq!(
        engine.session().stage,
        crate::product::workspace_engine::WorkspaceStage::HumanConfirm
    );
    let final_session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("final session");
    assert_eq!(
        final_session.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman
    );
    assert_ne!(
        final_session.status,
        crate::product::models::WorkspaceSessionStatus::Confirmed
    );

    // 无 RepairOnce/初轮 0 额外：policy 计数零污染，attempt2 不再动可用
    // 预算（交付结算归 SG6a.3），物理启动恰 2、无第三次。
    assert_eq!(
        final_session.run_history.repairs_used,
        run_history_before.repairs_used
    );
    assert_eq!(
        final_session.run_history.manual_repairs_used,
        run_history_before.manual_repairs_used
    );
    let remaining_after = final_session
        .human_gate_snapshot
        .as_ref()
        .map(|snapshot| snapshot.manual_repairs_remaining);
    assert_eq!(remaining_after, remaining_before);
    assert_eq!(
        sg62_turn_ledger_entries(&lifecycle, engine.session().session_id.as_str(), &turn_id).len(),
        2,
        "no third physical start for the turn"
    );
}

/// SG6a.2 验收②：input/fingerprint join 拒绝 → 零 provider start（claim
/// 已发生但真实 start 不登记、不启动）；unknown（未 typed/无授权窗口）
/// → 等待原门，claim 不携案。
#[tokio::test]
async fn sg_06a_2_input_fingerprint_reject_zero_start_unknown_waits() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;

    // —— 变体 a：claim 后交付 join 的 input 面（候选漂移）与 fingerprint
    // 面（prompt 身份失配）都在任何真实 start 之前稳定码拒绝 ——
    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06a2_join_reject", 3);
    let turn_id = open_running_revision_turn(&mut engine, "sg06a2_join_reject_command").await;
    let turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after open");
    sg61_seed_structural_grant(&lifecycle, &engine, &turn, 1);
    let output = sg61_line414_illegal_heading_candidate();
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, output)
        .await
        .expect("typed rejection in-band");
    let crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
        claim: Some(carry),
        ..
    } = result
    else {
        panic!("expected claimed attempt2 carry for the join-rejection variant")
    };

    // input 面：当前候选漂移（≠ claim 绑定的失败候选）→ 稳定码拒绝。
    engine.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: "# Work Item Plan\n\n## Work Item WI-999: drifted candidate\n".to_string(),
        diff: None,
    });
    let drift = engine
        .build_sc_revision_delivery_for_resume(&turn.command_id, &turn, Some(&carry.frozen))
        .expect_err("candidate drift must refuse delivery before any start");
    assert!(
        drift.starts_with("STRUCTURAL_REVISION_DELIVERY_CANDIDATE_DRIFTED"),
        "unexpected drift rejection: {drift}"
    );

    // fingerprint 面：伪造 prompt 身份（ref 与 claim 绑定不符、digest 仍
    // 绑定真实内容以先过预算裁决）→ 身份 join 稳定码拒绝，不回落原
    // builder。
    let mut forged = carry.frozen.clone();
    forged.prompt_ref = "structural-prompt://forged-ref".to_string();
    let identity = engine
        .build_sc_revision_delivery_for_resume(&turn.command_id, &turn, Some(&forged))
        .expect_err("prompt identity mismatch must refuse delivery before any start");
    assert!(
        identity.starts_with("STRUCTURAL_REVISION_DELIVERY_PROMPT_IDENTITY_MISMATCH"),
        "unexpected identity rejection: {identity}"
    );

    // 零 start：join 拒绝后不登记实际 launch，turn 仍 Running、无第三次。
    let entries =
        sg62_turn_ledger_entries(&lifecycle, engine.session().session_id.as_str(), &turn_id);
    assert_eq!(
        entries.len(),
        1,
        "join rejections must not register any provider start"
    );
    let turn_after = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after join rejections");
    assert_eq!(
        turn_after.status,
        crate::product::models::HumanGateTurnStatus::Running
    );

    // —— 变体 b：unknown（未 typed 失败）→ 等待原门，claim 不携案、零
    // attempt2 消费 ——
    let (_root2, lifecycle2, mut engine2) = durable_revision_fixture("sg06a2_unknown_wait", 3);
    let turn_id2 = open_running_revision_turn(&mut engine2, "sg06a2_unknown_wait_command").await;
    let turn2 = lifecycle2
        .get_human_gate_turn(engine2.session().session_id.as_str(), &turn_id2)
        .expect("turn2 after open");
    sg61_seed_structural_grant(&lifecycle2, &engine2, &turn2, 1);
    let untyped = sg61_untyped_section_candidate();
    let result2 = engine2
        .run_sc_manual_revision_turn(&turn_id2, untyped)
        .await
        .expect("untyped failure is in-band");
    let crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
        claim, ..
    } = &result2
    else {
        panic!("expected in-band rejection for the untyped candidate")
    };
    assert!(
        claim.is_none(),
        "untyped failures carry no claim: the original human gate decides"
    );
    assert_eq!(
        sg62_turn_ledger_entries(
            &lifecycle2,
            engine2.session().session_id.as_str(),
            &turn_id2
        )
        .len(),
        1,
        "unknown waits with zero additional starts"
    );
    let turn2_after = lifecycle2
        .get_human_gate_turn(engine2.session().session_id.as_str(), &turn_id2)
        .expect("turn2 after untyped rejection");
    assert_eq!(
        turn2_after.status,
        crate::product::models::HumanGateTurnStatus::Failed,
        "SG6a.3（复审①）：untyped 拒收收口 turn——原门重新可接新反馈"
    );
}

/// SG6a.2 P3 修复轮（Main 审查派回）：attempt2 冻结交付 join 的候选
/// 基线窗口在 join 未成行（拒/身份失配）后 MUST 还原内存修订基线——
/// 否则后续 `handle_human_gate_feedback` 以残留的失败候选 hash 落
/// durable 新 turn.source_hash（≠当前修订基线），恢复车道
/// MarkFailed(ValidationReject) 会误杀合法修订轮。链：claim 消费→
/// join 拒→取消终态化→新反馈→新 turn.source_hash==上一接受候选 hash。
#[tokio::test]
async fn sg_06a_2_attempt2_join_failure_restores_candidate_baseline_for_next_turn() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06a2_p3_restore", 3);
    let turn_id = open_running_revision_turn(&mut engine, "sg06a2_p3_command").await;
    let turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after open");
    sg61_seed_structural_grant(&lifecycle, &engine, &turn, 1);
    // 上一接受候选（修订基线）＝fixture artifact（rep4）。
    let baseline_markdown = engine
        .session
        .artifact
        .as_ref()
        .and_then(|artifact| artifact.markdown())
        .expect("baseline candidate markdown")
        .to_string();
    let output = sg61_line414_illegal_heading_candidate();
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, output.clone())
        .await
        .expect("typed structural rejection is in-band");
    let crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
        claim: Some(carry),
        ..
    } = result
    else {
        panic!("expected claimed attempt2 carry for the P3 chain")
    };
    // arm 消费面的 join 拒变体：伪造 prompt 身份（真实稳定码拒）。
    let crate::product::workspace_engine::conversational_gate::ScRevisionClaimCarry {
        frozen,
        attempt_no,
        provider_start_key,
    } = carry.clone();
    let mut forged = frozen.clone();
    forged.prompt_ref = "structural-prompt://forged-ref".to_string();
    let forged_carry =
        crate::product::workspace_engine::conversational_gate::ScRevisionClaimCarry {
            frozen: forged,
            attempt_no,
            provider_start_key,
        };
    let rejected = crate::web::workspace_ws_handler::deliver_attempt2_frozen_prompt(
        &mut engine,
        &turn,
        output.as_str(),
        &forged_carry,
    )
    .expect_err("prompt identity mismatch must refuse the join");
    assert!(
        rejected.starts_with("STRUCTURAL_REVISION_DELIVERY_PROMPT_IDENTITY_MISMATCH"),
        "unexpected identity rejection: {rejected}"
    );
    // P3 合同：join 未成行后内存修订基线还原（红：残留失败候选）。
    assert_eq!(
        engine
            .session
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.markdown()),
        Some(baseline_markdown.as_str()),
        "join failure must restore the in-memory candidate baseline (P3)"
    );
    // 链尾：取消终态化 + 新反馈 → durable 新 turn.source_hash==上一接受
    // 候选 hash（红：失败候选 hash）。
    engine
        .fail_human_gate_turn(
            &turn_id,
            crate::product::models::HumanGateTurnFailureClass::ProviderErr,
        )
        .await
        .expect("terminalize the cancelled turn");
    let reopened = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "sg06a2_p3_new_command".to_string(),
            feedback: "P3 后续新反馈".to_string(),
        })
        .await
        .expect("new feedback opens a fresh turn after the cancelled one");
    let new_turn = match reopened {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn,
        other => panic!("expected a fresh turn, got {other:?}"),
    };
    use sha2::{Digest, Sha256};
    let baseline_hash = hex::encode(Sha256::digest(baseline_markdown.as_bytes()));
    assert_eq!(
        new_turn.source_hash, baseline_hash,
        "next turn must hash the last accepted candidate, not the failed attempt1 output (P3)"
    );
}

// ---------------------------------------------------------------------------
// SG6a.3（续）：合法交付 settlement/Reviewing 与终态分离 + 断点幂等。
// ---------------------------------------------------------------------------

/// SG6a.3：合法交付的结算与终态分离（REQ-CG-03「修订成功回呈」/REQ-CG-05
/// 「同键崩溃重放」）——deliver settle 恰一次（DeliveredCharge）+ grant
/// Reviewing + checkpoint 先于终态写；synthetic Pass 收口后 turn Completed
/// 且 grant Resolved（review 证据绑定）；commit→终态断点崩溃重放（turn
/// 回置 Running 的中间态重演）零二次 settle/checkpoint/start。
#[tokio::test]
async fn sg_06a_3_legal_delivery_settles_reviewing_separately_from_terminal() {
    use crate::product::models::UserBudgetReceiptKind;
    use crate::product::models::workspace::{StructuralCheckpointStatus, StructuralGrantPhase};

    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06a3_deliver_settle", 3);
    let turn_id = open_running_revision_turn(&mut engine, "sg06a3_deliver_command").await;
    let candidate = handoff_clean_rep4_v2();
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, candidate.clone())
        .await
        .expect("legal delivery completes");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));

    let session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session after delivery");
    let state = session
        .structural_governance_state
        .as_ref()
        .expect("state after delivery");
    assert_eq!(
        state
            .user_receipts
            .iter()
            .filter(|receipt| receipt.turn_id == turn_id
                && receipt.kind == UserBudgetReceiptKind::DeliveredCharge)
            .count(),
        1,
        "deliver settlement happens exactly once, separated from the terminal write"
    );
    assert_eq!(state.active_hold_count(), 0);
    let grant = state.current_grant.as_ref().expect("grant after delivery");
    assert_eq!(
        grant.candidate_source_revision_ref, session.work_item_plan_source_revision_ref,
        "grant 绑定交付代次中间候选 refs"
    );
    assert_eq!(
        grant.phase,
        Some(StructuralGrantPhase::Resolved),
        "synthetic Pass 收口后 grant 携 review 证据终态 Resolved"
    );
    assert!(grant.review_identity.is_some());
    assert_eq!(state.structure_checkpoints.len(), 1);
    let checkpoint = &state.structure_checkpoints[0];
    assert_eq!(checkpoint.status, StructuralCheckpointStatus::Active);
    assert!(
        checkpoint.review_scope.is_some(),
        "finalize 回填 review scope"
    );
    assert_eq!(
        checkpoint.receipt_ref.as_deref(),
        Some(format!("user_budget_receipt:{turn_id}").as_str())
    );
    assert_eq!(state.available_user_budget(), 2);
    let turn_after = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after delivery");
    assert_eq!(
        turn_after.status,
        crate::product::models::HumanGateTurnStatus::Completed
    );
    assert!(turn_after.result_artifact_ref.is_some());
    let ledger_after_delivery = session.provider_start_ledger.len();

    // —— 断点幂等（REQ-SG-05「同键崩溃重放」）：commit 已结算、终态写
    // 前崩溃的 durable 中间态（turn 仍 Running）重演同候选——不重复
    // settle/checkpoint/start、refs 不变、grant 终态不回退。中间态以
    // fixture 直写回置（Completed→Running 非 store 合法迁移，SG4b 直写
    // 先例），只模拟崩溃残局，不新增第二状态机。
    let session_record = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("record for poke");
    let mut poke = turn_after.clone();
    poke.status = crate::product::models::HumanGateTurnStatus::Running;
    poke.updated_at = "2026-10-14T00:00:00Z".to_string();
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_lifecycle_root(&session_record.project_id, &session_record.issue_id)
            .join("workspace-sessions")
            .join(&session_record.id)
            .join("human-gate-turns")
            .join(format!("{turn_id}.json")),
        &poke,
    )
    .expect("re-open the turn to the pre-terminal crash window");
    let result2 = engine
        .run_sc_manual_revision_turn(&turn_id, candidate)
        .await
        .expect("crash replay of the same delivery completes idempotently");
    assert!(matches!(
        result2,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));
    let session2 = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session after crash replay");
    let state2 = session2
        .structural_governance_state
        .as_ref()
        .expect("state after crash replay");
    assert_eq!(
        state2
            .user_receipts
            .iter()
            .filter(|receipt| receipt.turn_id == turn_id)
            .count(),
        1,
        "settle 断点重放零二次结算"
    );
    assert_eq!(
        state2.structure_checkpoints.len(),
        1,
        "checkpoint 断点重放零重复"
    );
    assert_eq!(
        state2.current_grant.as_ref().expect("grant").phase,
        Some(StructuralGrantPhase::Resolved),
        "已终态 grant 不因重放降级"
    );
    assert_eq!(
        session2.work_item_plan_source_revision_ref,
        session.work_item_plan_source_revision_ref
    );
    assert_eq!(
        session2.provider_start_ledger.len(),
        ledger_after_delivery,
        "refs/settle 断点重放零 provider 启动"
    );
    let turn2 = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after crash replay");
    assert_eq!(
        turn2.status,
        crate::product::models::HumanGateTurnStatus::Completed
    );
}
