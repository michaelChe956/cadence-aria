//! S0（任务1）：provider run 体积基线（A 层门禁，只构造不 poll——设计 §6.1）。
//!
//! 机械提取前 `provider_run_task_body` 不可命名 ⇒ 首红为 E0425/E0432（缺
//! 构造符号，不是栈崩溃）；提取后本用例直接构造未 box 的 task body /
//! 基础臂 / SC author 臂原始 future，`size_of_val` 逐对象量测。131072 B
//! （128 KiB）是具名 body 的 S0 工程门禁（设计 §2.2）；注册前入口与取消
//! settle helper 只记录真实大小，不设硬断言（阈值随各族迁移后冻结）。
//!
//! String/Vec 的堆内容不计入 future 体积（设计 §2.3）；本用例不 poll 任何
//! 被测 future，构造即量测，Drop 即释放夹具资源。

use super::*;

/// S0 具名 task body 的未装箱体积门禁（设计 §2.2：≤128 KiB 硬上限）。
const RAW_TASK_BODY_BUDGET: usize = 131072;

/// （P2/任务3 起 `pub(super)`：基础族+SC 行为/预算用例 `basic_sc_arm_cases`
/// 复用同一量测面——仍限 provider_run_events 测试树，不做公共 API。）
pub(super) fn print_raw_baseline(object: &str, bytes: usize) {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let target = option_env!("TARGET")
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS));
    let toolchain = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "rustc(<version unavailable>)".to_string());
    println!(
        "[stack-budget] {object}: {bytes} B | profile={profile} | target={target} | toolchain={toolchain}"
    );
}

/// 与 r16 `supersede_reclaims_engine_lock_from_run_hanging_outside_select`
/// 同构的最小夹具：真实 LifecycleStore/engine/registry/run_context（量测
/// 不依赖 provider 行为，PendingStartProvider 仅占位注册表项）。
pub(super) async fn stack_budget_fixture() -> (
    ProviderRunContext,
    mpsc::Sender<OutboundControl>,
    CancellationToken,
    mpsc::Receiver<ProviderCommand>,
    u64,
) {
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
    let (_engine_tx, engine_rx) = mpsc::channel(8);
    drop(engine_rx);
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle,
        _engine_tx,
        WorkspaceSession::from_record(session_record.clone()),
    )));
    let starts = Arc::new(AtomicUsize::new(0));
    let held_event_senders = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(PendingStartProvider {
            starts: starts.clone(),
            held_event_senders: held_event_senders.clone(),
        }),
    );
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine.clone(),
        workspace_runs.clone(),
        session_record.id.clone(),
        app_paths,
        session_record.clone(),
    );
    let (outbound_tx, _outbound_rx) = mpsc::channel::<OutboundControl>(8);
    let (_command_tx, command_rx) = mpsc::channel(8);
    let run_cancel = CancellationToken::new();
    (run_context, outbound_tx, run_cancel, command_rx, 1)
}

/// raw-budget 子模式用例：全部量测+断言在隔离子进程内执行（父不 poll
/// 被测 future——设计 §6.3）；常规套件直通空壳（由父用例验证分类）。
#[tokio::test]
async fn provider_case_child_raw_budget() {
    if std::env::var_os(super::small_stack_worker::PROVIDER_CASE_CHILD_MODE_ENV).is_none() {
        return;
    }
    run_raw_budget_measurement().await;
}

/// S2 body-probe 子模式用例：wrapper 首次 poll 构造语义探针（子进程隔离
/// ——进程内其它用例的并发 provider spawn 不会污染构造计数）。
#[tokio::test]
async fn provider_case_child_body_probe() {
    if std::env::var_os(super::small_stack_worker::PROVIDER_CASE_CHILD_MODE_ENV).is_none() {
        return;
    }
    run_body_probe_measurement().await;
}

/// S2 同步父用例（不依赖进程内计数）：隔离子例证明「spawn 调用侧不构造
/// task body、body 构造发生在 wrapper 首次 poll 内」（设计 §3.1.2）。
#[test]
fn body_is_not_constructed_before_wrapper_first_poll() {
    super::small_stack_worker::run_isolated_provider_case(
        "provider_case_child_body_probe",
        std::time::Duration::from_secs(120),
    )
    .expect("body 构造必须推迟到 wrapper 首次 poll（隔离子例证据）");
}

/// S2 门禁：小外壳未装箱体积 ≤32 KiB（设计 §2.2）。直接量
/// `provider_run_task` 的未 box 返回值——`Pin<Box<_>>` 的指针大小不是本
/// 门禁对象（不测 box 后的指针形成假门禁）。
#[tokio::test]
async fn raw_wrapper_is_at_most_32768_bytes() {
    let (run_context, outbound_tx, run_cancel, command_rx, run_token) =
        stack_budget_fixture().await;
    let provider_for_run = run_context
        .provider_registry
        .get(&ProviderName::ClaudeCode)
        .expect("fixture provider");
    let entity_run_lease =
        crate::product::provider_workspace_runner::acquire_run_lease_for_session(
            &run_context.app_paths,
            &run_context.session_record,
        )
        .expect("fixture run lease");
    let provider_drive_guard = run_context
        .workspace_runs
        .begin_provider_drive(&run_context.session_id);
    let wrapper = provider_run_task(
        ProviderRunKind::Author {
            content: "stack budget baseline".to_string(),
        },
        run_context.clone(),
        run_context.engine.clone(),
        run_context.manager.clone(),
        run_context.provider_registry.clone(),
        provider_for_run.clone(),
        outbound_tx.clone(),
        outbound_tx.clone(),
        run_context.session_id.clone(),
        "run-1".to_string(),
        run_token,
        run_cancel.clone(),
        command_rx,
        provider_drive_guard,
        entity_run_lease,
        None,
        false,
        None,
    );
    let wrapper_bytes = std::mem::size_of_val(&wrapper);
    print_raw_baseline("provider_run_task(wrapper)", wrapper_bytes);
    drop(wrapper);
    assert!(
        wrapper_bytes <= 32768,
        "raw future provider_run_task(外壳) = {wrapper_bytes} B exceeds 32768 B（S2 外壳门禁，设计 §2.2；profile/target/toolchain 见上方打印）"
    );
}

/// S2 探针子模式执行体：真实 author 链夹具（engine 事件通道存活，与 r16
/// 用例同构），构造 wrapper→断言计数 0→手动首 poll→断言计数 1→中途 drop
/// 验证三守卫随任务体释放。
async fn run_body_probe_measurement() {
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
    let held_event_senders = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(PendingStartProvider {
            starts: starts.clone(),
            held_event_senders: held_event_senders.clone(),
        }),
    );
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine.clone(),
        workspace_runs.clone(),
        session_record.id.clone(),
        app_paths,
        session_record.clone(),
    );
    let (outbound_tx, _outbound_rx) = mpsc::channel::<OutboundControl>(64);
    let forward =
        spawn_engine_event_forward_task(engine_rx, outbound_tx.clone(), Some(run_context.clone()));
    let provider_for_run = run_context
        .provider_registry
        .get(&ProviderName::ClaudeCode)
        .expect("fixture provider");
    let entity_run_lease =
        crate::product::provider_workspace_runner::acquire_run_lease_for_session(
            &run_context.app_paths,
            &run_context.session_record,
        )
        .expect("fixture run lease");
    let provider_drive_guard = run_context
        .workspace_runs
        .begin_provider_drive(&run_context.session_id);
    let run_cancel = CancellationToken::new();
    let (_command_tx, command_rx) = mpsc::channel(8);

    PROVIDER_RUN_TASK_BODY_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    let mut wrapper = Box::pin(provider_run_task(
        ProviderRunKind::Author {
            content: "body probe first poll".to_string(),
        },
        run_context.clone(),
        run_context.engine.clone(),
        run_context.manager.clone(),
        run_context.provider_registry.clone(),
        provider_for_run.clone(),
        outbound_tx.clone(),
        outbound_tx.clone(),
        run_context.session_id.clone(),
        "run-probe".to_string(),
        1,
        run_cancel.clone(),
        command_rx,
        provider_drive_guard,
        entity_run_lease,
        None,
        false,
        None,
    ));
    assert_eq!(
        PROVIDER_RUN_TASK_BODY_CONSTRUCTIONS.load(Ordering::SeqCst),
        0,
        "构造 provider_run_task 返回值（spawn 调用侧、未 poll）不得构造 task body"
    );
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    let _ = wrapper.as_mut().poll(&mut cx);
    assert_eq!(
        PROVIDER_RUN_TASK_BODY_CONSTRUCTIONS.load(Ordering::SeqCst),
        1,
        "task body 必须在 wrapper 首次 poll 内构造（计数不得为 0 或 >1）"
    );
    // 中途 drop：engine guard/idle/实体 lease 随任务体释放（r16 合同）——
    // engine 锁必须立即可取。
    drop(wrapper);
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let _ = engine.lock().await;
    })
    .await
    .expect("engine lock must be re-acquirable after dropping the wrapper mid-run");

    held_event_senders.lock().await.clear();
    forward.abort();
    let _ = forward.await;
}

/// S0 characterization 同步父用例（不 poll 被测 future）：隔离执行专用
/// raw-budget 子模式，验证其真实测量输出与预算失败分类并保存 S0 红值。
/// 这不是「预算已通过」——任务 9 启用最终 raw 硬 assert 时移除本通过
/// 逻辑，不留 ignore/should_panic。
#[test]
fn raw_task_body_baseline() {
    let result = super::small_stack_worker::run_isolated_provider_case(
        "provider_case_child_raw_budget",
        std::time::Duration::from_secs(120),
    );
    match result {
        Ok(()) => {
            // S2 后 task_body 只持外壳捕获+settle 装配，≤128 KiB 属预期绿
            //（drive 整体体积改由 provider_run_drive_body 基线单独记录；
            // 未拆族臂的超线治理属任务 3–8，不在此宣告根修完成）。
            println!(
                "[stack-budget] raw-budget child green: task_body within \
                 {RAW_TASK_BODY_BUDGET} B after S2 shell/boxed-drive split"
            );
        }
        Err(report) => {
            assert!(
                report.contains("isolated child failure must be rejected")
                    && report.contains("exceeds 131072"),
                "raw-budget child failed without budget failure classification: {report}"
            );
            println!("[stack-budget] S0 红值经隔离子例留存: {report}");
        }
    }
}

async fn run_raw_budget_measurement() {
    let (run_context, outbound_tx, run_cancel, command_rx, run_token) =
        stack_budget_fixture().await;
    let provider_for_run = run_context
        .provider_registry
        .get(&ProviderName::ClaudeCode)
        .expect("fixture provider");
    let entity_run_lease =
        crate::product::provider_workspace_runner::acquire_run_lease_for_session(
            &run_context.app_paths,
            &run_context.session_record,
        )
        .expect("fixture run lease");
    let provider_drive_guard = run_context
        .workspace_runs
        .begin_provider_drive(&run_context.session_id);

    // —— 未 box task body：只构造不 poll ——
    let task_body = provider_run_task_body(
        ProviderRunKind::Author {
            content: "stack budget baseline".to_string(),
        },
        run_context.clone(),
        run_context.engine.clone(),
        run_context.manager.clone(),
        run_context.provider_registry.clone(),
        provider_for_run.clone(),
        outbound_tx.clone(),
        outbound_tx.clone(),
        run_context.session_id.clone(),
        "run-1".to_string(),
        run_token,
        run_cancel.clone(),
        command_rx,
        provider_drive_guard,
        entity_run_lease,
        None,
        false,
        None,
    );
    let task_body_bytes = std::mem::size_of_val(&task_body);
    print_raw_baseline("provider_run_task_body", task_body_bytes);

    // —— S2 具名 drive body：未装箱整体量测（boxed 前的真实基线；尚含
    //    未拆族臂 ⇒ 预期超 128 KiB 硬线，任务 3–8 逐族拆分后才达标——
    //    只记录，硬断言随任务 9 冻结）——
    {
        let engine_guard = run_context.engine.lock().await;
        let mut idle_guard = Some(
            run_context
                .workspace_runs
                .begin_provider_drive(&run_context.session_id),
        );
        let (_drive_tx, drive_command_rx) = mpsc::channel(8);
        let drive_body = provider_run_drive_body(
            arm_drive(
                engine_guard,
                drive_command_rx,
                provider_for_run.clone(),
                &run_context,
                &outbound_tx,
                &mut idle_guard,
                &run_cancel,
                run_token,
            ),
            ProviderRunKind::Author {
                content: "stack budget baseline".to_string(),
            },
        );
        let drive_bytes = std::mem::size_of_val(&drive_body);
        print_raw_baseline("provider_run_drive_body", drive_bytes);
        drop(drive_body);
    }
    // —— 基础臂 / SC author 臂原始 child：逐对象构造量测（各自作用域，互不
    //    共享 &mut idle Option 与 engine guard；仍只构造不 poll）——
    let arm_bytes = measure_named_arms(
        &run_context,
        &outbound_tx,
        &run_cancel,
        provider_for_run.clone(),
        run_token,
    )
    .await;

    // —— 注册前入口 future 与取消 settle helper：只记录（不设硬断言）——
    let entry = spawn_provider_run_from_handler(
        run_context.clone(),
        ProviderRunKind::Author {
            content: "stack budget baseline".to_string(),
        },
        outbound_tx.clone(),
    );
    print_raw_baseline(
        "spawn_provider_run_from_handler",
        std::mem::size_of_val(&entry),
    );
    drop(entry);

    let settle = crate::web::workspace_ws_handler::run::drive_until_cancelled_settles(
        std::future::pending(),
        &run_cancel,
        None,
        &run_context.engine,
        &outbound_tx,
        &run_context.session_id,
        "run-1",
        &run_context.manager,
        run_token,
    );
    print_raw_baseline(
        "drive_until_cancelled_settles(pending)",
        std::mem::size_of_val(&settle),
    );
    drop(settle);

    println!("[stack-budget] 基础/SC author 臂量测汇总: {arm_bytes:?}");
    assert!(
        task_body_bytes <= RAW_TASK_BODY_BUDGET,
        "raw future provider_run_task_body = {task_body_bytes} B exceeds \
         {RAW_TASK_BODY_BUDGET} B（S0 机械提取基线门禁；profile/target/toolchain 见上方打印）"
    );
}

/// 基础臂与 SC author 臂的原始 child 体积量测（S0 只记录，硬断言随各族
/// 迁移逐项冻结——设计 §2.2「先记录机械提取后的真实基线」）。
async fn measure_named_arms(
    run_context: &ProviderRunContext,
    outbound_tx: &mpsc::Sender<OutboundControl>,
    run_cancel: &CancellationToken,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    run_token: u64,
) -> Vec<(&'static str, usize)> {
    let mut sizes = Vec::new();
    {
        let engine_guard = run_context.engine.lock().await;
        let mut idle_guard = Some(
            run_context
                .workspace_runs
                .begin_provider_drive(&run_context.session_id),
        );
        let (_tx, command_rx) = mpsc::channel(8);
        let author = drive_author_arm(
            arm_drive(
                engine_guard,
                command_rx,
                provider_for_run.clone(),
                run_context,
                outbound_tx,
                &mut idle_guard,
                run_cancel,
                run_token,
            ),
            "stack budget baseline".to_string(),
        );
        let bytes = std::mem::size_of_val(&author);
        print_raw_baseline("drive_author_arm", bytes);
        sizes.push(("drive_author_arm", bytes));
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
        let followup = drive_author_choice_followup_arm(
            arm_drive(
                engine_guard,
                command_rx,
                provider_for_run.clone(),
                run_context,
                outbound_tx,
                &mut idle_guard,
                run_cancel,
                run_token,
            ),
            "stack budget baseline".to_string(),
        );
        let bytes = std::mem::size_of_val(&followup);
        print_raw_baseline("drive_author_choice_followup_arm", bytes);
        sizes.push(("drive_author_choice_followup_arm", bytes));
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
        let revision = drive_revision_arm(arm_drive(
            engine_guard,
            command_rx,
            provider_for_run.clone(),
            run_context,
            outbound_tx,
            &mut idle_guard,
            run_cancel,
            run_token,
        ));
        let bytes = std::mem::size_of_val(&revision);
        print_raw_baseline("drive_revision_arm", bytes);
        sizes.push(("drive_revision_arm", bytes));
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
        let review_only = drive_review_only_arm(arm_drive(
            engine_guard,
            command_rx,
            provider_for_run.clone(),
            run_context,
            outbound_tx,
            &mut idle_guard,
            run_cancel,
            run_token,
        ));
        let bytes = std::mem::size_of_val(&review_only);
        print_raw_baseline("drive_review_only_arm", bytes);
        sizes.push(("drive_review_only_arm", bytes));
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
        let single_candidate = drive_single_candidate_author_arm(arm_drive(
            engine_guard,
            command_rx,
            provider_for_run.clone(),
            run_context,
            outbound_tx,
            &mut idle_guard,
            run_cancel,
            run_token,
        ));
        let bytes = std::mem::size_of_val(&single_candidate);
        print_raw_baseline("drive_single_candidate_author_arm", bytes);
        sizes.push(("drive_single_candidate_author_arm", bytes));
        drop(single_candidate);
    }
    {
        let engine_guard = run_context.engine.lock().await;
        let mut idle_guard = Some(
            run_context
                .workspace_runs
                .begin_provider_drive(&run_context.session_id),
        );
        let (_tx, command_rx) = mpsc::channel(8);
        let sc_manual = drive_human_gate_sc_revision_arm(
            arm_drive(
                engine_guard,
                command_rx,
                provider_for_run,
                run_context,
                outbound_tx,
                &mut idle_guard,
                run_cancel,
                run_token,
            ),
            "turn-0001".to_string(),
            String::new(),
        );
        let bytes = std::mem::size_of_val(&sc_manual);
        print_raw_baseline("drive_human_gate_sc_revision_arm", bytes);
        sizes.push(("drive_human_gate_sc_revision_arm", bytes));
        drop(sc_manual);
    }
    sizes
}

/// 臂共享入参的构造捷径（与 task_body 分发点字段一一对应）。
/// （P2/任务3 起 `pub(super)`：`basic_sc_arm_cases` 直驱臂用例复用。）
#[allow(clippy::too_many_arguments)]
pub(super) fn arm_drive<'e, 'd>(
    engine: tokio::sync::MutexGuard<'e, WorkspaceEngine>,
    command_rx: mpsc::Receiver<ProviderCommand>,
    provider: Arc<dyn StreamingProviderAdapter>,
    run_context: &'d ProviderRunContext,
    outbound_tx: &'d mpsc::Sender<OutboundControl>,
    provider_drive_guard: &'d mut Option<crate::web::state::WorkspaceProviderDriveGuard>,
    run_cancel: &'d CancellationToken,
    run_token: u64,
) -> crate::web::workspace_ws_handler::run::ArmDrive<'e, 'd> {
    crate::web::workspace_ws_handler::run::ArmDrive {
        engine,
        command_rx,
        provider,
        manager: &run_context.manager,
        provider_registry: &run_context.provider_registry,
        run_context,
        outbound_tx,
        run_label: "run-1",
        run_token,
        provider_drive_guard,
        run_cancel,
        // S2：drive 尾部字段量测构造默认空置（尺寸按类型固定，不受运行期
        // 值影响；legacy 载荷随 P3 迁移后再按族冻结）。
        rebuild_context: None,
        outline_revision_feedback: None,
        is_outline_revision_run: false,
    }
}
