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

/// 打印单对象基线：实际字节 / profile / target / toolchain 一次成型，供
/// 报告留档与 dev/release 双基线对比。
fn print_raw_baseline(object: &str, bytes: usize) {
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
async fn stack_budget_fixture() -> (
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
            // 机械提取后预算意外已绿：如实打印（S0 预期为红，绿属异常需复核）。
            println!(
                "[stack-budget] raw-budget child unexpectedly green: task_body already \
                 within {RAW_TASK_BODY_BUDGET} B"
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
    print_raw_baseline("spawn_provider_run_from_handler", std::mem::size_of_val(&entry));
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
        let mut idle_guard =
            Some(run_context.workspace_runs.begin_provider_drive(&run_context.session_id));
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
        let mut idle_guard =
            Some(run_context.workspace_runs.begin_provider_drive(&run_context.session_id));
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
        let mut idle_guard =
            Some(run_context.workspace_runs.begin_provider_drive(&run_context.session_id));
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
        let mut idle_guard =
            Some(run_context.workspace_runs.begin_provider_drive(&run_context.session_id));
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
        let mut idle_guard =
            Some(run_context.workspace_runs.begin_provider_drive(&run_context.session_id));
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
        let mut idle_guard =
            Some(run_context.workspace_runs.begin_provider_drive(&run_context.session_id));
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
#[allow(clippy::too_many_arguments)]
fn arm_drive<'e, 'd>(
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
        run_context,
        outbound_tx,
        run_label: "run-1",
        run_token,
        provider_drive_guard,
        run_cancel,
    }
}
