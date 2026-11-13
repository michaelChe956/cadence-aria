//! P4（任务5）early-stop 分支用例（brief Step 1：load/build/start/output/parse
//! 失败与 AwaitConfirmation 的原 tail 锚定）。自 `draft_batch_cases.rs` 行数
//! 门禁拆分而来（basic_sc_arm_cases 同型：#[path] 链下子模块，`use super::*`
//! 继承父作用域的夹具与观察助手）。
//!
//! 三守卫不可混用（r16 现场死锁根修，缺陷#10家族——设计 §3.4）：engine
//! MutexGuard 按值属 drive/当前臂；idle `provider_drive_guard` 是外层 Option
//! 的 `&mut`；`entity_run_lease` 是独立外层 binding（不传给臂）。

use super::*;
/// draft/batch 的 Stop 出口原样映射（brief Step 3：不借拆分补 finish）：
/// 原 drive 级 `return` 不调用 `finish_run`——注册保持在场，由接替 run 的
/// supersede（abort_active_run_from_attachment）回收。此为重构前原行为的
/// characterization 锚（对照：Continue 正常尾部才 finish_run）。
async fn case_assert_registration_still_held(
    manager: &Arc<crate::web::workspace_session::WorkspaceSessionManager>,
) {
    assert!(
        manager.active_run().await.is_some(),
        "draft/batch Stop 出口原样不调用 finish_run——注册必须保持在场（不统一补 finish）"
    );
}

/// 轮询等待 provider start 计数达标（无 Error 帧分支的完成信号）。
async fn case_wait_starts(starts: &Arc<AtomicUsize>, expected: usize, note: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while starts.load(Ordering::SeqCst) < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{note}：5s 内未达到 {expected} 次 start"));
}

/// 排空已缓冲 input 直至静默（会话驱动内部 bounded 重试链的既有行为面：
/// start 后失败可在 drive 内重试，缓冲 input 晚于 run 收口才被读取）。
/// 返回排空条数；静默即 exit（含通道关闭）。
async fn case_drain_provider_inputs_until_quiescent(
    input_rx: &mut mpsc::UnboundedReceiver<StreamingProviderInput>,
) -> usize {
    let mut drained = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(400), input_rx.recv()).await {
            Ok(Some(_)) => drained += 1,
            _ => break,
        }
    }
    drained
}

/// early-stop 各失败支的公共底座：Legacy 直连夹具 + 恢复记录（FreshRunning +
/// LegacyDraft，冻结 feedback 见各场景）。返回 (fixture, run_context,
/// outbound 通道对)。各场景自带 scripted provider / 业务种子差异。
fn case_draft_stop_fixture(
    provider: Arc<dyn StreamingProviderAdapter>,
) -> (
    ProviderRunFixture,
    ProviderRunContext,
    mpsc::Sender<OutboundControl>,
    mpsc::Receiver<OutboundControl>,
) {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::Legacy);
    let run_context = case_event_run_context(&fixture, provider);
    let (outbound_tx, outbound_rx) = mpsc::channel(64);
    (fixture, run_context, outbound_tx, outbound_rx)
}

/// early-stop 支共用的 LegacyDraft 恢复载荷。
fn case_draft_recovery_payload(
    fixture: &ProviderRunFixture,
    frozen_feedback: &str,
) -> LcRecoveryRunContext {
    LcRecoveryRunContext::LegacyDraft {
        business: case_recovery_business(fixture),
        generation_round_id: "round_0001".to_string(),
        active_outline_id: "outline_a".to_string(),
        draft_refs: Vec::new(),
        pending_feedback: Some(frozen_feedback.to_string()),
        outline_ref: None,
        candidate_ref: None,
    }
}

// ---------------------------------------------------------------------------
// 用例 2（brief Step 1）：early stop 分支的原 tail 锚定
// ---------------------------------------------------------------------------

/// P4 分支合同总用例：五个子场景各自独立夹具（load/build/start/output/parse；
/// AwaitConfirmation 已在用例 1 的两场景内锚定）。
#[tokio::test]
async fn draft_batch_early_stop_does_not_finish_twice() {
    draft_recovery_load_failure_stops_with_error_frame_zero_start().await;
    draft_input_build_failure_marks_failed_node_and_stops().await;
    draft_provider_start_failure_stops_without_error_frame().await;
    draft_structured_output_parse_failure_stops_with_error_frame().await;
    draft_candidate_parse_failure_stops_with_error_frame().await;
}

/// load 失败支：恢复记录 JSON 损坏 → `lc_fresh_recovery_record_if_running`
/// Err → mark→drop engine→Error 帧→Stop；零 provider start、不消耗 RetryOnce
/// 预算（typed 拒面零 start 不改预算）；注册按原样保持（不补 finish_run）。
async fn draft_recovery_load_failure_stops_with_error_frame_zero_start() {
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseDraftBatchProvider {
        rounds: std::sync::Mutex::new(VecDeque::new()),
        starts: starts.clone(),
        inputs: input_tx,
        fail_start: false,
    });
    let (fixture, run_context, outbound_tx, mut outbound_rx) = case_draft_stop_fixture(provider);
    let outline = case_outline_item(
        "outline_a",
        "wi_a",
        "P5-LA",
        "src/a.rs",
        None,
        &fixture.story_id,
        &fixture.design_id,
    );
    {
        let mut engine = fixture.engine.lock().await;
        engine.update_artifact(case_outline_artifact(
            vec![outline.clone()],
            &fixture.story_id,
            &fixture.design_id,
        )).await.expect("fixture artifact write");
        engine.pending_revision_context =
            Some("P5内存残留反馈（不得进入draft fresh输入）".to_string());
    }
    case_seed_serial_business(&fixture, &outline);
    case_seed_fresh_recovery(
        &fixture,
        "recovery_p5_load",
        case_draft_recovery_payload(&fixture, "P5load冻结反馈"),
    );
    // 损坏会话 JSON：准入用内存 record 快照（不受影响），臂内
    // lc_fresh_recovery_record_if_running 的 durable 读 fail-closed 上浮。
    let session_path = fixture
        .app_paths
        .issue_root("project_0001", "issue_0001")
        .join("workspace-sessions")
        .join(format!("{}.json", fixture.record.id));
    std::fs::write(&session_path, "{ not-valid-json").expect("corrupt session record json");
    let manager = run_context.manager.clone();
    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanDraft {
            feedback: Some("P5load冻结反馈".to_string()),
        },
        None,
        outbound_tx,
    )
    .await
    .expect("spawn draft run (load failure case)");
    // 完成信号：Error 帧先于 Stop 发出（engine 已 drop）。
    let message = case_next_error_message(&mut outbound_rx).await;
    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "load 失败支必须零 provider start"
    );
    assert!(
        message.contains("load workspace session for fresh recovery failed"),
        "load 失败支必须回显 durable 读失败（实测 {message}）"
    );
    {
        let engine = fixture.engine.lock().await;
        assert!(
            engine.work_item_draft_repair_states.is_empty(),
            "零 start 分支不得消耗 RetryOnce 预算"
        );
        assert_eq!(
            engine.pending_revision_context.as_deref(),
            Some("P5内存残留反馈（不得进入draft fresh输入）")
        );
    }
    case_assert_registration_still_held(&manager).await;
    case_assert_engine_lock_reacquirable(&fixture, "load 失败 Stop 后").await;
    case_assert_no_more_provider_inputs(&mut input_rx, "load 失败 Stop 后不得再 start").await;
}

/// build 失败支：outline artifact 缺席 → 输入构建 fail-closed →
/// `finish_active_run_with_failed_node`（节点 Failed）→drop engine→Error 帧→
/// Stop；零 start。
async fn draft_input_build_failure_marks_failed_node_and_stops() {
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseDraftBatchProvider {
        rounds: std::sync::Mutex::new(VecDeque::new()),
        starts: starts.clone(),
        inputs: input_tx,
        fail_start: false,
    });
    let (fixture, run_context, outbound_tx, mut outbound_rx) = case_draft_stop_fixture(provider);
    // 只 seed 业务 index（节点重建可过），不注入 outline artifact——
    // build_current_work_item_draft_streaming_input_explicit_fresh fail-closed。
    let outline = case_outline_item(
        "outline_a",
        "wi_a",
        "P5-BU",
        "src/a.rs",
        None,
        &fixture.story_id,
        &fixture.design_id,
    );
    case_seed_serial_business(&fixture, &outline);
    case_seed_fresh_recovery(
        &fixture,
        "recovery_p5_build",
        case_draft_recovery_payload(&fixture, "P5build冻结反馈"),
    );
    let manager = run_context.manager.clone();
    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanDraft {
            feedback: Some("P5build冻结反馈".to_string()),
        },
        None,
        outbound_tx,
    )
    .await
    .expect("spawn draft run (build failure case)");
    // 完成信号：Error 帧先于 Stop 发出（engine 已 drop）。
    let message = case_next_error_message(&mut outbound_rx).await;
    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "build 失败支必须零 provider start"
    );
    assert!(
        message.contains("latest WorkItemPlan Outline artifact is unavailable"),
        "build 失败支必须回显输入构建失败（实测 {message}）"
    );
    {
        let engine = fixture.engine.lock().await;
        let draft_node = engine
            .timeline_nodes
            .iter()
            .rev()
            .find(|node| node.node_type == TimelineNodeType::WorkItemDraftRun)
            .expect("draft run node rebuilt before build failure");
        assert_eq!(
            draft_node.status,
            crate::web::workspace_ws_types::TimelineNodeStatus::Failed,
            "build 失败支原 tail 是 finish_active_run_with_failed_node（非裸 mark）"
        );
    }
    case_assert_registration_still_held(&manager).await;
    case_assert_engine_lock_reacquirable(&fixture, "build 失败 Stop 后").await;
    case_assert_no_more_provider_inputs(&mut input_rx, "build 失败 Stop 后不得再 start").await;
}

/// provider start/输出驱动失败支：start Err → 驱动失败 → fail_plan_split_run
/// → mark_active_run_finished → return——**不 drop engine 显式语句、不补
/// Error 帧、不 Failed 节点**（原 tail 的差异不得被统一补帧）。
async fn draft_provider_start_failure_stops_without_error_frame() {
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseDraftBatchProvider {
        rounds: std::sync::Mutex::new(VecDeque::new()),
        starts: starts.clone(),
        inputs: input_tx,
        fail_start: true,
    });
    let (fixture, run_context, outbound_tx, mut outbound_rx) = case_draft_stop_fixture(provider);
    let outline = case_outline_item(
        "outline_a",
        "wi_a",
        "P5-ST",
        "src/a.rs",
        None,
        &fixture.story_id,
        &fixture.design_id,
    );
    {
        let mut engine = fixture.engine.lock().await;
        engine.update_artifact(case_outline_artifact(
            vec![outline.clone()],
            &fixture.story_id,
            &fixture.design_id,
        )).await.expect("fixture artifact write");
    }
    case_seed_serial_business(&fixture, &outline);
    case_seed_fresh_recovery(
        &fixture,
        "recovery_p5_start",
        case_draft_recovery_payload(&fixture, "P5start冻结反馈"),
    );
    let manager = run_context.manager.clone();
    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanDraft {
            feedback: Some("P5start冻结反馈".to_string()),
        },
        None,
        outbound_tx,
    )
    .await
    .expect("spawn draft run (start failure case)");
    // 完成信号：本支无 Error 帧——先轮询恰一次真实 start，再以 engine
    // 释放（drive 返回）为准；注册按原样保持（不补 finish_run）。
    case_wait_starts(&starts, 1, "start 失败支必须恰一次真实 start 后失败").await;
    case_assert_engine_lock_reacquirable(&fixture, "start 失败 Stop 后").await;
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "start 失败支必须恰一次真实 start 后失败"
    );
    case_assert_no_error_frame(
        &mut outbound_rx,
        "start/驱动失败支原 tail 不补 Error 帧（不得统一补帧）",
    )
    .await;
    {
        let engine = fixture.engine.lock().await;
        let draft_node = engine
            .timeline_nodes
            .iter()
            .rev()
            .find(|node| node.node_type == TimelineNodeType::WorkItemDraftRun)
            .expect("draft run node rebuilt before start failure");
        assert_eq!(
            draft_node.status,
            crate::web::workspace_ws_types::TimelineNodeStatus::Failed,
            "节点 Failed 由会话驱动内部收口（finish_failed_run）；本支臂尾差异的\
             判据是不补 Error 帧 + 不显式 drop engine + 注册保持"
        );
    }
    case_assert_registration_still_held(&manager).await;
    let drained = case_drain_provider_inputs_until_quiescent(&mut input_rx).await;
    assert_eq!(
        drained,
        starts.load(Ordering::SeqCst),
        "start 失败支排空后零新增 start（每次 start 恰一条缓冲 input）"
    );
}

/// 结构化输出解析失败支：无 sentinel 纯文本 → fail_plan_split_run→mark→drop
/// engine→Error 帧→Stop。
async fn draft_structured_output_parse_failure_stops_with_error_frame() {
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseDraftBatchProvider {
        rounds: std::sync::Mutex::new(VecDeque::from(vec![CaseDraftRound::PlainText])),
        starts: starts.clone(),
        inputs: input_tx,
        fail_start: false,
    });
    let (fixture, run_context, outbound_tx, mut outbound_rx) = case_draft_stop_fixture(provider);
    let outline = case_outline_item(
        "outline_a",
        "wi_a",
        "P5-SP",
        "src/a.rs",
        None,
        &fixture.story_id,
        &fixture.design_id,
    );
    {
        let mut engine = fixture.engine.lock().await;
        engine.update_artifact(case_outline_artifact(
            vec![outline.clone()],
            &fixture.story_id,
            &fixture.design_id,
        )).await.expect("fixture artifact write");
    }
    case_seed_serial_business(&fixture, &outline);
    case_seed_fresh_recovery(
        &fixture,
        "recovery_p5_parse",
        case_draft_recovery_payload(&fixture, "P5parse冻结反馈"),
    );
    let manager = run_context.manager.clone();
    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanDraft {
            feedback: Some("P5parse冻结反馈".to_string()),
        },
        None,
        outbound_tx,
    )
    .await
    .expect("spawn draft run (structured parse failure case)");
    // 完成信号：Error 帧先于 Stop 发出（engine 已 drop）。
    let message = case_next_error_message(&mut outbound_rx).await;
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert!(
        message.contains("work item draft generate failed"),
        "结构化解析失败支必须回显 generate failed（实测 {message}）"
    );
    case_assert_registration_still_held(&manager).await;
    case_assert_engine_lock_reacquirable(&fixture, "结构化解析失败 Stop 后").await;
    let drained = case_drain_provider_inputs_until_quiescent(&mut input_rx).await;
    assert_eq!(drained, 1, "结构化解析失败支恰一次 start，静默后零新增");
}

/// draft 候选解析失败支：sentinel 在场但载荷非法（drafts 数组）→ close_plan_
/// split_run（structured 可解析 → complete）后 parse_work_item_draft_output
/// Err →mark→drop engine→Error 帧→Stop。
async fn draft_candidate_parse_failure_stops_with_error_frame() {
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseDraftBatchProvider {
        rounds: std::sync::Mutex::new(VecDeque::from(vec![CaseDraftRound::InvalidDraftPayload])),
        starts: starts.clone(),
        inputs: input_tx,
        fail_start: false,
    });
    let (fixture, run_context, outbound_tx, mut outbound_rx) = case_draft_stop_fixture(provider);
    let outline = case_outline_item(
        "outline_a",
        "wi_a",
        "P5-DP",
        "src/a.rs",
        None,
        &fixture.story_id,
        &fixture.design_id,
    );
    {
        let mut engine = fixture.engine.lock().await;
        engine.update_artifact(case_outline_artifact(
            vec![outline.clone()],
            &fixture.story_id,
            &fixture.design_id,
        )).await.expect("fixture artifact write");
    }
    case_seed_serial_business(&fixture, &outline);
    case_seed_fresh_recovery(
        &fixture,
        "recovery_p5_draft_parse",
        case_draft_recovery_payload(&fixture, "P5draftparse冻结反馈"),
    );
    let manager = run_context.manager.clone();
    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanDraft {
            feedback: Some("P5draftparse冻结反馈".to_string()),
        },
        None,
        outbound_tx,
    )
    .await
    .expect("spawn draft run (draft parse failure case)");
    // 完成信号：Error 帧先于 Stop 发出（engine 已 drop）。
    let message = case_next_error_message(&mut outbound_rx).await;
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert!(
        message.contains("work item draft parse failed"),
        "draft 候选解析失败支必须回显 parse failed（实测 {message}）"
    );
    case_assert_registration_still_held(&manager).await;
    case_assert_engine_lock_reacquirable(&fixture, "draft 解析失败 Stop 后").await;
    let drained = case_drain_provider_inputs_until_quiescent(&mut input_rx).await;
    assert_eq!(drained, 1, "draft 解析失败支恰一次 start，静默后零新增");
}
// @@STOP_SECTION@@
