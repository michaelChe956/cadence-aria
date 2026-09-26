// Step 1 —— 总路径：人为注入 human_required 后，
// `request-change:A;request-change:B;confirm` 两个 typed turn 各经完整
// revision/compiler/validator，approve→compile→durable Confirmed。
#[tokio::test]
async fn campaign_stage3_interactive_multi_turn_revision_then_approve_confirms_plan() {
    let harness = campaign_stage3_fixture(
        2,
        vec![
            RevisionScriptStep::Complete(campaign_candidate_v2()),
            RevisionScriptStep::Complete(campaign_candidate_v3()),
        ],
    )
    .await;

    // —— turn 1：typed request-change:A ——
    let budget_before = harness.budget_remaining();
    assert_eq!(budget_before, 2);
    harness
        .send(WsInMessage::HumanGateFeedback {
            command_id: "cmd-campaign-fb-1".to_string(),
            feedback: "反馈A：补充验收条件".to_string(),
        })
        .await;
    let open1 = harness.await_gate_event("human_gate_turn_open").await;
    let WsOutMessage::HumanGateTurnOpen {
        turn_id: turn_id_1,
        command_id: command_id_1,
        remaining_budget: remaining_1,
    } = open1
    else {
        panic!("expected turn open, got {open1:?}");
    };
    assert_eq!(command_id_1, "cmd-campaign-fb-1");
    assert_eq!(remaining_1, 1, "每 turn 预算减一");
    let completed1 = harness.await_gate_event("human_gate_turn_completed").await;
    let WsOutMessage::HumanGateTurnCompleted {
        turn_id: completed_turn_1,
        artifact_ref: artifact_ref_1,
    } = completed1
    else {
        panic!("expected turn completed, got {completed1:?}");
    };
    assert_eq!(completed_turn_1, turn_id_1);
    assert_eq!(artifact_ref_1, "artifact_version_002");

    let turn1_bytes_prefix = harness.turn_bytes(&turn_id_1);
    let ledger_prefix = harness.provider_start_keys();
    assert_eq!(ledger_prefix.len(), 1);

    // —— turn 2：typed request-change:B ——
    harness
        .send(WsInMessage::HumanGateFeedback {
            command_id: "cmd-campaign-fb-2".to_string(),
            feedback: "反馈B：修正写域范围".to_string(),
        })
        .await;
    let open2 = harness.await_gate_event("human_gate_turn_open").await;
    let WsOutMessage::HumanGateTurnOpen {
        turn_id: turn_id_2,
        command_id: command_id_2,
        remaining_budget: remaining_2,
    } = open2
    else {
        panic!("expected turn 2 open, got {open2:?}");
    };
    assert_eq!(command_id_2, "cmd-campaign-fb-2");
    assert_ne!(turn_id_2, turn_id_1, "turn IDs 唯一");
    // C2（REQ-CG-02 预算重置边界修订，改写登记）：修订完成后 Evaluate route
    // 重建 approval 门快照 MUST carry-forward durable 剩余（2−1=1），本轮
    // reserve 后 remaining=0——同 logical gate 预算真实递减，不回填默认 3。
    assert_eq!(remaining_2, 0, "门预算同 logical gate 内真实递减（改写登记：原重置断言 2）");
    let completed2 = harness.await_gate_event("human_gate_turn_completed").await;
    let WsOutMessage::HumanGateTurnCompleted {
        artifact_ref: artifact_ref_2,
        ..
    } = completed2
    else {
        panic!("expected turn 2 completed");
    };
    assert_eq!(artifact_ref_2, "artifact_version_003");
    assert_ne!(artifact_ref_1, artifact_ref_2, "候选 artifact refs 递进");

    // —— confirm：裸 typed Confirm（非 human_confirm{RequestChange}）——
    harness.send(WsInMessage::Confirm).await;

    // durable Confirmed（compile 在 handler await 内同步完成，仍以落盘为准）。
    let confirmed = loop {
        let record = harness.session_record().await;
        if record.status == WorkspaceSessionStatus::Confirmed
            && record.single_candidate_phase == Some(SingleCandidatePhase::Completed)
        {
            break record;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(confirmed.human_gate_snapshot.is_some(), "批准链保留门快照");

    // —— durable 审计（Step 6 面：不以最后 WS 消息为准）——
    let turns = harness.durable_turns();
    assert_eq!(turns.len(), 2, "两个 turn 各一条 durable 记录");
    assert_ne!(turns[0].turn_id, turns[1].turn_id);
    let mut durable_refs: Vec<Option<String>> = turns
        .iter()
        .map(|turn| turn.result_artifact_ref.clone())
        .collect();
    durable_refs.sort();
    assert_eq!(
        durable_refs,
        vec![
            Some("artifact_version_002".to_string()),
            Some("artifact_version_003".to_string())
        ],
        "候选 refs 递进且只保存 ref（durable 列表新→旧，与顺序无关地核对）",
    );
    // C2（改写登记）：两轮修订重建均 carry-forward（2→1→0），耗尽态如实
    // 显示 0 而非回填默认 3；预算耗尽不关门，confirm 仍可用。
    assert_eq!(harness.budget_remaining(), 0, "门预算经两次路由重建后真实耗尽（改写登记：原重置断言 3）");
    let keys = harness.provider_start_keys();
    assert_eq!(keys.len(), 2, "provider ledger 每真实 start 一项");
    assert!(
        keys.iter().all(
            |key| key.starts_with("human_gate:human_gate_turn_") && key.ends_with(":attempt:1")
        ),
        "两次真实 start 各自幂等键: {keys:?}"
    );
    assert_eq!(
        harness.turn_bytes(&turn_id_1),
        turn1_bytes_prefix,
        "turn-1 durable 记录是稳定前缀，不被 turn 2 改写"
    );
    assert_eq!(
        keys[..ledger_prefix.len()],
        ledger_prefix[..],
        "provider ledger 前缀不变"
    );

    // —— 每步审计行（harness contract）——
    let audit = vec![
        campaign_step_audit(
            "8.2_multi_turn",
            "cmd-campaign-fb-1",
            turns
                .iter()
                .find(|turn| turn.command_id == "cmd-campaign-fb-1"),
            budget_before,
            Some(1),
            vec![keys[0].clone()],
            &turn1_bytes_prefix,
            "human_gate_turn_completed",
        ),
        campaign_step_audit(
            "8.2_multi_turn",
            "cmd-campaign-fb-2",
            turns
                .iter()
                .find(|turn| turn.command_id == "cmd-campaign-fb-2"),
            3,
            Some(2),
            vec![keys[1].clone()],
            &harness.turn_bytes(
                &turns
                    .iter()
                    .find(|turn| turn.command_id == "cmd-campaign-fb-2")
                    .expect("fb-2 durable turn")
                    .turn_id,
            ),
            "human_gate_turn_completed",
        ),
    ];
    assert_eq!(
        audit[0].artifact_ref.as_deref(),
        Some("artifact_version_002")
    );
    assert_eq!(audit[1].attempt_no, Some(1));
    for step in &audit {
        assert_eq!(step.event_prefix_digest.len(), 64);
        assert!(!step.observed_status.is_empty());
    }

    // 无 legacy RequestChange：本用例全程只发 typed 形态。L0 typed 重承载后
    // `handle_human_gate_termination` 的参数（HumanGateCloseDecision）已无法
    // 表达 RequestChange——结构性拒绝；wire 面防回归锁迁移至
    // campaign_stage3_legacy_request_change_is_rejected_at_sc_gate_wire_boundary
    //（stage 白名单直接拒为 STAGE_INVALID）。
}

// Step 3a —— 预算耗尽：feedback 明确 reason 拒绝且零副作用；approve/abandon 仍可用。
#[tokio::test]
async fn campaign_stage3_budget_exhaustion_rejects_feedback_but_allows_approve_or_abandon() {
    // —— fixture A：budget=0 时 feedback 拒绝 ——
    let harness = campaign_stage3_fixture(0, vec![]).await;
    let before = harness.session_bytes();
    harness
        .send(WsInMessage::HumanGateFeedback {
            command_id: "cmd-campaign-exhausted".to_string(),
            feedback: "预算耗尽后的反馈".to_string(),
        })
        .await;
    let rejected = harness.await_gate_event("protocol_error").await;
    let WsOutMessage::ProtocolError { code, message, .. } = rejected else {
        panic!("expected protocol error, got {rejected:?}");
    };
    assert_eq!(code, "HUMAN_GATE_BUDGET_EXHAUSTED");
    assert!(!message.is_empty(), "拒绝必须带明确 reason");
    assert!(harness.durable_turns().is_empty(), "不创建 turn");
    assert!(harness.provider_start_keys().is_empty(), "不写 ledger");
    assert_eq!(harness.session_bytes(), before, "session 零变化");
    drop(harness);

    // —— fixture B：budget=0 时 approve 仍 compile/Confirmed ——
    let harness = campaign_stage3_fixture(0, vec![]).await;
    harness.send(WsInMessage::Confirm).await;
    let record = loop {
        let record = harness.session_record().await;
        if record.status == WorkspaceSessionStatus::Confirmed {
            break record;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(
        record.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    drop(harness);

    // —— fixture C：budget=0 时 abandon 仍终止 ——
    let harness = campaign_stage3_fixture(0, vec![]).await;
    // L2 重钉（T5/REQ-RET-02）：abandon=typed AbandonHumanGate（HumanConfirm 桥接已删）。
    harness
        .send(WsInMessage::AbandonHumanGate {
            command_id: "cmd-campaign-abandon".to_string(),
        })
        .await;
    let record = loop {
        let record = harness.session_record().await;
        if record.status == WorkspaceSessionStatus::Terminated {
            break record;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(record.status, WorkspaceSessionStatus::Terminated);
    assert!(harness.durable_turns().is_empty());
}

// Step 3a-L0 —— typed abandon 命令（REQ-RET-02 L0/REQ-CG-04，双审修订红测）：
// `AbandonHumanGate{command_id}` 必须走真实 ws inbound 分发链（stage 白名单 →
// dispatch → handler → engine close）关门。白名单（protocol.rs
// is_message_valid_for_stage_with_flow SC HumanConfirm 分支）漏加该变体时，
// 本用例以 WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID protocol error 形态复现
// （真实链路红，非仅编译红）。
#[tokio::test]
async fn campaign_stage3_abandon_human_gate_typed_command_closes_gate_through_socket_dispatch() {
    let harness = campaign_stage3_fixture(2, vec![]).await;
    harness
        .send(WsInMessage::AbandonHumanGate {
            command_id: "cmd-campaign-abandon-typed".to_string(),
        })
        .await;
    // 红形态锚：关门成功路径通道静默；被拒时必为 STAGE_INVALID protocol error。
    if let Some(rejected) = harness.probe_outbound(Duration::from_millis(300)).await {
        panic!("typed abandon must reach the gate close chain, got rejected: {rejected:?}");
    }
    let record = loop {
        let record = harness.session_record().await;
        if record.status == WorkspaceSessionStatus::Terminated {
            break record;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(record.status, WorkspaceSessionStatus::Terminated);
    assert!(
        record.human_gate_snapshot.is_none(),
        "关门后快照清理与 legacy Terminate 路径一致"
    );
    assert!(harness.durable_turns().is_empty());
    assert!(
        harness.provider_start_keys().is_empty(),
        "abandon 零 provider start"
    );
}

// Step 3a-L0 —— typed abandon 命令 command_id 边界：空白 command_id 在
// handler 边界拒绝（与 HumanGateFeedback/Advance 同族），零 durable 副作用。
#[tokio::test]
async fn campaign_stage3_abandon_human_gate_rejects_blank_command_id_without_side_effects() {
    let harness = campaign_stage3_fixture(2, vec![]).await;
    let before = harness.session_bytes();
    harness
        .send(WsInMessage::AbandonHumanGate {
            command_id: "   ".to_string(),
        })
        .await;
    let rejected = harness.await_gate_event("protocol_error").await;
    let WsOutMessage::ProtocolError { code, .. } = rejected else {
        panic!("expected protocol error, got {rejected:?}");
    };
    assert_eq!(code, "INVALID_COMMAND_ID");
    assert_eq!(harness.session_bytes(), before, "session 零变化");
    assert!(harness.durable_turns().is_empty(), "不创建 turn");
}

// Step 3a-L0 —— legacy RequestChange 在 SC 门 wire 面直接拒绝（原 multi-turn
// 用例内直呼 engine 的防回归锁迁移至此：L0 typed 重承载后
// `handle_human_gate_termination` 的参数已无法表达 RequestChange——结构性
// 保证；此处钉 wire 面：门开启 stage 白名单只放行 HumanGateFeedback/Confirm/
// HumanConfirm{Terminate} 桥接/AbandonHumanGate）。
// 退役留档（T5/REQ-RET-02）：`campaign_stage3_legacy_request_change_is_rejected_at_sc_gate_wire_boundary` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// Step 3b —— 超长反馈：反馈超长与构造 prompt 超预算各一案，
// turn/budget/ledger/session 全零变化；缩短后新 command 可受理。
#[tokio::test]
async fn campaign_stage3_oversized_feedback_rejects_before_turn_reservation() {
    let harness = campaign_stage3_fixture(2, vec![]).await;

    // 案 1：反馈文本超长。
    let oversized_feedback = "x".repeat(
        crate::product::workspace_engine::SC_MANUAL_REVISION_FEEDBACK_MAX_BYTES + 1,
    );
    let before = harness.session_bytes();
    harness
        .send(WsInMessage::HumanGateFeedback {
            command_id: "cmd-campaign-oversized-feedback".to_string(),
            feedback: oversized_feedback,
        })
        .await;
    let rejected = harness.await_gate_event("protocol_error").await;
    let WsOutMessage::ProtocolError { code, .. } = &rejected else {
        panic!("expected protocol error, got {rejected:?}");
    };
    assert_eq!(code, "HUMAN_GATE_FEEDBACK_TOO_LARGE");
    assert!(harness.durable_turns().is_empty());
    assert!(harness.provider_start_keys().is_empty());
    assert_eq!(harness.session_bytes(), before, "session 零变化");

    // 案 2：command id 超长（构造 prompt 前的 bounded 拒绝面）。
    harness
        .send(WsInMessage::HumanGateFeedback {
            command_id: "c".repeat(257),
            feedback: "短反馈".to_string(),
        })
        .await;
    let rejected = harness.await_gate_event("protocol_error").await;
    let WsOutMessage::ProtocolError { code, .. } = &rejected else {
        panic!("expected protocol error, got {rejected:?}");
    };
    assert_eq!(code, "HUMAN_GATE_COMMAND_ID_TOO_LARGE");
    assert_eq!(harness.session_bytes(), before, "session 仍零变化");

    // 案 3：候选超大导致构造 prompt 超预算（反馈本身合法）。
    {
        let mut engine = harness.engine.lock().await;
        engine.session.artifact = Some(ArtifactPayload::Markdown {
            markdown: format!(
                "# Work Item Plan\n\n## 注释\n{}\n",
                "超长候选。".repeat(20_000)
            ),
            diff: None,
        });
    }
    harness
        .send(WsInMessage::HumanGateFeedback {
            command_id: "cmd-campaign-oversized-prompt".to_string(),
            feedback: "短反馈".to_string(),
        })
        .await;
    let rejected = harness.await_gate_event("protocol_error").await;
    let WsOutMessage::ProtocolError { code, .. } = &rejected else {
        panic!("expected protocol error, got {rejected:?}");
    };
    assert_eq!(code, "HUMAN_GATE_REVISION_PROMPT_TOO_LARGE");
    assert!(
        harness.durable_turns().is_empty(),
        "turn reservation 前拒绝"
    );
    assert_eq!(harness.budget_remaining(), 2, "预算零变化");
    assert!(harness.provider_start_keys().is_empty());

    // 缩短后：新 command 可受理。
    {
        let mut engine = harness.engine.lock().await;
        engine.session.artifact = Some(ArtifactPayload::Markdown {
            markdown: REP4_CANDIDATE.to_string(),
            diff: None,
        });
    }
    harness
        .send(WsInMessage::HumanGateFeedback {
            command_id: "cmd-campaign-shortened".to_string(),
            feedback: "缩短后的反馈".to_string(),
        })
        .await;
    let opened = harness.await_gate_event("human_gate_turn_open").await;
    let WsOutMessage::HumanGateTurnOpen {
        remaining_budget, ..
    } = opened
    else {
        panic!("expected turn open, got {opened:?}");
    };
    assert_eq!(remaining_budget, 1);
    let _ = harness.await_gate_event("human_gate_turn_completed").await;
}

// Step 4a —— 单飞：首 turn 阻塞期间 feedback/approve/abandon 全部 gate_busy，
// 无排队/关门/预算/ledger 增量；释放后才允许下一决定。
#[tokio::test]
async fn campaign_stage3_inflight_rejects_feedback_approve_and_abandon_as_busy() {
    let harness = campaign_stage3_fixture(
        2,
        vec![
            RevisionScriptStep::Hang,
            RevisionScriptStep::Complete(REP4_CANDIDATE.to_string()),
        ],
    )
    .await;

    harness
        .send(WsInMessage::HumanGateFeedback {
            command_id: "cmd-campaign-inflight-1".to_string(),
            feedback: "第一轮反馈".to_string(),
        })
        .await;
    let open = harness.await_gate_event("human_gate_turn_open").await;
    let WsOutMessage::HumanGateTurnOpen { turn_id, .. } = open else {
        panic!("expected turn open, got {open:?}");
    };
    // 等 provider 真正挂起（单飞证据）。
    timeout(
        Duration::from_secs(10),
        harness.provider.hang_entered.notified(),
    )
    .await
    .expect("provider hang entered");

    // 三类并发命令都返回同 turn_id 的 gate_busy（第二个 ws worker：
    // 与首个连接独立，busy 来自 durable 单飞 turn 判定）。
    harness
        .send_isolated_worker(WsInMessage::HumanGateFeedback {
            command_id: "cmd-campaign-inflight-2".to_string(),
            feedback: "并发反馈".to_string(),
        })
        .await;
    let busy_feedback = harness.await_gate_event("human_gate_busy").await;
    harness.send_isolated_worker(WsInMessage::Confirm).await;
    let busy_approve = harness.await_gate_event("human_gate_busy").await;
    harness
        .send_isolated_worker(WsInMessage::AbandonHumanGate {
            command_id: "cmd-campaign-inflight-abandon".to_string(),
        })
        .await;
    let busy_abandon = harness.await_gate_event("human_gate_busy").await;
    for busy in [busy_feedback, busy_approve, busy_abandon] {
        let WsOutMessage::HumanGateBusy { turn_id: busy_turn } = busy else {
            panic!("expected gate busy, got {busy:?}");
        };
        assert_eq!(busy_turn, turn_id, "busy 必须指向同一 in-flight turn");
    }

    // 无排队/关门/预算/ledger 增量。
    assert_eq!(harness.durable_turns().len(), 1, "不创建第二个 turn");
    assert_eq!(harness.budget_remaining(), 1, "预算不重复扣");
    assert_eq!(harness.provider_start_keys().len(), 1, "ledger 不增量");
    assert_eq!(
        harness.session_record_blocking().status,
        WorkspaceSessionStatus::WaitingForHuman,
        "不关门"
    );

    // 释放后才允许下一决定：turn 1 完成后第二个 command 受理。
    harness.provider.hang_release.notify_one();
    let completed = harness.await_gate_event("human_gate_turn_completed").await;
    let WsOutMessage::HumanGateTurnCompleted { turn_id: done, .. } = completed else {
        panic!("expected turn completed, got {completed:?}");
    };
    assert_eq!(done, turn_id);

    harness
        .send(WsInMessage::HumanGateFeedback {
            command_id: "cmd-campaign-inflight-2".to_string(),
            feedback: "释放后的第二轮反馈".to_string(),
        })
        .await;
    let open2 = harness.await_gate_event("human_gate_turn_open").await;
    let WsOutMessage::HumanGateTurnOpen {
        turn_id: turn_id_2, ..
    } = open2
    else {
        panic!("expected second turn open, got {open2:?}");
    };
    assert_ne!(turn_id_2, turn_id);
    let _ = harness.await_gate_event("human_gate_turn_completed").await;
    assert!(
        harness.provider.start_count() >= 2,
        "释放后真实再启动 provider"
    );
}

// Step 4b —— reservation crash 恰好恢复一次：
// CAS durable 后/启动前、启动 ledger 后/完成前两个 fault point 重启并同
// command resend；同 turn_id、预算恰减一次、provider alive 等待 / dead 同
// turn attempt_no++、不超过上限、事件前缀不改。
#[tokio::test]
async fn campaign_stage3_turn_reservation_crash_recovers_exactly_once() {
    // —— fault point A：CAS durable 后 / 启动前 ——
    let harness = campaign_stage3_fixture(2, vec![]).await;
    {
        let mut engine_a = harness.engine.lock().await;
        let opened = engine_a
            .handle_human_gate_feedback(HumanGateFeedbackInput {
                command_id: "cmd-campaign-crash-a".to_string(),
                feedback: "崩溃前的反馈".to_string(),
            })
            .await
            .expect("reserve turn");
        let (turn_a, remaining_a) = match opened {
            HumanGateCommandOutcome::TurnOpened {
                turn,
                remaining_budget,
                ..
            } => (turn, remaining_budget),
            other => panic!("expected opened turn, got {other:?}"),
        };
        assert_eq!(remaining_a, 1);
        assert_eq!(turn_a.status, HumanGateTurnStatus::Reserved, "启动前崩溃");
    }
    let turn_a = harness
        .durable_turns()
        .into_iter()
        .find(|turn| turn.command_id == "cmd-campaign-crash-a")
        .expect("durable reserved turn");
    // “重启”：丢弃内存态，仅从磁盘重建（Step 6 面）。
    let recovered_engine = |root: &TempDir, lifecycle: &LifecycleStore, session_id: &str| {
        let (event_tx, _event_rx) = mpsc::channel(64);
        WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
            lifecycle.clone(),
            event_tx,
            WorkspaceSession::from_record(
                lifecycle.get_workspace_session(session_id).expect("durable session"),
            ),
        )
    };
    let mut recovered =
        recovered_engine(&harness.root, &harness.lifecycle, &harness.session_id);
    let actions = recovered
        .recover_human_gate_turns(false)
        .expect("recover fp-a");
    assert_eq!(
        actions,
        vec![(
            turn_a.turn_id.clone(),
            crate::product::workspace_engine::HumanGateRecoveryAction::ResumeSameTurn {
                next_attempt_no: 1
            }
        )],
        "Reserved 恢复同一 turn 的 attempt 1"
    );
    recovered
        .mark_human_gate_turn_running(&turn_a.turn_id)
        .expect("mark running");
    let accepted = recovered
        .run_sc_manual_revision_turn(&turn_a.turn_id, campaign_candidate_v2())
        .await
        .expect("complete recovered turn");
    assert!(matches!(
        accepted,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));
    let recovered_turn = harness
        .lifecycle
        .get_human_gate_turn(&harness.session_id, &turn_a.turn_id)
        .expect("durable turn");
    assert_eq!(recovered_turn.turn_id, turn_a.turn_id, "同 turn_id");
    assert_eq!(recovered_turn.status, HumanGateTurnStatus::Completed);
    assert_eq!(recovered_turn.attempt_no, 1, "不超过上限");
    // C2（改写登记）：修订完成后 Evaluate route 重建 MUST carry-forward
    // durable 剩余（fixture 预算 2，本轮 reserve 后 1），不回填默认 3。
    assert_eq!(harness.budget_remaining(), 1, "门预算经路由重建接续（改写登记：原重置断言 3）");
    assert_eq!(harness.provider_start_keys().len(), 1, "ledger 恰一项");

    // —— fault point B：启动 ledger 后 / 完成前 ——
    let harness = campaign_stage3_fixture(2, vec![]).await;
    let turn_b = {
        let mut engine_b = harness.engine.lock().await;
        let opened = engine_b
            .handle_human_gate_feedback(HumanGateFeedbackInput {
                command_id: "cmd-campaign-crash-b".to_string(),
                feedback: "启动后崩溃前的反馈".to_string(),
            })
            .await
            .expect("reserve turn");
        match opened {
            HumanGateCommandOutcome::TurnOpened { turn, .. } => turn,
            other => panic!("expected opened turn, got {other:?}"),
        }
    };
    harness
        .engine
        .lock()
        .await
        .mark_human_gate_turn_running(&turn_b.turn_id)
        .expect("provider start 后崩溃（attempt 1 in ledger）");

    // provider alive：恢复分类为等待，turn/ledger 不动。
    {
        let mut recovered_alive =
            recovered_engine(&harness.root, &harness.lifecycle, &harness.session_id);
        let alive_actions = recovered_alive
            .recover_human_gate_turns(true)
            .expect("recover alive");
        assert_eq!(
            alive_actions,
            vec![(
                turn_b.turn_id.clone(),
                crate::product::workspace_engine::HumanGateRecoveryAction::WaitForProvider
            )],
            "provider alive 则等待"
        );
        assert_eq!(
            harness.provider_start_keys().len(),
            1,
            "等待期 ledger 不增量"
        );
        let turn_after_alive = harness
            .lifecycle
            .get_human_gate_turn(&harness.session_id, &turn_b.turn_id)
            .expect("turn after alive wait");
        assert_eq!(turn_after_alive.status, HumanGateTurnStatus::Running);
        assert_eq!(turn_after_alive.attempt_no, 1);
    }

    // provider dead：同 turn attempt_no++（≤ 上限），ledger 增 attempt:2，预算不再扣。
    let mut recovered_dead =
        recovered_engine(&harness.root, &harness.lifecycle, &harness.session_id);
    let dead_actions = recovered_dead
        .recover_human_gate_turns(false)
        .expect("recover dead");
    assert_eq!(
        dead_actions,
        vec![(
            turn_b.turn_id.clone(),
            crate::product::workspace_engine::HumanGateRecoveryAction::ResumeSameTurn {
                next_attempt_no: 2
            }
        )],
        "provider dead 则同 turn attempt_no++"
    );
    let resumed = harness
        .lifecycle
        .get_human_gate_turn(&harness.session_id, &turn_b.turn_id)
        .expect("resumed turn");
    assert_eq!(resumed.attempt_no, 2);
    assert_eq!(
        resumed.attempt_no,
        crate::product::workspace_engine::HUMAN_GATE_PROVIDER_MAX_ATTEMPTS,
        "不超过上限"
    );
    assert_eq!(
        harness.budget_remaining(),
        1,
        "预算恰减一次（恢复不重复扣）"
    );
    let keys = harness.provider_start_keys();
    assert_eq!(keys.len(), 2, "attempt:1 + attempt:2");
    assert!(keys.iter().any(|key| key.ends_with(":attempt:2")));

    // 同 command resend：Replayed 同 turn，无第二次扣预算。
    let replay = recovered_dead
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd-campaign-crash-b".to_string(),
            feedback: "同 command 重发".to_string(),
        })
        .await
        .expect("replay");
    assert!(matches!(
        replay,
        HumanGateCommandOutcome::Replayed { ref turn } if turn.turn_id == turn_b.turn_id
    ));
    assert_eq!(harness.budget_remaining(), 1);
    assert_eq!(harness.provider_start_keys().len(), 2);

    // 事件前缀不改：turn 文件的身份字段在两次恢复后保持。
    assert_eq!(resumed.session_id, turn_b.session_id);
    assert_eq!(resumed.command_id, turn_b.command_id);
    assert_eq!(resumed.feedback_text, turn_b.feedback_text);
    assert_eq!(resumed.created_at, turn_b.created_at);

    // —— provider 故障 flavor：transport death 与 validation reject 后，
    // 同 command 重发均 Replay 同一终态 turn，不二次扣预算/不新建 turn。
    for (scenario, script, expected_class) in [
        (
            "crash_transport_death",
            vec![RevisionScriptStep::TransportDeath],
            "provider_err",
        ),
        (
            "crash_validation_reject",
            vec![RevisionScriptStep::ValidationReject],
            "validation_reject",
        ),
    ] {
        let harness = campaign_stage3_fixture(2, script).await;
        harness
            .send(WsInMessage::HumanGateFeedback {
                command_id: format!("cmd-campaign-{scenario}"),
                feedback: "故障 flavor 反馈".to_string(),
            })
            .await;
        let open = harness.await_gate_event("human_gate_turn_open").await;
        let WsOutMessage::HumanGateTurnOpen { turn_id, .. } = open else {
            panic!("expected turn open, got {open:?}");
        };
        let failed = harness.await_gate_event("human_gate_turn_failed").await;
        let WsOutMessage::HumanGateTurnFailed { failure_class, .. } = failed else {
            panic!("expected turn failed, got {failed:?}");
        };
        assert_eq!(failure_class, expected_class);
        // 终态 turn 上的同 command 重发：Replay 且零增量。
        let replay = {
            let mut engine = harness.engine.lock().await;
            engine
                .handle_human_gate_feedback(HumanGateFeedbackInput {
                    command_id: format!("cmd-campaign-{scenario}"),
                    feedback: "重发".to_string(),
                })
                .await
                .expect("replay")
        };
        assert!(matches!(
            replay,
            HumanGateCommandOutcome::Replayed { ref turn } if turn.turn_id == turn_id
        ));
        assert_eq!(harness.budget_remaining(), 1, "预算恰减一次");
        assert_eq!(harness.durable_turns().len(), 1, "不新建 turn");
        assert_eq!(harness.provider_start_keys().len(), 1);
    }
}

// Step 2 —— 8.2a takeover：stopped_needs_human auto parent 两次 takeover 幂等，
// child interactive + 门可接 typed feedback，继承 snapshot/candidate/diagnostic
// refs/预算，parent bytes/event prefix 完全不变；不满足前提时拒绝且无 child。
#[tokio::test]
async fn campaign_stage3_takeover_auto_stopped_reuses_snapshot_budget_and_candidate() {
    use axum::extract::{Path, State};

    let harness = campaign_stage3_fixture(2, vec![]).await;
    // 组装 stopped_needs_human auto parent（无 fatal/persistence diagnostic）。
    let mut parent = harness.session_record().await;
    parent.status = WorkspaceSessionStatus::StoppedNeedsHuman;
    parent.run_policy = RunPolicy::AutoIfValid;
    parent.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    });
    parent.policy_diagnostics = vec![PolicyDiagnostic {
        code: "transition_budget_low".to_string(),
        message: "非致命诊断".to_string(),
        field: None,
    }];
    let parent_path = harness
        .app_paths
        .issue_root(&parent.project_id, &parent.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", parent.id));
    crate::product::json_store::write_json(&parent_path, &parent).expect("persist parent");
    let parent_bytes_before = std::fs::read(&parent_path).expect("parent bytes");

    let state = WebAppState::new(
        harness.root.path().to_path_buf(),
        crate::web::runtime::WebRuntime::new_fake(harness.root.path().to_path_buf()),
    );

    // 两次 takeover：同一 child / 同一 takeover_event。
    let first = workspace_session_takeover(State(state.clone()), Path(parent.id.clone()))
        .await
        .expect("first takeover")
        .0;
    let second = workspace_session_takeover(State(state.clone()), Path(parent.id.clone()))
        .await
        .expect("second takeover")
        .0;
    assert_eq!(
        first.workspace_session.workspace_session_id, second.workspace_session.workspace_session_id,
        "重复 takeover 幂等返回同一 child"
    );
    assert_eq!(first.takeover_event_id, second.takeover_event_id);
    assert_eq!(first.parent_session_id, parent.id);
    let child_id = first.workspace_session.workspace_session_id.clone();
    assert_ne!(child_id, parent.id);

    // child interactive + 继承 snapshot/candidate refs/预算。
    let child = harness
        .lifecycle
        .get_workspace_session(&child_id)
        .expect("durable child");
    assert_eq!(child.run_policy, RunPolicy::Interactive);
    assert_eq!(child.status, WorkspaceSessionStatus::WaitingForHuman);
    assert_eq!(child.flow_kind, WorkItemPlanFlowKind::SingleCandidate);
    assert_eq!(
        child
            .human_gate_snapshot
            .as_ref()
            .expect("child snapshot")
            .manual_repairs_remaining,
        2,
        "预算继承"
    );
    assert_eq!(
        child.human_gate_snapshot, parent.human_gate_snapshot,
        "snapshot 继承"
    );
    assert_eq!(
        child.work_item_plan_source_revision_ref, parent.work_item_plan_source_revision_ref,
        "candidate source ref 继承"
    );
    assert_eq!(child.plan_candidate_ir_ref, parent.plan_candidate_ir_ref);
    assert_eq!(child.mechanical_report_ref, parent.mechanical_report_ref);
    assert_eq!(child.policy_diagnostics, parent.policy_diagnostics);
    assert!(
        child.provider_start_ledger.is_empty(),
        "不复制 in-flight ledger"
    );

    // child 门可接 typed feedback（真实 ws 分发面）。
    {
        let (event_tx, _event_rx) = mpsc::channel(64);
        let mut child_session = WorkspaceSession::from_record(child.clone());
        child_session.stage = WorkspaceStage::HumanConfirm;
        child_session.session_status = WorkspaceSessionStatus::WaitingForHuman;
        child_session.artifact = Some(ArtifactPayload::Markdown {
            markdown: REP4_CANDIDATE.to_string(),
            diff: None,
        });
        let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(
                harness.root.path().join("child-checkpoints"),
            )),
            harness.lifecycle.clone(),
            event_tx,
            child_session,
        )));
        let workspace_runs = WorkspaceRunRegistry::default();
        let mut registry = ProviderRegistry::new();
        registry.register(
            ProviderName::ClaudeCode,
            harness.provider.clone() as Arc<dyn StreamingProviderAdapter>,
        );
        let (outbound_tx, mut outbound_rx) = mpsc::channel(256);
        let context = WorkspaceInboundContext {
            app_state: WebAppState::new(
                harness.root.path().to_path_buf(),
                crate::web::runtime::WebRuntime::new_fake(harness.root.path().to_path_buf()),
            ),
            engine: engine.clone(),
            run_context: ProviderRunContext::test_fixture(
                Arc::new(registry),
                engine.clone(),
                workspace_runs.clone(),
                child.id.clone(),
                harness.app_paths.clone(),
                child.clone(),
            ),
            outbound_tx,
            session_id: child.id.clone(),
        };
        handle_workspace_inbound_message(
            context,
            WsInMessage::HumanGateFeedback {
                command_id: "cmd-campaign-child-feedback".to_string(),
                feedback: "child 上的反馈".to_string(),
            },
        )
        .await;
        let outbound = timeout(Duration::from_secs(10), outbound_rx.recv())
            .await
            .expect("child turn open outbound")
            .expect("outbound open");
        let OutboundControl::Text(json) = outbound else {
            panic!("expected text outbound");
        };
        let value: serde_json::Value = serde_json::from_str(&json).expect("child outbound json");
        assert_eq!(value["type"], "human_gate_turn_open");
        assert_eq!(value["command_id"], "cmd-campaign-child-feedback");
        assert_eq!(value["remaining_budget"], 1, "child 预算从继承值扣一次");
        // child durable turn 落盘（Step 6 面）。
        let child_turns = harness.lifecycle.list_human_gate_turns(&child_id);
        assert!(
            child_turns.expect("child turns").len() == 1,
            "child turn durable"
        );
    }

    // parent bytes/event prefix 完全不变（含 child feedback 之后）。
    assert_eq!(
        std::fs::read(&parent_path).expect("parent bytes after"),
        parent_bytes_before,
        "parent bytes 完全不变"
    );
    let event = harness
        .lifecycle
        .get_human_gate_takeover_event(&parent.id)
        .expect("takeover event")
        .expect("event exists");
    assert_eq!(event.child_session_id, child_id);
    assert_eq!(event.parent_session_id, parent.id);

    // 不满足前提时 endpoint 拒绝且无 child。
    let mut fatal_parent = parent.clone();
    fatal_parent.id = format!("{child_id}_fatal_sibling");
    fatal_parent.policy_diagnostics = vec![PolicyDiagnostic {
        code: "state_corruption".to_string(),
        message: "fatal".to_string(),
        field: None,
    }];
    let fatal_path = harness
        .app_paths
        .issue_root(&fatal_parent.project_id, &fatal_parent.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", fatal_parent.id));
    crate::product::json_store::write_json(&fatal_path, &fatal_parent)
        .expect("persist fatal parent");
    let rejected = workspace_session_takeover(State(state), Path(fatal_parent.id.clone())).await;
    assert!(
        rejected.is_err(),
        "fatal diagnostic parent 必须被 takeover 拒绝"
    );
    let event_missing = harness
        .lifecycle
        .get_human_gate_takeover_event(&fatal_parent.id)
        .expect("lookup");
    assert!(event_missing.is_none(), "无 takeover event");
}

// Step F-49 —— 门内轮次切换的 durable 事实（A5 双 active 收口 + A6 修订事实落点）。
//
// 现场同构（issue_0002/workspace_session_0009）：门节点真实在场（node_006），用户在门内
// 提交反馈 → 修订轮 → 复评 → 新门（node_010）。缺陷形态：
//   A5 旧门节点滞留 Active（node_006 与 node_010 双 active）；
//   A6 修订事实（prompt/输出流/artifact_ref）落进门节点（v3 挂 timeline_node_006），
//      而 human_confirm 节点在前端 rebuild 判 role=null → 「author 修订步不可见」。
// 本用例在「门节点真实在场」的形态下断言 durable 落点（4 个 campaign fixture 只设
// stage 不建节点，故此处显式 enter_human_confirm 复现现场）。
#[tokio::test]
async fn campaign_stage3_interactive_gate_round_records_author_revision_facts() {
    use crate::product::models::AgentRole;
    use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

    let harness = campaign_stage3_fixture(
        2,
        vec![RevisionScriptStep::Complete(campaign_candidate_v2())],
    )
    .await;
    {
        let mut engine = harness.engine.lock().await;
        engine
            .enter_human_confirm(Some("第一轮人工确认".to_string()))
            .await;
    }
    let first_gate_node = harness
        .engine
        .lock()
        .await
        .active_timeline_node_id()
        .expect("first gate node");

    harness
        .send(WsInMessage::HumanGateFeedback {
            command_id: "cmd-f49-gate-round".to_string(),
            feedback: "反馈A：补充验收条件".to_string(),
        })
        .await;
    let open = harness.await_gate_event("human_gate_turn_open").await;
    let WsOutMessage::HumanGateTurnOpen { turn_id, .. } = open else {
        panic!("expected turn open, got {open:?}");
    };
    let completed = harness.await_gate_event("human_gate_turn_completed").await;
    let WsOutMessage::HumanGateTurnCompleted { artifact_ref, .. } = completed else {
        panic!("expected turn completed, got {completed:?}");
    };
    assert!(!artifact_ref.is_empty());

    let nodes = harness.durable_timeline_nodes();

    // A6：门修订轮的修订事实必须落在一个 author 节点上（与普通 SC 修订同构）。
    let author_node = nodes
        .iter()
        .find(|node| node.node_type == TimelineNodeType::AuthorRun)
        .expect("gate revision must run on an author node");
    assert_ne!(
        author_node.node_id, first_gate_node,
        "门修订不得把修订事实写进门节点"
    );
    let author_detail = harness.durable_node_detail(&author_node.node_id);
    assert_eq!(author_detail.agent_role, Some(AgentRole::Author));
    assert!(
        author_detail
            .prompt
            .as_deref()
            .is_some_and(|prompt| !prompt.trim().is_empty()),
        "修订 prompt 必须持久化在 author 节点"
    );
    assert!(
        !author_detail.streaming_content.trim().is_empty(),
        "修订输出流必须持久化在 author 节点（前端 rebuild 据 author_run detail 出气泡）"
    );
    assert_eq!(
        author_detail
            .artifact_ref
            .as_ref()
            .map(|artifact| artifact.artifact_id.as_str()),
        Some(artifact_ref.as_str()),
        "human_gate_turn_completed 帧与 author 节点 detail 必须同源"
    );
    // 修订成功即收口该节点（与普通修订 author 节点同构），不留 Active 的「修订中」节点。
    let completed_author_node = harness
        .durable_timeline_nodes()
        .into_iter()
        .find(|node| node.node_id == author_node.node_id)
        .expect("author node in durable timeline");
    assert_eq!(
        completed_author_node.status,
        TimelineNodeStatus::Completed,
        "修订完成必须收口 author 节点"
    );
    // 门节点不得携带修订产物引用（现状缺陷：v3 挂门节点）。
    let gate_detail = harness.durable_node_detail(&first_gate_node);
    assert!(
        gate_detail.artifact_ref.is_none(),
        "门节点不得携带修订产物: {:?}",
        gate_detail.artifact_ref
    );

    // A5：新门取代旧门时收口旧门节点；durable 任一时刻只有一个 Active 门节点。
    let superseded_gate = nodes
        .iter()
        .find(|node| node.node_id == first_gate_node)
        .expect("first gate node in durable timeline");
    assert_eq!(
        superseded_gate.status,
        TimelineNodeStatus::Completed,
        "门内轮次切换必须收口被取代的门节点"
    );
    assert!(
        superseded_gate.completed_at.is_some(),
        "收口必须落完成时间"
    );
    let active_gates: Vec<&str> = nodes
        .iter()
        .filter(|node| {
            node.node_type == TimelineNodeType::HumanConfirm
                && node.status == TimelineNodeStatus::Active
        })
        .map(|node| node.node_id.as_str())
        .collect();
    assert_eq!(
        active_gates.len(),
        1,
        "durable 时间线只允许一个 Active 门节点: {active_gates:?}"
    );
    assert_ne!(active_gates[0], first_gate_node, "新门必须是另一个节点");
    // 修订轮完成、门重开：durable 门仍是等待态（人可继续反馈或 approve）。
    assert_eq!(
        harness.session_record().await.status,
        WorkspaceSessionStatus::WaitingForHuman
    );
    assert!(!turn_id.is_empty());
}

// ---------------------------------------------------------------------------
// P0 1.3（REQ-WIGA-05）Task 10 —— 无 driver 的人工门/compile recovery REST
// （POST /api/workspace-sessions/{id}/human-actions）。全程零 WS attachment：
// manager 以 connection_id=None 的 provider_run_context 复用唯一 run 与事件
// 路由；expected_gate_id 与当前 active gate/timeline node 比对。
// ---------------------------------------------------------------------------

struct WorkspaceHumanActionHttpFixture {
    #[allow(dead_code)]
    harness: CampaignStage3Harness,
    router: axum::Router,
    manager: std::sync::Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    session_id: String,
}

async fn workspace_human_action_http_fixture(
    budget: u32,
    script: Vec<RevisionScriptStep>,
) -> WorkspaceHumanActionHttpFixture {
    let harness = campaign_stage3_fixture(budget, script).await;
    let root_path = harness.root.path().to_path_buf();
    // campaign 基座只设 stage 不建节点（F-49 注）；显式 enter_human_confirm
    // 落 durable 门节点——REST 的 expected_gate_id 以 active timeline node
    // 为准，且 manager 引擎晚于该 durable 事实构建。
    {
        let mut engine = harness.engine.lock().await;
        engine
            .enter_human_confirm(Some("REST 人工确认门".to_string()))
            .await;
    }
    // 脚本化修订 provider 注入 manager 消费的 registry（与 WS 用例同一测试
    // 构造器；manager 以 durable 记录重建 engine，不共享 harness 内存引擎）。
    let record = harness.session_record().await;
    let mut registry = crate::cross_cutting::provider_registry::ProviderRegistry::new();
    registry.register(
        record.author_provider.clone(),
        harness.scripted_provider_handle(),
    );
    let state = crate::web::state::WebAppState::with_provider_registry(
        root_path.clone(),
        crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
        registry,
    );
    let manager = state
        .workspace_sessions
        .get_or_create(&harness.session_id, || {
            crate::web::workspace_session::WorkspaceSessionManager::create(
                &state,
                &harness.session_id,
            )
        })
        .await
        .expect("manager for human action fixture");
    let session_id = harness.session_id.clone();
    WorkspaceHumanActionHttpFixture {
        harness,
        router: crate::web::app::build_web_router(state),
        manager,
        session_id,
    }
}

impl WorkspaceHumanActionHttpFixture {
    async fn post_human_action(
        &self,
        body: serde_json::Value,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        use tower::ServiceExt;
        let uri = format!("/api/workspace-sessions/{}/human-actions", self.session_id);
        let response = self
            .router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    async fn active_gate_id(&self) -> Option<String> {
        let engine = self.manager.engine();
        let engine = engine.lock().await;
        engine.active_timeline_node_id()
    }

    async fn session_record(&self) -> WorkspaceSessionRecord {
        self.harness.session_record().await
    }

    fn durable_turn_count(&self) -> usize {
        self.harness.durable_turns().len()
    }

    async fn await_turn_terminal(&self, command_id: &str) -> crate::product::models::HumanGateTurn {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(turn) = self
                    .harness
                    .lifecycle
                    .get_human_gate_turn_by_command_id(&self.session_id, command_id)
                    .expect("turn lookup")
                    && !matches!(
                        turn.status,
                        crate::product::models::HumanGateTurnStatus::Reserved
                            | crate::product::models::HumanGateTurnStatus::Running
                    )
                {
                    return turn;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("turn reaches terminal state")
    }

    /// 修订轮完成 → 复评 → 新门节点（active node 换代）。
    async fn await_gate_rebuild(&self, previous_gate_id: String) -> String {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(current) = self.active_gate_id().await
                    && current != previous_gate_id
                {
                    return current;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("gate rebuilds after revision round")
    }

    async fn await_session_status(
        &self,
        expected: WorkspaceSessionStatus,
    ) -> WorkspaceSessionRecord {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let record = self.session_record().await;
                if record.status == expected {
                    return record;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("session reaches {expected:?}"))
    }
}

/// Task 10 主链：关闭 driver 后 REST feedback 开修订 turn；同 command 重试
/// 幂等（Replayed，只一条 durable turn）；修订轮完成后新门上 approve 走
/// deterministic compile 落 durable Confirmed——全程无 WS attachment。
#[tokio::test]
async fn workspace_human_action_http_feedback_replay_and_approve_without_driver() {
    let fixture = workspace_human_action_http_fixture(
        2,
        vec![RevisionScriptStep::Complete(campaign_candidate_v2())],
    )
    .await;
    let gate_id = fixture.active_gate_id().await.expect("active gate node");

    let feedback_body = serde_json::json!({
        "type": "feedback",
        "command_id": "cmd-rest-fb-1",
        "expected_gate_id": gate_id,
        "feedback": "REST 反馈：补充验收条件",
    });
    let (status, body) = fixture.post_human_action(feedback_body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "accepted");
    assert_eq!(body["command_id"], "cmd-rest-fb-1");
    assert_eq!(body["gate_id"], serde_json::json!(gate_id));

    // 同 command 重试：durable 查重 → Replayed 幂等 accepted，不二开 turn。
    let (retry_status, retry_body) = fixture.post_human_action(feedback_body).await;
    assert_eq!(retry_status, StatusCode::OK);
    assert_eq!(retry_body["state"], "accepted");

    fixture.await_turn_terminal("cmd-rest-fb-1").await;
    assert_eq!(
        fixture.durable_turn_count(),
        1,
        "同 command 重试只创建一个 turn"
    );

    // 修订轮完成 → 新门（新 active 节点）；approve 必须用新 gate id。
    let gate_id_2 = fixture.await_gate_rebuild(gate_id.clone()).await;
    let (approve_status, approve_body) = fixture
        .post_human_action(serde_json::json!({
            "type": "approve",
            "command_id": "cmd-rest-approve-1",
            "expected_gate_id": gate_id_2,
        }))
        .await;
    assert_eq!(approve_status, StatusCode::OK);
    assert_eq!(approve_body["state"], "accepted");

    // approve 走 deterministic compile：durable Confirmed + Completed 相位，
    // 且零 provider start（compile 不经 provider）。
    let confirmed = fixture
        .await_session_status(WorkspaceSessionStatus::Confirmed)
        .await;
    assert_eq!(
        confirmed.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    assert_eq!(
        fixture.harness.scripted_provider_starts(),
        1,
        "仅修订 turn 一次 provider start；approve 是 deterministic compile"
    );
}

/// Task 10 防误答面：门 id 不匹配 409 零副作用；compile recovery 只在
/// WorkItemPlanCompileRecovery 节点放行（当前 human gate → 422）；空白
/// command_id 400；会话全程保持等待态。
#[tokio::test]
async fn workspace_human_action_http_rejects_stale_gate_recovery_and_blank_command() {
    let fixture = workspace_human_action_http_fixture(1, vec![]).await;
    let gate_id = fixture.active_gate_id().await.expect("active gate node");

    let (mismatch_status, mismatch_body) = fixture
        .post_human_action(serde_json::json!({
            "type": "feedback",
            "command_id": "cmd-stale-1",
            "expected_gate_id": "node_not_the_gate",
            "feedback": "过期门的反馈",
        }))
        .await;
    assert_eq!(mismatch_status, StatusCode::CONFLICT);
    assert_eq!(mismatch_body["code"], "human_action_gate_mismatch");
    assert!(
        fixture.harness.durable_turns().is_empty(),
        "门 id 不匹配不得开 turn（不误答）"
    );

    let (recovery_status, recovery_body) = fixture
        .post_human_action(serde_json::json!({
            "type": "compile_recovery",
            "command_id": "cmd-recovery-1",
            "expected_gate_id": gate_id,
            "action": "continue",
            "reason": null,
        }))
        .await;
    assert_eq!(recovery_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(recovery_body["code"], "invalid_compile_recovery_action");
    assert_eq!(
        fixture.session_record().await.status,
        WorkspaceSessionStatus::WaitingForHuman
    );

    let (blank_status, blank_body) = fixture
        .post_human_action(serde_json::json!({
            "type": "abandon",
            "command_id": "   ",
            "expected_gate_id": gate_id,
        }))
        .await;
    assert_eq!(blank_status, StatusCode::BAD_REQUEST);
    assert_eq!(blank_body["code"], "invalid_command_id");
    assert_eq!(
        fixture.session_record().await.status,
        WorkspaceSessionStatus::WaitingForHuman,
        "空白 command_id 零副作用"
    );
}

/// Task 10 abandon：REST 关门 → durable Terminated，快照清理与 WS 路径一致。
#[tokio::test]
async fn workspace_human_action_http_abandon_terminates_session_without_driver() {
    let fixture = workspace_human_action_http_fixture(1, vec![]).await;
    let gate_id = fixture.active_gate_id().await.expect("active gate node");
    let (status, body) = fixture
        .post_human_action(serde_json::json!({
            "type": "abandon",
            "command_id": "cmd-rest-abandon-1",
            "expected_gate_id": gate_id,
        }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "accepted");

    let record = fixture
        .await_session_status(WorkspaceSessionStatus::Terminated)
        .await;
    assert!(
        record.human_gate_snapshot.is_none(),
        "关门后快照清理与 legacy Terminate 路径一致"
    );
    assert!(
        fixture.harness.durable_turns().is_empty(),
        "abandon 零 durable turn"
    );
    assert_eq!(
        fixture.harness.scripted_provider_starts(),
        0,
        "abandon 零 provider start"
    );
}

/// Task 10 approve 失败面：把候选 IR 的首个 item handoff 加一个无消费者契约
///（批准链 compile 以 source/IR 为源；freshness 只看 source hash/编译器版本/
/// 报告，三者不动）——canonical 校验（unconsumed_required_handoff，Error 级）
/// 使 deterministic compile 如实失败并落 Failed 事务：REST approve 422 上抛
/// single_candidate_approval_compile_failed，会话不得宣称 Confirmed
///（REQ-CG-04）。
#[tokio::test]
async fn workspace_human_action_http_approve_compile_failure_never_confirms() {
    let fixture = workspace_human_action_http_fixture(1, vec![]).await;
    {
        use crate::product::work_item_plan_source_store::SourceStoreScope;
        let record = fixture.harness.session_record().await;
        let scope = SourceStoreScope {
            project_id: record.project_id.clone(),
            issue_id: record.issue_id.clone(),
            plan_id: record.entity_id.clone(),
        };
        let source_store =
            crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
                fixture.harness.lifecycle.app_paths(),
            );
        let ir_ref = record
            .plan_candidate_ir_ref
            .clone()
            .expect("fixture IR ref");
        let mut ir = source_store
            .get_plan_candidate_ir(&scope, &ir_ref)
            .expect("fixture IR");
        ir.id = format!("{}-dirty", ir.id);
        ir.ir.items[0]
            .contract
            .handoff_contract
            .provided_contract_refs
            .push("contract.unconsumed.rest".to_string());
        ir.content_hash = ir.content_hash().expect("dirty IR content hash");
        let dirty_ir_ref = source_store
            .put_plan_candidate_ir(&scope.project_id, &scope.issue_id, &scope.plan_id, &ir)
            .expect("persist dirty IR");
        let mut dirty_record = record.clone();
        dirty_record.plan_candidate_ir_ref = Some(dirty_ir_ref);
        crate::product::json_store::write_json(
            &fixture
                .harness
                .app_paths
                .issue_root(&dirty_record.project_id, &dirty_record.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", dirty_record.id)),
            &dirty_record,
        )
        .expect("rebind dirty IR ref");
    }
    let gate_id = fixture.active_gate_id().await.expect("active gate node");
    let (status, body) = fixture
        .post_human_action(serde_json::json!({
            "type": "approve",
            "command_id": "cmd-rest-approve-dirty",
            "expected_gate_id": gate_id,
        }))
        .await;
    // 该失败形态无 Failed 事务（引擎未落 findings 事务）——与 WS 层同形映射：
    // 通用引擎错误面（human_action_engine_error，5xx），绝不 200/accepted。
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["code"], "human_action_engine_error");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("compile failed; human gate remains open")),
        "错误必须如实携带 deterministic compile 失败语义: {body}"
    );
    assert_eq!(
        fixture.session_record().await.status,
        WorkspaceSessionStatus::WaitingForHuman,
        "compile 失败不得宣称 Confirmed（门保持等待人工）"
    );
    assert_eq!(
        fixture.harness.scripted_provider_starts(),
        0,
        "approve 失败同样零 provider start（deterministic compile）"
    );
}

/// Task 10：REST 人工命令不反向提升 observer WS 权限——人工门命令族
/// （feedback/approve/abandon/compile recovery）在 observer 连接上仍被
/// 只读仲裁拒绝（OBSERVER_WRITE_REJECTED 的仲裁来源）。
#[tokio::test]
async fn workspace_human_action_observer_ws_writes_stay_rejected() {
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_types::WsInMessage;

    let manager = crate::web::workspace_session::WorkspaceSessionManager::test_fixture(
        "session_human_action_observer",
    );
    let (observer_tx, _observer_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("observer_connection", observer_tx.clone());
    manager.bind_role(&observer_tx, ConnectionRole::Observer, None);
    for message in [
        WsInMessage::HumanGateFeedback {
            command_id: "cmd-obs-1".to_string(),
            feedback: "观察者反馈".to_string(),
        },
        WsInMessage::Confirm,
        WsInMessage::AbandonHumanGate {
            command_id: "cmd-obs-2".to_string(),
        },
        WsInMessage::WorkItemPlanCompileRecoveryAction {
            action: crate::web::workspace_ws_types::WorkItemPlanCompileRecoveryActionDto::Continue,
            reason: None,
        },
    ] {
        assert!(
            matches!(
                manager.arbitrate("observer_connection", &message),
                Err(ConnectionRole::Observer)
            ),
            "observer 人工命令写面必须维持拒绝: {:?}",
            message
        );
    }
}
