//! P2（任务3，brief task-3）：基础族（Author / AuthorChoiceFollowup /
//! Revision / ReviewOnly）与 SC author 臂的体积/行为门禁。
//!
//! - `raw_basic_sc_children_respect_budget`：隔离子进程内逐对象量测未装箱
//!   五臂与 SC child（`run_single_candidate_author`）的原始 future，硬断言
//!   ≤131072 B（设计 §2.2「每个具名臂 ≤128 KiB 硬上限」；量的是未 box 类型，
//!   不是 `Pin<Box<_>>` 指针）。失败格式：`raw future <symbol> <bytes>
//!   exceeds 131072`。
//! - `sc_author_stop_does_not_start_followups`：SC author 五类 typed 出口
//!   （AlreadyFinished / Superseded / Message / AdmissionWaiting /
//!   NativeSupersededWaiting）经真实 spawn 链逐分支锚定：原分支 start 数量、
//!   phase/reservation、Error/指导帧、finish 次序保持；Stop 后父层不再
//!   followups / 二次 finish（ArmExit 合同冻结，设计 §3.3）。
//! - `review_only_completed_frame_waits_for_durable_and_skips_late_turns`：
//!   ReviewOnly 两路（gateway/传统）durable Completed 之后才发本 turn
//!   Completed；turn 未完成、grant 已消费（迟到已完 turn）不重复发帧。
//! - `empty_content_author_starts_and_late_valid_run_regenerates`：空 content
//!   Author 真正 start 且有输出/durable 消息；取消在途 run 后另一个有效 run
//!   可再生成。
//! - `revision_run_consumes_single_receiver_and_completes`：Revision 臂单一
//!   owned receiver（只移动一次）经真实 spawn 链收口。
//!
//! 三守卫不可混用（r16 现场死锁根修，缺陷#10家族——设计 §3.4）：engine
//! MutexGuard 按值属 drive/当前臂；idle `provider_drive_guard` 是外层 Option
//! 的 `&mut`；`entity_run_lease` 是独立外层 binding（不传给臂）。

use super::stack_budget::{arm_drive, print_raw_baseline, stack_budget_fixture};
use super::*;

/// P2 具名臂/SC child 的未装箱体积硬预算（设计 §2.2：≤128 KiB 硬上限）。
const RAW_BASIC_SC_BUDGET: usize = 131072;

/// 量测并硬断言单对象预算；失败格式逐字对齐 brief Step 2。
fn assert_raw_budget(symbol: &'static str, bytes: usize) {
    print_raw_baseline(symbol, bytes);
    assert!(
        bytes <= RAW_BASIC_SC_BUDGET,
        "raw future {symbol} {bytes} exceeds {RAW_BASIC_SC_BUDGET}\
         （P2 基础族/SC 预算；profile/target/toolchain 见上方打印）"
    );
}

// ---------------------------------------------------------------------------
// P2 体积门禁（A 层，只构造不 poll；隔离子进程内执行——设计 §6.3）
// ---------------------------------------------------------------------------

/// raw-budget 子模式用例：P2 五臂 + SC child 硬断言全部在隔离子进程内执行
/// （父不 poll 被测 future）；常规套件直通空壳。
#[tokio::test]
async fn provider_case_child_basic_sc_raw_budget() {
    if std::env::var_os(super::small_stack_worker::PROVIDER_CASE_CHILD_MODE_ENV).is_none() {
        return;
    }
    run_basic_sc_raw_budget_measurement().await;
}

/// P2 同步父用例（brief Step 1/2）：隔离子例逐对象量未装箱五臂与 SC child。
#[test]
fn raw_basic_sc_children_respect_budget() {
    super::small_stack_worker::run_isolated_provider_case(
        "provider_case_child_basic_sc_raw_budget",
        std::time::Duration::from_secs(120),
    )
    .expect("基础族与SC author臂的未装箱体积必须全部≤131072（隔离子例证据）");
}

/// 逐对象构造未装箱 future 并硬断言（只构造不 poll，Drop 即释放夹具资源）。
async fn run_basic_sc_raw_budget_measurement() {
    let (run_context, outbound_tx, run_cancel, _command_rx, run_token) =
        stack_budget_fixture().await;
    let provider_for_run = run_context
        .provider_registry
        .get(&ProviderName::ClaudeCode)
        .expect("fixture provider");

    {
        let engine_guard = run_context.engine.lock().await;
        let mut idle_guard = Some(
            run_context
                .workspace_runs
                .begin_provider_drive(&run_context.session_id),
        );
        let (_tx, command_rx) = mpsc::channel(8);
        let author = super::run::drive_author_arm(
            arm_drive(
                engine_guard,
                command_rx,
                provider_for_run.clone(),
                &run_context,
                &outbound_tx,
                &mut idle_guard,
                &run_cancel,
                run_token,
            ),
            String::new(),
        );
        assert_raw_budget("drive_author_arm", std::mem::size_of_val(&author));
        drop(author);
    }
    {
        let engine_guard = run_context.engine.lock().await;
        let mut idle_guard = Some(
            run_context
                .workspace_runs
                .begin_provider_drive(&run_context.session_id),
        );
        let (_tx, command_rx) = mpsc::channel(8);
        let followup = super::run::drive_author_choice_followup_arm(
            arm_drive(
                engine_guard,
                command_rx,
                provider_for_run.clone(),
                &run_context,
                &outbound_tx,
                &mut idle_guard,
                &run_cancel,
                run_token,
            ),
            "stack budget baseline".to_string(),
        );
        assert_raw_budget(
            "drive_author_choice_followup_arm",
            std::mem::size_of_val(&followup),
        );
        drop(followup);
    }
    {
        let engine_guard = run_context.engine.lock().await;
        let mut idle_guard = Some(
            run_context
                .workspace_runs
                .begin_provider_drive(&run_context.session_id),
        );
        let (_tx, command_rx) = mpsc::channel(8);
        let revision = super::run::drive_revision_arm(arm_drive(
            engine_guard,
            command_rx,
            provider_for_run.clone(),
            &run_context,
            &outbound_tx,
            &mut idle_guard,
            &run_cancel,
            run_token,
        ));
        assert_raw_budget("drive_revision_arm", std::mem::size_of_val(&revision));
        drop(revision);
    }
    {
        let engine_guard = run_context.engine.lock().await;
        let mut idle_guard = Some(
            run_context
                .workspace_runs
                .begin_provider_drive(&run_context.session_id),
        );
        let (_tx, command_rx) = mpsc::channel(8);
        let review_only = super::run::drive_review_only_arm(arm_drive(
            engine_guard,
            command_rx,
            provider_for_run.clone(),
            &run_context,
            &outbound_tx,
            &mut idle_guard,
            &run_cancel,
            run_token,
        ));
        assert_raw_budget("drive_review_only_arm", std::mem::size_of_val(&review_only));
        drop(review_only);
    }
    {
        let engine_guard = run_context.engine.lock().await;
        let mut idle_guard = Some(
            run_context
                .workspace_runs
                .begin_provider_drive(&run_context.session_id),
        );
        let (_tx, command_rx) = mpsc::channel(8);
        let single_candidate = super::run::drive_single_candidate_author_arm(arm_drive(
            engine_guard,
            command_rx,
            provider_for_run.clone(),
            &run_context,
            &outbound_tx,
            &mut idle_guard,
            &run_cancel,
            run_token,
        ));
        assert_raw_budget(
            "drive_single_candidate_author_arm",
            std::mem::size_of_val(&single_candidate),
        );
        drop(single_candidate);
    }
    // —— SC child 本体（brief Step 3：先测该 child，超 131072 才在调用点 box）——
    {
        let mut engine_guard = run_context.engine.lock().await;
        let (_tx, mut sc_command_rx) = mpsc::channel(8);
        let sc_child = super::run::run_single_candidate_author(
            &mut engine_guard,
            provider_for_run.clone(),
            run_cancel.clone(),
            &mut sc_command_rx,
            super::run::SingleCandidateAuthorRecoveryEntry::Ordinary,
            &run_context,
        );
        assert_raw_budget(
            "run_single_candidate_author",
            std::mem::size_of_val(&sc_child),
        );
        drop(sc_child);
    }
}

// ---------------------------------------------------------------------------
// 行为夹具：provider / run context / 帧与收口助手
// ---------------------------------------------------------------------------

/// 记录 input 的 author provider：`Some(output)` start 后立即完成一轮输出；
/// `None` start 后立即 Failed（触发 SC child 驱动失败→AlreadyFinished）。
struct CaseAuthorProvider {
    output: Option<String>,
    starts: Arc<AtomicUsize>,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CaseAuthorProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let _ = self.inputs.send(input);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        match self.output.clone() {
            Some(output) => {
                tokio::spawn(async move {
                    let _ = event_tx
                        .send(ProviderEvent::Completed(ProviderCompletion::plain(
                            output, None,
                        )))
                        .await;
                });
            }
            None => {
                tokio::spawn(async move {
                    let _ = event_tx
                        .send(ProviderEvent::Failed {
                            message: "case planned provider failure".to_string(),
                        })
                        .await;
                });
            }
        }
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    /// gateway 启动经 `start_validated` 分发：拆出 input 后与裸 start 同路径。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("workspace runs use start")
    }
}

/// 真实入站链夹具（与 `single_candidate_context` 同构）：注册表挂
/// ClaudeCode→provider，manager 换 fixture 实例。
fn case_inbound_context(
    fixture: &ProviderRunFixture,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> (WorkspaceInboundContext, mpsc::Receiver<OutboundControl>) {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    run_context.manager = fixture.manager.clone();
    let root = fixture.root_path();
    let (outbound_tx, outbound_rx) = mpsc::channel(64);
    (
        WorkspaceInboundContext {
            app_state: WebAppState::new(
                root.clone(),
                crate::web::runtime::WebRuntime::new_fake(root),
            ),
            engine: fixture.engine.clone(),
            run_context,
            outbound_tx,
            session_id: fixture.record.id.clone(),
        },
        outbound_rx,
    )
}

/// 直驱 `spawn_provider_run_from_event` 的 run_context（不经入站校验的形态）。
fn case_event_run_context(
    fixture: &ProviderRunFixture,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> ProviderRunContext {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    run_context.manager = fixture.manager.clone();
    run_context
}

/// 等待 manager 注册被 finish_run 清空（Stop 收口的可观测面）。
async fn case_wait_run_registration_cleared(
    manager: &Arc<crate::web::workspace_session::WorkspaceSessionManager>,
) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while manager.active_run().await.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("run 注册必须被 finish_run 清空（ArmExit::Stop 收口）");
}

/// 读取下一条 type=="error" 的出站帧消息文本。
async fn case_next_error_message(outbound_rx: &mut mpsc::Receiver<OutboundControl>) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let outbound = outbound_rx.recv().await.expect("outbound control expected");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("outbound json");
            if value["type"] == "error" {
                return value["message"].as_str().unwrap_or_default().to_string();
            }
        }
    })
    .await
    .expect("error outbound expected")
}

/// 观察窗口内不得出现 error 帧（静默退场分支的负锚）。
async fn case_assert_no_error_frame(outbound_rx: &mut mpsc::Receiver<OutboundControl>, note: &str) {
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(150), outbound_rx.recv()).await,
            Ok(Some(OutboundControl::Text(json)))
                if serde_json::from_str::<serde_json::Value>(&json)
                    .map(|value| value["type"] == "error")
                    .unwrap_or(false)
        ),
        "{note}"
    );
}

/// 观察窗口内不得再出现下一条 provider input（Stop 后无 followups）。
async fn case_assert_no_more_provider_inputs(
    input_rx: &mut mpsc::UnboundedReceiver<StreamingProviderInput>,
    note: &str,
) {
    if let Ok(Some(input)) =
        tokio::time::timeout(std::time::Duration::from_millis(150), input_rx.recv()).await
    {
        let head: String = input.prompt.chars().take(160).collect();
        panic!("{note}——意外 input prompt[..]={head}");
    }
}

/// durable 会话记录重读。
fn case_durable_session(
    fixture: &ProviderRunFixture,
) -> crate::product::models::WorkspaceSessionRecord {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session")
}

/// durable 会话记录 + SG grant 原地改写（json 同源直写，t17 先例）。
fn case_rewrite_session_record(
    fixture: &ProviderRunFixture,
    mutate: impl FnOnce(&mut crate::product::models::WorkspaceSessionRecord),
) -> crate::product::models::WorkspaceSessionRecord {
    let mut record = case_durable_session(fixture);
    mutate(&mut record);
    let path = fixture
        .app_paths
        .issue_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id));
    crate::product::json_store::write_json(&path, &record).expect("rewrite durable record");
    record
}

// ---------------------------------------------------------------------------
// SC author：五类 typed 出口的 Stop 合同（brief Step 1/4）
// ---------------------------------------------------------------------------

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

/// ReviewOnly 两路帧时序用例（行数门禁拆分；#[path] 链下需显式路径）。
#[path = "basic_sc_arm_cases/review_only_cases.rs"]
mod review_only_cases;
// ---------------------------------------------------------------------------
// 空 content Author / Revision（brief Step 1/5）
// ---------------------------------------------------------------------------

/// 空 content 是正常 Author 载荷：必须真正 start、有输出与 durable 消息；
/// 取消在途 run 后另一个有效 run 可再生成。
#[tokio::test]
async fn empty_content_author_starts_and_late_valid_run_regenerates() {
    let root = tempfile::tempdir().expect("temporary workspace root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: Some(ProviderName::Codex),
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("workspace session");
    let (engine_tx, engine_rx) = mpsc::channel(64);
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle,
        engine_tx,
        WorkspaceSession::from_record(session_record.clone()),
    )));
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let author_provider = Arc::new(CaseAuthorProvider {
        output: Some("case empty content assistant reply".to_string()),
        starts: starts.clone(),
        inputs: input_tx,
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, author_provider.clone());
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine.clone(),
        workspace_runs.clone(),
        session_record.id.clone(),
        app_paths.clone(),
        session_record.clone(),
    );
    let manager = run_context.manager.clone();
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);
    let forward = spawn_engine_event_forward_task(engine_rx, outbound_tx.clone(), None);

    // —— 空 content Author：真正 start 且有输出/durable 消息 ——
    spawn_provider_run_from_event(
        run_context.clone(),
        ProviderRunKind::Author {
            content: String::new(),
        },
        None,
        outbound_tx.clone(),
    )
    .await
    .expect("spawn empty-content author run");
    case_wait_run_registration_cleared(&manager).await;
    // 空 content 是正常载荷：Author 必须真正 start（不得按内容特判归零）。
    // 本夹具基线为两轮 provider start（author 链既有行为），以 ≥1 锚定
    // 「真实启动」，绝对次数不作断言（不同轮次的 resume/fallback 属既有
    // 行为面，不在本锚收缩）。
    let first_phase_starts = starts.load(Ordering::SeqCst);
    assert!(
        first_phase_starts >= 1,
        "空 content Author 必须真正 start（实测 {first_phase_starts} 次，不得按内容特判）"
    );
    let first_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("provider input expected")
        .expect("provider input channel open");
    assert!(
        !first_input.prompt.is_empty(),
        "provider 输入必须真实构造（非空 prompt）"
    );
    // 输出：outbound 收到含输出文本的帧（事件转发链）。
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let outbound = outbound_rx.recv().await.expect("outbound control expected");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            if json.contains("case empty content assistant reply") {
                break;
            }
        }
    })
    .await
    .expect("空 content run 的 assistant 输出必须到达 outbound");
    // durable 消息：timeline 节点落盘非空。
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let nodes = LifecycleStore::new(app_paths.clone())
                .load_timeline_nodes(&session_record.id)
                .expect("load durable timeline nodes");
            if !nodes.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("空 content run 必须留下 durable timeline 消息");

    // —— 取消在途 run 后另一个有效 run 可再生成 ——
    let held_event_senders = Arc::new(Mutex::new(Vec::new()));
    let mut held_registry = ProviderRegistry::new();
    held_registry.register(
        ProviderName::ClaudeCode,
        Arc::new(PendingStartProvider {
            starts: starts.clone(),
            held_event_senders: held_event_senders.clone(),
        }),
    );
    let mut held_run_context = ProviderRunContext::test_fixture(
        Arc::new(held_registry),
        engine.clone(),
        workspace_runs.clone(),
        session_record.id.clone(),
        app_paths.clone(),
        session_record.clone(),
    );
    held_run_context.manager = manager.clone();
    spawn_provider_run_from_event(
        held_run_context,
        ProviderRunKind::Author {
            content: "second held run".to_string(),
        },
        None,
        outbound_tx.clone(),
    )
    .await
    .expect("spawn held run");
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while starts.load(Ordering::SeqCst) <= first_phase_starts {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("held run must start");
    let held_phase_starts = starts.load(Ordering::SeqCst);
    let _ = manager.abort_active_run().await;
    case_wait_run_registration_cleared(&manager).await;
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let _ = engine.lock().await;
    })
    .await
    .expect("取消收口后 engine 锁必须可重取");

    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::Author {
            content: "third valid run".to_string(),
        },
        None,
        outbound_tx.clone(),
    )
    .await
    .expect("spawn third valid run");
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while starts.load(Ordering::SeqCst) <= held_phase_starts {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("取消后另一个有效 run 必须能再生成（真实 start）");

    let _ = manager.abort_active_run().await;
    held_event_senders.lock().await.clear();
    forward.abort();
    let _ = forward.await;
}

/// Revision 臂：单一 owned receiver 只移动一次（进入当前臂），经真实 spawn
/// 链收口；不重建第二消费者。
#[tokio::test]
async fn revision_run_consumes_single_receiver_and_completes() {
    // Story 夹具（engine/manager/生命周期真实），registry 换可完成的 provider
    //（stack_budget_fixture 的 PendingStartProvider 永不完成，收口断言需要
    // 会话终结）；出站通道自持存活接收端。
    let (fixture_context, _dead_outbound_tx, _run_cancel, _command_rx, _run_token) =
        stack_budget_fixture().await;
    let starts = Arc::new(AtomicUsize::new(0));
    let (_input_tx, _input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseAuthorProvider {
        output: Some("case revision completed output".to_string()),
        starts: starts.clone(),
        inputs: _input_tx,
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture_context.engine.clone(),
        fixture_context.workspace_runs.clone(),
        fixture_context.session_id.clone(),
        fixture_context.app_paths.clone(),
        fixture_context.session_record.clone(),
    );
    let (outbound_tx, _outbound_rx) = mpsc::channel(64);
    let manager = run_context.manager.clone();
    let engine = run_context.engine.clone();
    spawn_provider_run_from_event(run_context, ProviderRunKind::Revision, None, outbound_tx)
        .await
        .expect("spawn revision run");
    case_wait_run_registration_cleared(&manager).await;
    assert!(
        starts.load(Ordering::SeqCst) <= 1,
        "Revision 臂单一 owned receiver：至多一次 provider 会话消费（实测 {}）",
        starts.load(Ordering::SeqCst)
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let _ = engine.lock().await;
    })
    .await
    .expect("Revision 收口后 engine 锁必须可重取");
}
