/// Step 1 —— 总路径：人为注入 human_required 后，
/// `request-change:A;request-change:B;confirm` 两个 typed turn 各经完整
/// revision/compiler/validator，approve→compile→durable Confirmed。
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
    assert_eq!(remaining_2, 0, "第二个 turn 预算再减一");
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
    assert_eq!(harness.budget_remaining(), 0, "预算恰好扣两次");
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
            1,
            Some(0),
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

    // 无 legacy RequestChange：本用例全程只发 typed 形态；服务端对 SC 的
    // RequestChange 分支必须直接拒绝（防回归锁）。
    {
        let mut engine = harness.engine.lock().await;
        let reject = engine
            .handle_human_gate_termination(HumanConfirmDecision::RequestChange)
            .await;
        assert!(reject.is_err(), "SC 人工门必须拒绝 legacy RequestChange");
    }
}

/// Step 3a —— 预算耗尽：feedback 明确 reason 拒绝且零副作用；approve/abandon 仍可用。
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
    harness
        .send(WsInMessage::HumanConfirm {
            decision: HumanConfirmDecision::Terminate,
            payload: None,
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

/// Step 3b —— 超长反馈：反馈超长与构造 prompt 超预算各一案，
/// turn/budget/ledger/session 全零变化；缩短后新 command 可受理。
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

/// Step 4a —— 单飞：首 turn 阻塞期间 feedback/approve/abandon 全部 gate_busy，
/// 无排队/关门/预算/ledger 增量；释放后才允许下一决定。
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
        .send_isolated_worker(WsInMessage::HumanConfirm {
            decision: HumanConfirmDecision::Terminate,
            payload: None,
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

/// Step 4b —— reservation crash 恰好恢复一次：
/// CAS durable 后/启动前、启动 ledger 后/完成前两个 fault point 重启并同
/// command resend；同 turn_id、预算恰减一次、provider alive 等待 / dead 同
/// turn attempt_no++、不超过上限、事件前缀不改。
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
    let recovered_engine = |root: &TempDir, lifecycle: &LifecycleStore| {
        let (event_tx, _event_rx) = mpsc::channel(64);
        WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
            lifecycle.clone(),
            event_tx,
            WorkspaceSession::from_record(
                lifecycle
                    .get_workspace_session(&harness.session_id)
                    .expect("durable session"),
            ),
        )
    };
    let mut recovered = recovered_engine(&harness.root, &harness.lifecycle);
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
    assert_eq!(harness.budget_remaining(), 1, "预算恰减一次");
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
        let mut recovered_alive = recovered_engine(&harness.root, &harness.lifecycle);
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
    let mut recovered_dead = recovered_engine(&harness.root, &harness.lifecycle);
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

/// Step 2 —— 8.2a takeover：stopped_needs_human auto parent 两次 takeover 幂等，
/// child interactive + 门可接 typed feedback，继承 snapshot/candidate/diagnostic
/// refs/预算，parent bytes/event prefix 完全不变；不满足前提时拒绝且无 child。
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
        let current_run = Arc::new(Mutex::new(None));
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
            run_context: ProviderRunContext {
                provider_registry: Arc::new(registry),
                engine,
                current_run: current_run.clone(),
                workspace_runs: workspace_runs.clone(),
                session_id: child.id.clone(),
                next_run_id: Arc::new(Mutex::new(0)),
                app_paths: harness.app_paths.clone(),
                session_record: child.clone(),
            },
            outbound_tx,
            current_run,
            workspace_runs,
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
