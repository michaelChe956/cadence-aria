//! P2（任务3，brief task-3 Step 1/4）：SC author 臂五类 typed 出口的 Stop
//! 合同用例——自 `basic_sc_arm_cases.rs` 拆出（行数门禁 1000~1200 继续拆）。
//!
//! 覆盖 AlreadyFinished / Superseded / Message / AdmissionWaiting /
//! NativeSupersededWaiting：原分支 start 数量、phase/reservation、Error 帧、
//! finish 次序保持；Stop 后父层不再 followups / 二次 finish（ArmExit 合同）。

use super::*;

/// 覆盖 AlreadyFinished / Superseded / Message / AdmissionWaiting /
/// NativeSupersededWaiting：原分支 start 数量、phase/reservation、Error 帧、
/// finish 次序保持；Stop 后父层不再 followups / 二次 finish。
#[tokio::test]
async fn sc_author_stop_does_not_start_followups() {
    sc_superseded_late_run_stops_silently().await;
    sc_terminal_failed_message_frame_after_finish().await;
    sc_drive_failure_already_finished_stops_silently().await;
    sc_admission_waiting_keeps_prepare_face_and_reservation().await;
    sc_native_superseded_waiting_keeps_key_and_phase().await;
}

/// Superseded（败者让位）：零 start、零 Error、durable phase/ledger 原样、
/// run 注册被 finish_run 清空、engine 锁可重取。
async fn sc_superseded_late_run_stops_silently() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    // 模拟健康胜者：reserve 已领取首轮键（phase→Generate，ledger [:0]）。
    {
        let mut engine = fixture.engine.lock().await;
        let claimed = engine
            .reserve_single_candidate_author_start()
            .expect("healthy winner claims the first provider start key");
        assert!(claimed);
    }
    let ledger_before = serde_json::to_vec(&case_durable_session(&fixture).provider_start_ledger)
        .expect("serialize ledger before");
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseAuthorProvider {
        output: None,
        starts: starts.clone(),
        inputs: input_tx,
    });
    let (context, mut outbound_rx) = case_inbound_context(&fixture, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    case_wait_run_registration_cleared(&fixture.manager).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "Superseded 败者不得启动 provider（原分支 start 数量保持 0）"
    );
    case_assert_no_error_frame(&mut outbound_rx, "Superseded 静默退场不得广播 Error").await;
    case_assert_no_more_provider_inputs(
        &mut input_rx,
        "Superseded Stop 后父层不得再燃 followups provider",
    )
    .await;
    let after = case_durable_session(&fixture);
    assert_eq!(
        after.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Generate),
        "Superseded 不得改写 durable phase"
    );
    assert_eq!(
        serde_json::to_vec(&after.provider_start_ledger).expect("serialize ledger"),
        ledger_before,
        "Superseded 不得追加/改写 provider_start_ledger（reservation 原样）"
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let _ = fixture.engine.lock().await;
    })
    .await
    .expect("Superseded Stop 后 engine 锁必须可重取（guard 按值释放）");
}

/// Message（终态 Failed 入口）：Error 帧到达时 run 注册已被 finish_run 清空
/// （finish 次序保持：failed node→drop engine→idle take→finish→Error）。
async fn sc_terminal_failed_message_frame_after_finish() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    {
        let mut engine = fixture.engine.lock().await;
        engine.persist_single_candidate_terminal_phase(
            crate::product::models::SingleCandidatePhase::Failed,
        );
    }
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseAuthorProvider {
        output: None,
        starts: starts.clone(),
        inputs: input_tx,
    });
    let run_context = case_event_run_context(&fixture, provider);
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);
    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
        None,
        outbound_tx,
    )
    .await
    .expect("spawn the terminal-failed start attempt");

    let message = case_next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("已终态失败") && message.contains("请显式重新开始生成"),
        "Message 分支必须保持可见恢复入口错误：{message}"
    );
    assert!(
        fixture.manager.active_run().await.is_none(),
        "Error 帧到达时 finish_run 必须已完成（arm 内 finish 先于 Error 帧）"
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "Message（终态 Failed）分支不得启动 provider"
    );
    case_assert_no_more_provider_inputs(
        &mut input_rx,
        "Message Stop 后父层不得再燃 followups provider",
    )
    .await;
    assert_eq!(
        case_durable_session(&fixture).single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Failed),
        "Message 分支不得改写 durable 终态 Failed"
    );
}

/// AlreadyFinished（驱动失败收口）：恰一次真实 start、臂内静默 Stop（不新发
/// Error、不改写 durable——Failed 已由 child 落盘）、无 followups。
async fn sc_drive_failure_already_finished_stops_silently() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseAuthorProvider {
        output: None,
        starts: starts.clone(),
        inputs: input_tx,
    });
    let (context, mut outbound_rx) = case_inbound_context(&fixture, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    case_wait_run_registration_cleared(&fixture.manager).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "AlreadyFinished 分支原形态：恰一次真实 start 后驱动失败"
    );
    // 消费恰一条真实 input（full prompt 非空），随后窗口内不得再有第二条
    //（Stop 后无 reredrive/followups）。
    let first_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("AlreadyFinished 必须恰有一条 provider input")
        .expect("provider input channel open");
    assert!(
        !first_input.prompt.is_empty(),
        "AlreadyFinished 的唯一一次 start 必须携带真实完整 prompt"
    );
    case_assert_no_error_frame(
        &mut outbound_rx,
        "AlreadyFinished 臂内收口不得新发 Error（durable Failed 已由 child 落盘）",
    )
    .await;
    case_assert_no_more_provider_inputs(
        &mut input_rx,
        "AlreadyFinished Stop 后父层不得再燃 followups/reredrive provider",
    )
    .await;
    assert_eq!(
        case_durable_session(&fixture).single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Failed),
        "AlreadyFinished 的 durable Failed 由 child 落盘（臂不重复改写）"
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let _ = fixture.engine.lock().await;
    })
    .await
    .expect("AlreadyFinished Stop 后 engine 锁必须可重取");
}

/// AdmissionWaiting（LC 成员规则缺失）：waiting/Prepare 面（不 generic Failed）、
/// reservation 不重臂（ledger 恰一条）、Error 帧在 finish 之后、零 start。
async fn sc_admission_waiting_keeps_prepare_face_and_reservation() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseAuthorProvider {
        output: Some(single_candidate_markdown("story_0001", "design_0001")),
        starts: Arc::new(AtomicUsize::new(0)),
        inputs: input_tx,
    });
    // 成员缺 .claude/rules/language.md（write_member_language_rules=false）。
    let fixture = ProviderRunFixture::new_logical(false, provider.clone());
    let (context, mut outbound_rx) = case_inbound_context(&fixture, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let message = case_next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("member_language_rules_missing"),
        "AdmissionWaiting 消息必须携带预检判别码，got: {message}"
    );
    assert!(
        fixture.manager.active_run().await.is_none(),
        "AdmissionWaiting 的 Error 帧到达时 finish_run 必须已完成"
    );
    let durable = case_durable_session(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Prepare),
        "AdmissionWaiting 必须保持 waiting/Prepare 面（不 generic Failed）"
    );
    assert_ne!(
        durable.status,
        crate::product::models::WorkspaceSessionStatus::Failed
    );
    assert_eq!(
        durable.provider_start_ledger.len(),
        1,
        "AdmissionWaiting 保留原 reservation（不重臂、不追加 ledger）"
    );
    case_assert_no_more_provider_inputs(
        &mut input_rx,
        "AdmissionWaiting 必须 reject before provider startup",
    )
    .await;
    case_assert_no_more_provider_inputs(
        &mut input_rx,
        "AdmissionWaiting Stop 后父层不得再燃 followups provider",
    )
    .await;
}

/// NativeSupersededWaiting（native mismatch typed 等待）：判别码/显式出口上浮、
/// phase=Generate 与 ledger 原样保留（不 reserve 第二次、不 rearm）、零 start。
async fn sc_native_superseded_waiting_keeps_key_and_phase() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let reservation_key = format!("single_candidate_author:{}:0", fixture.record.id);
    // 修订轮形态：phase=Generate + ledger [reservation_key] + Author conversation
    // 指向旧 native 且已被 tombstone（anchor_t17 同构种子）。
    let seeded = case_rewrite_session_record(&fixture, |record| {
        record.status = crate::product::models::WorkspaceSessionStatus::Running;
        record.single_candidate_phase =
            Some(crate::product::models::SingleCandidatePhase::Generate);
        record.provider_start_ledger = vec![
            crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
                provider_start_idempotency_key: reservation_key.clone(),
                started: true,
                provider: None,
                started_at: None,
                provider_attempt_key: None,
            },
        ];
        record.provider_conversations = vec![crate::product::models::ProviderConversationRef {
            role: crate::product::models::ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "case-old-native".to_string(),
            updated_at: "2026-11-06T00:00:00Z".to_string(),
            last_node_id: Some("timeline_node_001".to_string()),
        }];
        record.superseded_native_sessions =
            vec![crate::product::models::SupersededNativeSessionRecord {
                role: crate::product::models::ProviderConversationRole::Author,
                provider: ProviderName::ClaudeCode,
                native_session_id: "case-old-native".to_string(),
                reason_code: "resume_fingerprint_unavailable".to_string(),
                recovery_id: "seed".to_string(),
                superseded_at: "2026-11-06T00:00:00Z".to_string(),
            }];
    });
    // 引擎内存镜像同步（保留 repository_path）。
    {
        let mut engine = fixture.engine.lock().await;
        let repository_path = engine.session.repository_path.clone();
        engine.session = crate::product::workspace_engine::WorkspaceSession::from_record(seeded);
        engine.session.repository_path = repository_path;
    }
    let ledger_before = serde_json::to_vec(&case_durable_session(&fixture).provider_start_ledger)
        .expect("serialize ledger before");
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseAuthorProvider {
        output: None,
        starts: starts.clone(),
        inputs: input_tx,
    });
    let (context, mut outbound_rx) = case_inbound_context(&fixture, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let message = case_next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("resume_fingerprint_unavailable") && message.contains("显式"),
        "NativeSupersededWaiting 必须携带稳定判别码与显式恢复出口，got: {message}"
    );
    assert!(
        fixture.manager.active_run().await.is_none(),
        "NativeSupersededWaiting 的 Error 帧到达时 finish_run 必须已完成"
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "NativeSupersededWaiting 必须 precheck 先于 provider 启动（零 start）"
    );
    let after = case_durable_session(&fixture);
    assert_eq!(
        after.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Generate),
        "等待保留原 phase（不 generic Failed、不回 Prepare 重臂）"
    );
    assert_ne!(
        after.status,
        crate::product::models::WorkspaceSessionStatus::Failed
    );
    assert_eq!(
        serde_json::to_vec(&after.provider_start_ledger).expect("serialize ledger"),
        ledger_before,
        "等待不得追加 provider_start_ledger（不 reserve 第二次）"
    );
    let recovery = after
        .lc_resume_recovery
        .clone()
        .expect("mismatch must persist a waiting recovery record");
    assert_eq!(
        recovery.state,
        crate::product::models::LcResumeRecoveryState::Waiting
    );
    assert_eq!(recovery.reason_code, "resume_fingerprint_unavailable");
    case_assert_no_more_provider_inputs(
        &mut input_rx,
        "NativeSupersededWaiting Stop 后父层不得再燃 followups provider",
    )
    .await;
}
