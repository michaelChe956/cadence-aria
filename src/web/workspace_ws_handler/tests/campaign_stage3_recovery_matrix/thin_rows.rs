// ============================================================================
// 薄行 ×3：turn reservation / takeover / advance journal。
// 每行以既有单窗口测试（8.2/8.2a/8.3）同款断言语义做最小复跑——只复跑身份/
// 账本不变量（同 command、同 id、ledger/attempt_no 对账、重启恢复），不重复
// 其深测面（故障 flavor 循环、busy 窗口、五 checkpoint 全扫、ws 全上下文
// 分发等）。深覆盖仍由 8.2/8.2a/8.3 原测试持有。
// ============================================================================

/// 薄行共用：从 durable 记录重建引擎（WS 断开 + 进程/存储重开，零内存态）。
fn matrix_reopened_engine(harness: &CampaignStage3Harness, checkpoints: &str) -> WorkspaceEngine {
    let record = harness.session_record_blocking();
    let (event_tx, _event_rx) = mpsc::channel(64);
    WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(harness.root.path().join(checkpoints))),
        harness.lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(record),
    )
}

/// 薄行 1 —— turn reservation（8.2 同款语义最小复跑；provider alive/dead
/// 两态只在本行区分）。fault point A（CAS durable 后/启动前）与 fault
/// point B（启动 ledger 后/完成前）：重开恢复后以 durable ledger 与
/// attempt_no 对账——Reserved 恢复 attempt 1 恰一次完成；alive 等待不动账；
/// dead 同 turn attempt_no++ 至 2；同 command 重发一律 Replay 零增量。
#[tokio::test]
async fn campaign_stage3_recovery_matrix_turn_reservation_row_alive_and_dead() {
    // —— fault point A：CAS durable 后 / 启动前 ——
    let harness = campaign_stage3_fixture(2, Vec::new()).await;
    {
        let mut engine = harness.engine.lock().await;
        let opened = engine
            .handle_human_gate_feedback(HumanGateFeedbackInput {
                command_id: "cmd-campaign-matrix-turn-a".to_string(),
                feedback: "矩阵行：崩溃前的反馈".to_string(),
            })
            .await
            .expect("reserve turn");
        match opened {
            HumanGateCommandOutcome::TurnOpened {
                turn,
                remaining_budget,
                ..
            } => {
                assert_eq!(remaining_budget, 1);
                assert_eq!(turn.status, HumanGateTurnStatus::Reserved, "启动前崩溃");
            }
            other => panic!("expected opened turn, got {other:?}"),
        }
    }
    let turn_a = harness
        .durable_turns()
        .into_iter()
        .find(|turn| turn.command_id == "cmd-campaign-matrix-turn-a")
        .expect("durable reserved turn");
    // 重启恢复：同 turn 的 attempt 1。
    let mut recovered = matrix_reopened_engine(&harness, "matrix-turn-a");
    let actions = recovered
        .recover_human_gate_turns(false)
        .expect("recover fault point a");
    assert_eq!(
        actions,
        vec![(
            turn_a.turn_id.clone(),
            HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 1 }
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
    assert!(matches!(accepted, ScManualRevisionResult::Accepted { .. }));
    let recovered_turn = harness
        .lifecycle
        .get_human_gate_turn(&harness.session_id, &turn_a.turn_id)
        .expect("durable turn");
    assert_eq!(recovered_turn.turn_id, turn_a.turn_id, "同 turn_id");
    assert_eq!(recovered_turn.status, HumanGateTurnStatus::Completed);
    assert_eq!(recovered_turn.attempt_no, 1, "attempt_no 对账：恰一次");
    // C2（改写登记）：修订完成后 Evaluate route 重建 MUST carry-forward
    // durable 剩余（fixture 预算 2，本轮 reserve 后 1），不回填默认 3。
    assert_eq!(
        harness.budget_remaining(),
        1,
        "门预算经路由重建接续（改写登记：原重置断言 3）"
    );
    assert_eq!(harness.provider_start_keys().len(), 1, "ledger 恰一项");
    // 重开后再同 command：Replay 同一终态 turn，零增量。
    let mut replay_a = matrix_reopened_engine(&harness, "matrix-turn-a-replay");
    let replayed = replay_a
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd-campaign-matrix-turn-a".to_string(),
            feedback: "矩阵行：同 command 重发".to_string(),
        })
        .await
        .expect("replay");
    assert!(matches!(
        &replayed,
        HumanGateCommandOutcome::Replayed { turn } if turn.turn_id == turn_a.turn_id
    ));
    assert_eq!(harness.budget_remaining(), 1);
    assert_eq!(harness.provider_start_keys().len(), 1);

    // —— fault point B：启动 ledger 后 / 完成前 ——
    let harness = campaign_stage3_fixture(2, Vec::new()).await;
    let turn_b = {
        let mut engine = harness.engine.lock().await;
        let opened = engine
            .handle_human_gate_feedback(HumanGateFeedbackInput {
                command_id: "cmd-campaign-matrix-turn-b".to_string(),
                feedback: "矩阵行：启动后崩溃前的反馈".to_string(),
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
    assert_eq!(harness.provider_start_keys().len(), 1);

    // provider alive：恢复分类等待，ledger/attempt_no 不动。
    {
        let mut alive = matrix_reopened_engine(&harness, "matrix-turn-b-alive");
        let alive_actions = alive.recover_human_gate_turns(true).expect("recover alive");
        assert_eq!(
            alive_actions,
            vec![(
                turn_b.turn_id.clone(),
                HumanGateRecoveryAction::WaitForProvider
            )],
            "provider alive 则等待"
        );
        assert_eq!(
            harness.provider_start_keys().len(),
            1,
            "等待期 ledger 不增量"
        );
        let after_alive = harness
            .lifecycle
            .get_human_gate_turn(&harness.session_id, &turn_b.turn_id)
            .expect("turn after alive wait");
        assert_eq!(after_alive.status, HumanGateTurnStatus::Running);
        assert_eq!(after_alive.attempt_no, 1);
    }

    // provider dead：同 turn attempt_no++（≤ 上限），ledger 增 attempt:2。
    let mut dead = matrix_reopened_engine(&harness, "matrix-turn-b-dead");
    let dead_actions = dead.recover_human_gate_turns(false).expect("recover dead");
    assert_eq!(
        dead_actions,
        vec![(
            turn_b.turn_id.clone(),
            HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 2 }
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
    let keys = harness.provider_start_keys();
    assert_eq!(keys.len(), 2, "attempt:1 + attempt:2");
    assert!(
        keys.iter().any(|key| key.ends_with(":attempt:2")),
        "ledger 对账: {keys:?}"
    );
    assert_eq!(harness.budget_remaining(), 1, "恢复不重复扣预算");
    // 重开后同 command：Replay 同 turn，无第三次 start。
    let mut replay_b = matrix_reopened_engine(&harness, "matrix-turn-b-replay");
    let replayed_b = replay_b
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd-campaign-matrix-turn-b".to_string(),
            feedback: "矩阵行：同 command 重发（dead 恢复后）".to_string(),
        })
        .await
        .expect("replay");
    assert!(matches!(
        &replayed_b,
        HumanGateCommandOutcome::Replayed { turn } if turn.turn_id == turn_b.turn_id
    ));
    assert_eq!(harness.provider_start_keys().len(), 2);
    assert_eq!(harness.budget_remaining(), 1);
}

/// 薄行 2 —— takeover（8.2a 同款语义最小复跑）：stopped_needs_human auto
/// parent 两次 takeover 幂等；重开（全新 lifecycle 读 + 从 durable child 记录
/// 重建引擎）后 child 门接 typed feedback，预算从继承值扣一次；parent bytes
/// 全程不变。fatal 拒绝/child ws 全上下文分发等深测面仍在 8.2a。
#[tokio::test]
async fn campaign_stage3_recovery_matrix_takeover_row_reconnect_continues_on_child() {
    use axum::extract::{Path, State};

    let harness = campaign_stage3_fixture(2, Vec::new()).await;
    // 组装 stopped_needs_human auto parent（无 fatal/persistence diagnostic）。
    let mut parent = harness.session_record().await;
    parent.status = WorkspaceSessionStatus::StoppedNeedsHuman;
    parent.run_policy = RunPolicy::AutoIfValid;
    parent.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
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
    // 两次 takeover：同一 child / 同一 takeover_event（幂等）。
    let first = workspace_session_takeover(State(state.clone()), Path(parent.id.clone()))
        .await
        .expect("first takeover")
        .0;
    let second = workspace_session_takeover(State(state), Path(parent.id.clone()))
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

    // 重开：全新 lifecycle 读 + 从 durable child 记录重建引擎（进程/存储重开）。
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
        "预算继承（原预算源交接给 child）"
    );
    assert_eq!(child.human_gate_snapshot, parent.human_gate_snapshot);
    assert!(
        child.provider_start_ledger.is_empty(),
        "不复制 in-flight ledger"
    );
    // 重开后的 child 门接 typed feedback：预算从继承值扣一次。
    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut child_session = WorkspaceSession::from_record(child.clone());
    child_session.stage = WorkspaceStage::HumanConfirm;
    child_session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    child_session.artifact = Some(ArtifactPayload::Markdown {
        markdown: campaign_candidate_base(),
        diff: None,
    });
    let mut child_engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(
            harness
                .root
                .path()
                .join("matrix-takeover-child-checkpoints"),
        )),
        harness.lifecycle.clone(),
        event_tx,
        child_session,
    );
    let opened = child_engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd-campaign-matrix-takeover-child".to_string(),
            feedback: "矩阵行：child 上的反馈".to_string(),
        })
        .await
        .expect("child feedback must open a turn");
    match opened {
        HumanGateCommandOutcome::TurnOpened {
            turn,
            remaining_budget,
            ..
        } => {
            assert_eq!(turn.session_id, child_id, "turn 落在 child session");
            assert_eq!(remaining_budget, 1, "child 预算从继承值扣一次");
        }
        other => panic!("expected child turn opened, got {other:?}"),
    }
    let child_turns = harness
        .lifecycle
        .list_human_gate_turns(&child_id)
        .expect("child turns");
    assert_eq!(child_turns.len(), 1, "child turn durable");

    // parent bytes 完全不变（含 child feedback 之后）；事件 durable；预算对账。
    assert_eq!(
        std::fs::read(&parent_path).expect("parent bytes after"),
        parent_bytes_before,
        "parent bytes 完全不变"
    );
    let event = harness
        .lifecycle
        .get_human_gate_takeover_event(&parent.id)
        .expect("takeover event lookup")
        .expect("event exists");
    assert_eq!(event.child_session_id, child_id);
    assert_eq!(event.parent_session_id, parent.id);
    let child_after = harness
        .lifecycle
        .get_workspace_session(&child_id)
        .expect("child after feedback");
    assert_eq!(
        child_after
            .human_gate_snapshot
            .as_ref()
            .expect("snapshot")
            .manual_repairs_remaining,
        1
    );
    let parent_after = harness
        .lifecycle
        .get_workspace_session(&parent.id)
        .expect("parent after");
    assert_eq!(
        parent_after
            .human_gate_snapshot
            .as_ref()
            .expect("parent snapshot")
            .manual_repairs_remaining,
        2,
        "parent bytes 不变 ⇒ 快照不变"
    );
}

/// 薄行 3 —— advance journal（8.3 同款语义最小复跑）：单 checkpoint
/// （attempt 持久化后）真实 ws 分发内崩溃 → 重启 worker 同 command →
/// advance_completed；同 record identity、复用 checkpoint 前 attempt、
/// 全 issue 恒一 attempt；provider ledger 在 advance 全程 byte 级不变。
/// record/binding/lock/units 五 checkpoint 全扫深测仍在 8.3。
#[tokio::test]
async fn campaign_stage3_recovery_matrix_advance_journal_row_restart_resumes_same_command() {
    let harness = Arc::new(confirmed_campaign_harness().await);
    let advance_store = AdvanceStore::new(harness.app_paths.clone());
    let coding_store = CodingAttemptStore::new(harness.app_paths.clone());
    let command_id = "cmd-campaign-matrix-adv-attempt".to_string();
    let request = crate::product::workspace_engine::AdvanceInput {
        command_id: command_id.clone(),
        project_id: harness.project_id.clone(),
        issue_id: harness.issue_id.clone(),
        plan_id: harness.record_plan_id(),
    };
    let ledger_before = harness
        .session_record_blocking()
        .provider_start_ledger
        .clone();
    assert!(
        ledger_before.is_empty(),
        "Confirmed 基座无 provider start（账本对账起点）"
    );
    let _failpoint = register_advance_initialization_failpoint(
        &request,
        AdvanceInitializationFailpoint::AttemptPersisted,
        AdvanceInitializationFailpointMode::Crash,
    );

    // crash 在真实 ws 分发路径内发生（panic 只影响该分发 task）。
    let crashed = {
        let harness = Arc::clone(&harness);
        let command_id = command_id.clone();
        tokio::spawn(async move {
            harness.send(WsInMessage::Advance { command_id }).await;
        })
    };
    assert!(
        crashed.await.is_err(),
        "crash failpoint 必须中断真实分发路径"
    );

    // 崩溃后 durable 证据：record 已落盘且未 Ready；attempt 已持久化（checkpoint 后）。
    let first_record = advance_store
        .get_advance_by_command_id(&harness.project_id, &harness.issue_id, &command_id)
        .expect("crash record lookup")
        .expect("record 必须在 checkpoint 前已落盘");
    assert_eq!(first_record.status, AdvanceStatus::Initializing);
    let first_group = coding_store
        .get_group_initialization(
            &harness.project_id,
            &harness.issue_id,
            &harness.record_plan_id(),
        )
        .ok();
    let first_attempt_id = first_group.as_ref().map(|group| group.attempt.id.clone());
    assert!(
        first_attempt_id.is_some(),
        "AttemptPersisted checkpoint 后 attempt 必须已落盘"
    );

    // 重启（全新 engine 实例）后同 command 恢复到 Ready。
    let mut outbound = Box::pin(
        send_via_restarted_worker(
            &harness,
            WsInMessage::Advance {
                command_id: command_id.clone(),
            },
        )
        .await,
    );
    let completed = next_restarted_outbound(&mut outbound, "advance_completed").await;
    let WsOutMessage::AdvanceCompleted {
        attempt_id: resumed_attempt_id,
        ..
    } = completed
    else {
        panic!("expected advance_completed after restart");
    };

    let final_record = advance_store
        .get_advance_by_command_id(&harness.project_id, &harness.issue_id, &command_id)
        .expect("final record lookup")
        .expect("final record durable");
    assert_eq!(final_record.id, first_record.id, "同一 record identity");
    assert_eq!(final_record.status, AdvanceStatus::Ready);
    assert_eq!(
        final_record.attempt_id.as_deref(),
        Some(resumed_attempt_id.as_str())
    );
    let journal = advance_store
        .get_advance_initialization(&final_record)
        .expect("journal lookup")
        .expect("journal durable");
    assert_eq!(journal.phase, AdvanceInitializationPhase::Ready);
    assert_eq!(
        resumed_attempt_id,
        first_attempt_id.expect("checkpoint 前 attempt id"),
        "恢复必须复用 checkpoint 前的 attempt identity"
    );
    assert_eq!(
        coding_store
            .list_attempts_for_issue(&harness.project_id, &harness.issue_id)
            .expect("attempts after resume")
            .len(),
        1,
        "crash+resume 全程只允许一个 group attempt"
    );
    // 账本对账：advance 全程 provider ledger byte 级不变（Ready 不需要 provider）。
    assert_eq!(
        harness.session_record_blocking().provider_start_ledger,
        ledger_before,
        "advance 前后 provider ledger byte 级相等"
    );
}

// ---------------------------------------------------------------------------
// B2d（Task 5.6，tasks.md 5.6 / 设计 §5.3 sc_manual_revision 行）：SC 人工
// 修订 turn 的 native mismatch → typed 等待 → 显式 fresh → 原 human gate
// 全链。同 turn/command/source_hash/attempt/budget/reservation 不因 native
// 漂移重臂；fresh 全新 native（resume=None）且 prompt 重装同 turn feedback；
// 等待期重连不自动 ResumeSameTurn（不增 attempt/零 spawn）。
// ---------------------------------------------------------------------------

/// T18 记录型 fake revision provider：记录完整 input（prompt/resume 断言
/// 面），返回固定候选输出。mismatch 等待阶段零启动由 starts 计数对账。
struct T18RecordingRevisionProvider {
    output: String,
    inputs: StdMutex<Vec<StreamingProviderInput>>,
    starts: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for T18RecordingRevisionProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inputs.lock().expect("t18 inputs lock").push(input);
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        let output = self.output.clone();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        output, None,
                    ),
                ))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: Some("t18-fresh-native".to_string()),
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "t18 uses start only",
            0,
        ))
    }
}

/// 取下一条 error 帧（跳过非 error 帧）——mismatch 等待消息的观测面。
async fn t18_next_error_message(outbound_rx: &mut mpsc::Receiver<OutboundControl>) -> String {
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), outbound_rx.recv())
            .await
            .expect("t18 outbound within timeout")
            .expect("t18 outbound channel open");
        let OutboundControl::Text(json) = frame else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(&json).expect("t18 ws json");
        let frame_type = value["type"].as_str().unwrap_or_default();
        if frame_type == "error" || frame_type == "human_gate_turn_failed" {
            return value["message"].as_str().unwrap_or_default().to_string();
        }
    }
}

/// T18 共用 registry/run_context 组装（fresh provider 同源注册）。
async fn t18_run_context(
    harness: &CampaignStage3Harness,
    provider: Arc<T18RecordingRevisionProvider>,
) -> ProviderRunContext {
    let record = harness.session_record().await;
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        provider as Arc<dyn StreamingProviderAdapter>,
    );
    ProviderRunContext::test_fixture(
        Arc::new(registry),
        harness.engine.clone(),
        WorkspaceRunRegistry::default(),
        harness.session_id.clone(),
        harness.app_paths.clone(),
        record,
    )
}

#[tokio::test]
async fn anchor_t18_manual_turn_fresh_preserves_turn_gate_and_budget() {
    let harness = campaign_stage3_fixture(2, Vec::new()).await;
    let session_id = harness.session_id.clone();
    let provider = Arc::new(T18RecordingRevisionProvider {
        output: campaign_candidate_v2(),
        inputs: StdMutex::new(Vec::new()),
        starts: std::sync::atomic::AtomicUsize::new(0),
    });

    // ① 门内人工反馈开 turn（Reserved；预算 2→1、reservation/ledger 恰一）。
    let turn_before = {
        let mut engine = harness.engine.lock().await;
        let opened = engine
            .handle_human_gate_feedback(HumanGateFeedbackInput {
                command_id: "cmd-campaign-matrix-t18-manual-fresh".to_string(),
                feedback: "T18:把后端接口标题改为round-2口径".to_string(),
            })
            .await
            .expect("open t18 manual turn");
        match opened {
            HumanGateCommandOutcome::TurnOpened { turn, .. } => turn,
            other => panic!("expected opened turn, got {other:?}"),
        }
    };
    assert_eq!(harness.budget_remaining(), 1, "预算恰扣一次");

    // ② seed 漂移形态：旧 Author native 会话在场且已被 tombstone 吊销——
    //    重连 resume 判定必得 FrozenBindingUnavailable（native 在而绑定
    //    不可用）。
    {
        let mut record = harness.session_record_blocking();
        record.provider_conversations = vec![crate::product::models::ProviderConversationRef {
            role: crate::product::models::ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "t18-old-native".to_string(),
            updated_at: "2026-10-12T00:00:00Z".to_string(),
            last_node_id: Some("timeline_node_001".to_string()),
        }];
        record.superseded_native_sessions =
            vec![crate::product::models::SupersededNativeSessionRecord {
                role: crate::product::models::ProviderConversationRole::Author,
                provider: ProviderName::ClaudeCode,
                native_session_id: "t18-old-native".to_string(),
                reason_code: "resume_fingerprint_unavailable".to_string(),
                recovery_id: "seed".to_string(),
                superseded_at: "2026-10-12T00:00:00Z".to_string(),
            }];
        crate::product::json_store::write_json(
            &harness
                .app_paths
                .issue_root(&harness.project_id, &harness.issue_id)
                .join("workspace-sessions")
                .join(format!("{session_id}.json")),
            &record,
        )
        .expect("seed t18 superseded native");
        let mut engine = harness.engine.lock().await;
        let repository_path = engine.session.repository_path.clone();
        let artifact = engine.session.artifact.clone();
        let stage = engine.session.stage.clone();
        let session_status = engine.session.session_status.clone();
        engine.session = WorkspaceSession::from_record(record);
        engine.session.repository_path = repository_path;
        engine.session.artifact = artifact;
        engine.session.stage = stage;
        engine.session.session_status = session_status;
    }
    let node_ids_after_open: Vec<String> = harness
        .durable_timeline_nodes()
        .into_iter()
        .map(|node| node.node_id)
        .collect();
    let ledger_before =
        serde_json::to_vec(&harness.session_record_blocking().provider_start_ledger)
            .expect("serialize ledger before");

    // ③ 重连恢复#1：漂移尚未检出，Reserved→ResumeSameTurn(1)（既有语义），
    //    manager 同款 from_handler spawn 修订臂 → Ordinary precheck 检出
    //    native mismatch → typed 等待（不 fail turn、不 rearm、零 start）。
    {
        let mut reopened = matrix_reopened_engine(&harness, "matrix-t18-reconnect-1");
        let actions = reopened
            .recover_human_gate_turns(false)
            .expect("t18 reconnect 1");
        assert!(
            matches!(
                actions.as_slice(),
                [(
                    turn_id,
                    HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 1 }
                )] if turn_id == &turn_before.turn_id
            ),
            "漂移检出前恢复语义不变，got {actions:?}"
        );
    }
    let (arm_tx, mut arm_rx) = mpsc::channel::<OutboundControl>(16);
    let run_context = t18_run_context(&harness, provider.clone()).await;
    spawn_provider_run_from_handler(
        run_context,
        ProviderRunKind::HumanGateScManualRevision {
            turn_id: turn_before.turn_id.clone(),
            prompt: String::new(),
        },
        arm_tx,
    )
    .await
    .expect("spawn t18 ordinary arm");
    let waiting_message = t18_next_error_message(&mut arm_rx).await;
    assert!(
        waiting_message.contains("resume_fingerprint_unavailable"),
        "mismatch 等待消息必须携带稳定判别码，got: {waiting_message}"
    );
    assert!(
        waiting_message.contains("显式"),
        "mismatch 等待消息必须给出显式恢复出口，got: {waiting_message}"
    );
    let after_mismatch = harness.session_record_blocking();
    let turn_after_wait = harness
        .lifecycle
        .get_human_gate_turn(&session_id, &turn_before.turn_id)
        .expect("t18 turn after wait");
    assert_eq!(
        turn_after_wait.status,
        HumanGateTurnStatus::Running,
        "mismatch 等待不 fail turn（原 Running 保持）"
    );
    assert_eq!(turn_after_wait.attempt_no, turn_before.attempt_no);
    assert!(
        turn_after_wait.failure_class.is_none(),
        "mismatch 不是业务失败"
    );
    let recovery = after_mismatch
        .lc_resume_recovery
        .clone()
        .expect("mismatch must persist a waiting recovery record");
    assert_eq!(
        recovery.state,
        crate::product::models::LcResumeRecoveryState::Waiting
    );
    assert_eq!(recovery.reason_code, "resume_fingerprint_unavailable");
    let crate::product::models::LcRecoveryRunContext::ScManualRevision {
        turn_id: payload_turn_id,
        command_id,
        source_hash,
        attempt_no,
        budget_reserved,
        human_gate_reservation_key,
        feedback_text,
        ..
    } = &recovery.run_context
    else {
        panic!(
            "t18 必须捕获 sc_manual_revision tag，got {:?}",
            recovery.run_context
        )
    };
    assert_eq!(payload_turn_id, &turn_before.turn_id);
    assert_eq!(command_id, &turn_before.command_id);
    assert_eq!(source_hash, &turn_before.source_hash);
    assert_eq!(*attempt_no, turn_before.attempt_no);
    assert_eq!(*budget_reserved, turn_before.budget_reserved);
    assert_eq!(
        feedback_text.as_deref(),
        Some(turn_before.feedback_text.as_str()),
        "等待冻结同 turn feedback"
    );
    assert_eq!(
        human_gate_reservation_key.as_deref(),
        after_mismatch
            .human_gate_reservation
            .as_ref()
            .map(|reservation| reservation.provider_start_idempotency_key.as_str()),
        "等待保留原 reservation key"
    );
    assert_eq!(
        provider.starts.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "mismatch 等待零 provider start"
    );
    assert_eq!(
        serde_json::to_vec(&after_mismatch.provider_start_ledger).expect("serialize ledger"),
        ledger_before,
        "等待不追加 provider_start_ledger（无 attempt:2/第二次 reserve）"
    );
    assert_eq!(harness.budget_remaining(), 1, "等待不二次扣预算");
    // 等待零新节点身份；旧 native 的 last AuthorRun 节点（种子 conversation
    // 的 timeline_node_001，仅引擎内存在场）按 §4.3 幂等终态化——Failed +
    // LC waiting 摘要，终态化将其首次持久化是唯一允许的节点写入。
    {
        let nodes_after_wait = harness.durable_timeline_nodes();
        assert!(
            nodes_after_wait
                .iter()
                .all(|node| node.node_id == "timeline_node_001"
                    || node_ids_after_open.contains(&node.node_id)),
            "等待只终态化旧节点，不建新节点身份，got {nodes_after_wait:?}"
        );
        if let Some(old_node) = nodes_after_wait
            .iter()
            .find(|node| node.node_id == "timeline_node_001")
        {
            assert!(
                matches!(
                    old_node.status,
                    crate::web::workspace_ws_types::TimelineNodeStatus::Failed
                ),
                "旧 native last 节点必须幂等终态化，got {:?}",
                old_node.status
            );
            assert!(
                old_node
                    .summary
                    .as_deref()
                    .unwrap_or_default()
                    .contains("LC resume recovery waiting"),
                "终态化节点必须携带 LC waiting 摘要，got {:?}",
                old_node.summary
            );
        }
    }
    assert_eq!(
        after_mismatch.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
        "门保持开态"
    );

    // ④ 重连恢复#2：active sc_manual recovery 阻塞该 turn——不自动
    //    ResumeSameTurn/不增 attempt/零 spawn（显式 fresh 授权后继续同 turn）。
    {
        let mut reopened = matrix_reopened_engine(&harness, "matrix-t18-reconnect-2");
        let actions = reopened
            .recover_human_gate_turns(false)
            .expect("t18 reconnect 2");
        assert_eq!(
            actions,
            vec![(
                turn_before.turn_id.clone(),
                HumanGateRecoveryAction::BlockedByLcRecovery {
                    recovery_id: recovery.recovery_id.clone(),
                }
            )],
            "阻塞期不得自动恢复，got {actions:?}"
        );
    }
    let turn_still_blocked = harness
        .lifecycle
        .get_human_gate_turn(&session_id, &turn_before.turn_id)
        .expect("t18 turn still blocked");
    assert_eq!(
        turn_still_blocked.attempt_no, turn_before.attempt_no,
        "阻塞期 attempt 不增"
    );
    assert_eq!(
        provider.starts.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "阻塞期零 spawn"
    );

    // ⑤ 显式 fresh：claim → dispatch（sc_manual_revision 放行臂 payload
    //    对照）→ 同 turn fresh 重入（重装同 turn prompt、resume=None 全新
    //    native）→ run_sc_manual_revision_turn 同 turn 完成回原 human gate。
    assert!(matches!(
        harness
            .lifecycle
            .claim_lc_fresh(&session_id, &recovery.recovery_id, "op-t18-manual-fresh",),
        Ok(crate::product::lifecycle_store::LcFreshClaimOutcome::Claimed { .. })
    ));
    let (dispatch_tx, _dispatch_rx) = mpsc::channel::<OutboundControl>(16);
    let dispatch_context = t18_run_context(&harness, provider.clone()).await;
    dispatch_lc_fresh_recovery_run(dispatch_context, dispatch_tx).await;
    let mut turn_after = None;
    for _ in 0..200 {
        if let Ok(turn) = harness
            .lifecycle
            .get_human_gate_turn(&session_id, &turn_before.turn_id)
        {
            if matches!(turn.status, HumanGateTurnStatus::Completed) {
                turn_after = Some(turn);
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let turn_after = turn_after.expect("显式 fresh 授权后必须同 turn 完成");
    assert_eq!(turn_after.turn_id, turn_before.turn_id, "同 turn_id");
    assert_eq!(
        turn_after.command_id, turn_before.command_id,
        "同 command_id"
    );
    assert_eq!(
        turn_after.source_hash, turn_before.source_hash,
        "同 source_hash"
    );
    assert_eq!(
        turn_after.attempt_no, turn_before.attempt_no,
        "fresh 同 turn 不加 attempt"
    );
    assert_eq!(turn_after.budget_reserved, turn_before.budget_reserved);

    // fresh input 对账：恰一次 start、resume=None、prompt 携同 turn feedback。
    let inputs = provider.inputs.lock().expect("t18 inputs").clone();
    assert_eq!(inputs.len(), 1, "fresh 恰一次 provider start");
    assert!(
        inputs[0].resume_provider_session_id.is_none(),
        "fresh 物理 run 必须全新启动（resume=None，不回捞旧 native）"
    );
    assert!(
        inputs[0].prompt.contains(&turn_before.feedback_text),
        "fresh prompt 必须重装同 turn feedback，got: {}",
        &inputs[0].prompt[..inputs[0].prompt.len().min(400)]
    );

    // 账本/预算/门不重臂；新候选经业务 compile 收口（有效 business target）。
    let after_fresh = harness.session_record_blocking();
    assert_eq!(
        serde_json::to_vec(&after_fresh.provider_start_ledger).expect("serialize ledger after"),
        ledger_before,
        "fresh 不追加 provider_start_ledger（不新 attempt/第二次 reserve）"
    );
    assert_eq!(harness.budget_remaining(), 1, "fresh 不二次扣预算");
    assert_eq!(
        after_fresh.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
        "回原 human gate（门重开、预算接续）"
    );
    assert_ne!(
        after_fresh.work_item_plan_source_revision_ref,
        after_mismatch.work_item_plan_source_revision_ref,
        "fresh 修订轮必须产出新 candidate source（业务 compile 收口）"
    );
    let recovery_after = after_fresh
        .lc_resume_recovery
        .clone()
        .expect("recovery kept after fresh run");
    assert_ne!(
        recovery_after.state,
        crate::product::models::LcResumeRecoveryState::Waiting,
        "fresh 至少离开 Waiting（Legacy 夹具无 split-run Resolved 收口；LC 生产链由 finalize_launch_after_success 收口），got {:?}",
        recovery_after.state
    );
}
