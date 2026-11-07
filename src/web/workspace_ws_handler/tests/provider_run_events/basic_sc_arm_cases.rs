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
// SC author：五类 typed 出口的 Stop 合同（brief Step 1/4）——行数门禁拆分
// 至 `basic_sc_arm_cases/sc_author_stop_cases.rs`（#[path] 链下显式路径）。
// ---------------------------------------------------------------------------
#[path = "basic_sc_arm_cases/sc_author_stop_cases.rs"]
mod sc_author_stop_cases;

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
/// 链收口。活 TempDir 夹具（root 活过整用例——stack_budget_fixture 的
/// TempDir 返回即删，durable 读写无从谈起）：先经 Author 轮建立可修订
/// 会话态，Revision 轮核定恰 2 次真实 provider start（1 次正式修订 + 1 次
/// artifact 补驱，见用例内注释依据）且 durable timeline 发生进展——brief
/// Step 5「真实 start 或该 typed 拒绝规定的零 start 证据」的前一支。
#[tokio::test]
async fn revision_run_consumes_single_receiver_and_completes() {
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
    drop(engine_rx);
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle,
        engine_tx,
        WorkspaceSession::from_record(session_record.clone()),
    )));
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseAuthorProvider {
        output: Some("case revision completed output".to_string()),
        starts: starts.clone(),
        inputs: input_tx,
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine.clone(),
        workspace_runs,
        session_record.id.clone(),
        app_paths.clone(),
        session_record.clone(),
    );
    let manager = run_context.manager.clone();
    let (outbound_tx, _outbound_rx) = mpsc::channel(64);

    // 前置 Author 轮：建立可修订会话态（助理消息 + AuthorConfirm）。
    spawn_provider_run_from_event(
        run_context.clone(),
        ProviderRunKind::Author {
            content: "case author before revision".to_string(),
        },
        None,
        outbound_tx.clone(),
    )
    .await
    .expect("spawn author run");
    case_wait_run_registration_cleared(&manager).await;
    let author_phase_starts = starts.load(Ordering::SeqCst);
    assert!(
        author_phase_starts >= 1,
        "前置 Author 轮必须真实 start（实测 {author_phase_starts}）"
    );
    // 排空 Author 轮 input：Revision 轮的会话消费单独计数。
    while input_rx.try_recv().is_ok() {}
    let nodes_before = LifecycleStore::new(app_paths.clone())
        .load_timeline_nodes(&session_record.id)
        .expect("durable timeline before revision")
        .len();
    // 建立 Story 修订上下文（用户反馈面）：无待修订反馈时 Revision 链在
    // provider 启动前早退（build_revision_input 失败 → Error 事件 +
    // finish_failed_run 收口，零 start）——本锚取「真实 start」支，先注入
    // pending 反馈再发 Revision run。
    {
        let mut engine_guard = engine.lock().await;
        engine_guard.pending_revision_context = Some("case revision feedback".to_string());
    }
    // Revision 轮：核定恰 2 次 start——1 次正式修订 start + 恰一次 artifact
    // 补驱。依据（诊断核定，与 Author 轮基线每轮 2 次同构、稳定复现）：
    // 本夹具 provider 输出为纯文本，修订产物被判「不完整 artifact」后
    // 引擎发恰一轮补驱（input[1] 头部即「上一轮已结束，但没有输出完整
    // artifact」补驱模板；input[0] 为正式修订 prompt）。单一 owned
    // receiver 语义锚定：恰这两轮会话消费（非 0、非 3+），收口后无新增。
    spawn_provider_run_from_event(run_context, ProviderRunKind::Revision, None, outbound_tx)
        .await
        .expect("spawn revision run");
    case_wait_run_registration_cleared(&manager).await;
    let revision_starts = starts.load(Ordering::SeqCst) - author_phase_starts;
    let mut revision_inputs = Vec::new();
    while let Ok(input) = input_rx.try_recv() {
        revision_inputs.push(input.prompt.clone());
    }
    assert_eq!(
        revision_starts,
        2,
        "Revision 轮必须恰为「1 次正式修订 start + 1 次 artifact 补驱」\
         （实测 {revision_starts}，总 {}／Author 基线 {author_phase_starts}）",
        starts.load(Ordering::SeqCst)
    );
    assert_eq!(
        revision_inputs.len(),
        2,
        "start 计数与 input 计数必须一一对应（实测 {} 条）",
        revision_inputs.len()
    );
    assert!(
        revision_inputs[0].contains("增量修订"),
        "input[0] 必须是正式修订 prompt（真实 start）：{}",
        revision_inputs[0].chars().take(80).collect::<String>()
    );
    assert!(
        revision_inputs[1].contains("上一轮已结束"),
        "input[1] 必须是 artifact 补驱 prompt：{}",
        revision_inputs[1].chars().take(80).collect::<String>()
    );
    // durable 进展：Revision 节点落盘使 timeline 严格增长。
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let nodes = LifecycleStore::new(app_paths.clone())
                .load_timeline_nodes(&session_record.id)
                .expect("durable timeline after revision")
                .len();
            if nodes > nodes_before {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Revision 轮必须留下 durable timeline 进展");
    // 无第二消费者/重驱：收口后不得再出现 provider input。
    case_assert_no_more_provider_inputs(
        &mut input_rx,
        "Revision 收口后不得再燃第二消费者（单一 receiver 只移动一次）",
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let _ = engine.lock().await;
    })
    .await
    .expect("Revision 收口后 engine 锁必须可重取");
}
