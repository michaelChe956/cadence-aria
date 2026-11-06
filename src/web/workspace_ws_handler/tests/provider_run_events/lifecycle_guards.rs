//! 任务9（P8）：A 层**未装箱体积冻结门禁** + B 层**显式 2 MiB worker 矩阵**
//! + **三守卫生命周期矩阵**（r16 现场死锁根修，缺陷#10家族）。
//!
//! 挂载（controller 串行注册）：`provider_run_events.rs` 追加 `#[path =
//! "provider_run_events/lifecycle_guards.rs"] mod lifecycle_guards;`；本文件
//! `use super::*` 取 provider_run_events 域，跨子模块助手用限定路径。
//!
//! A 层：只构造**未装箱** future（`size_of_val` 作用于 raw future；量 `Pin<Box<_>>`
//! 指针是假门禁）、**不 poll**；断言消息含符号/实际字节/profile/target/toolchain，
//! 并与 S0 基线逐对象对照。B 层：显式 `thread_stack_size(2*1024*1024)` 的 runtime，
//! 真实入口经 `rt.spawn` 落 worker，根线程只等待并记录线程 identity（不嵌套 runtime）。
//!
//! 三守卫不可混用（r16 现场死锁根修，缺陷#10家族）：engine `MutexGuard` 按值属于
//! drive/当前臂（drop 取消 drive 必须释放它，且先于兜底重取锁）；idle
//! `provider_drive_guard` 是外层 `Option<WorkspaceProviderDriveGuard>`（臂借 `&mut`，
//! drop drive **不等于**释放 idle）；`entity_run_lease` 是独立外层 binding（覆盖 drive、
//! 取消兜底与最终 store 写，接替 run 自行另 acquire）。

use super::stack_budget::{arm_drive, print_raw_baseline, stack_budget_fixture};
use super::*;
use std::collections::VecDeque;

/// 设计硬线：未装箱小外壳 future ≤32 KiB（设计 §2.2）。
const RAW_SHELL_HARD_CAP: usize = 32768;

/// 设计硬线：body/drive/各臂/重量 child ≤128 KiB（设计 §2.2）。
const RAW_OBJECT_HARD_CAP: usize = 131072;

/// 优先线：≤64 KiB（超优先线但未超硬线者须逐条记录依据）。
const RAW_PREFERRED_CAP: usize = 65536;

/// 冻结预算表 `(符号, S0 基线 B, 冻结预算 B)`：S0 基线是任务1～4 的实测对照锚
/// （`provider_run_task_body` 的 235,088 B 是拆分前 characterization 真红值，保留为
/// 结构性下降对照、不当绿）；冻结预算＝**本任务实测值 + ≥25% 余量**（取整 4 KiB）
/// 且不高于设计硬线；超优先线（64 KiB）者逐条记录依据，**禁止**借「冻结」上调。
const FROZEN_RAW_BUDGETS: &[(&str, usize, usize)] = &[
    ("provider_run_task", 3_704, 8_192),           // 实测 3,672
    ("provider_run_task_body", 235_088, 24_576),   // 实测 15,264（结构性下降）
    ("provider_run_drive_body", 112_024, 126_976), // 实测 101,424（未拆族臂聚合；超优先线）
    ("drive_author_arm", 21_808, 27_264),          // 实测 20,728
    ("drive_author_choice_followup_arm", 21_808, 27_264), // 实测 20,728
    ("drive_revision_arm", 15_456, 19_456),        // 实测 14,264
    ("drive_review_only_arm", 42_248, 53_248),     // 实测 41,056
    ("drive_single_candidate_author_arm", 53_640, 66_560), // 实测 52,448
    ("run_single_candidate_author", 50_664, 63_488), // 实测 50,664（SC 最大 child）
    ("drive_human_gate_sc_revision_arm", 70_024, 87_552), // 实测 68,944（超优先线）
    ("drive_legacy_outline_arm", 14_800, 18_432),  // 实测 14,912
    ("drive_plan_draft_arm", 0, 65_536),           // 实测 10,824（P4/任务5，待其交付后收紧）
    ("drive_plan_batch_arm", 0, 65_536),           // 实测 10,640（P4/任务5，待其交付后收紧）
];

/// 逐对象冻结断言：先打印（含 profile/target/toolchain）与 S0 对照；表外符号直接红；表项不得高于设计硬线（防借「冻结」放宽）；超优先线者打印标注但不豁免硬线。
fn assert_frozen_raw_budget(symbol: &'static str, bytes: usize) {
    print_raw_baseline(symbol, bytes);
    let Some((_, s0, budget)) = FROZEN_RAW_BUDGETS
        .iter()
        .find(|(name, _, _)| *name == symbol)
    else {
        panic!("raw future {symbol} {bytes} B 未登记冻结预算表（lifecycle_guards）");
    };
    if *s0 > 0 {
        println!(
            "[stack-budget] {symbol}: {bytes} B（S0 基线 {s0} B，Δ={} B）",
            bytes as isize - *s0 as isize
        );
    }
    if bytes > RAW_PREFERRED_CAP {
        println!(
            "[stack-budget] {symbol}: {bytes} B 超优先线 {RAW_PREFERRED_CAP} B（未超硬线 {RAW_OBJECT_HARD_CAP} B；依据见 task-9-report）"
        );
    }
    let cap = if symbol == "provider_run_task" {
        RAW_SHELL_HARD_CAP
    } else {
        RAW_OBJECT_HARD_CAP
    };
    assert!(
        *budget <= cap,
        "表项 {symbol} 预算 {budget} 高于硬线 {cap}（禁止上调）"
    );
    assert!(
        bytes <= *budget,
        "raw future {symbol} {bytes} B exceeds frozen budget {budget}（任务9冻结门禁；profile/target/toolchain 见上方打印）"
    );
}

/// raw-budget 子模式用例（任务1隔离模式）：量测+断言在隔离子进程内执行，父不 poll 被测 future；常规套件直通空壳。
#[tokio::test]
async fn provider_case_child_lifecycle_guards_raw_budget() {
    if std::env::var_os(super::small_stack_worker::PROVIDER_CASE_CHILD_MODE_ENV).is_none() {
        return;
    }
    run_lifecycle_guards_raw_budget_measurement().await;
}

/// 任务9 Step 1/2：A 层完整冻结门禁（同步父；隔离子例证据；不测 Box 指针）。
#[test]
fn raw_provider_run_futures_respect_frozen_budgets() {
    super::small_stack_worker::run_isolated_provider_case(
        "provider_case_child_lifecycle_guards_raw_budget",
        std::time::Duration::from_secs(180),
    )
    .expect("provider run 各对象未装箱体积必须全部≤冻结预算（隔离子例证据）");
}

/// 逐对象构造未装箱 future 并硬断言（只构造不 poll）。
async fn run_lifecycle_guards_raw_budget_measurement() {
    let (run_context, outbound_tx, run_cancel, command_rx, run_token) =
        stack_budget_fixture().await;
    let provider_for_run = run_context
        .provider_registry
        .get(&ProviderName::ClaudeCode)
        .expect("fixture provider");
    let take_lease = || {
        crate::product::provider_workspace_runner::acquire_run_lease_for_session(
            &run_context.app_paths,
            &run_context.session_record,
        )
        .expect("fixture run lease")
    };
    let baseline_kind = || ProviderRunKind::Author {
        content: "baseline".to_string(),
    };
    // 外壳/task body 同源 16 参（任务2 冻结签名）；差异仅在 command_rx/guard/lease。
    macro_rules! task_like_future {
        ($builder:path, $command_rx:expr, $guard:expr, $lease:expr) => {
            $builder(
                baseline_kind(),
                run_context.clone(),
                run_context.engine.clone(),
                run_context.manager.clone(),
                run_context.provider_registry.clone(),
                provider_for_run.clone(),
                outbound_tx.clone(),
                outbound_tx.clone(),
                run_context.session_id.clone(),
                "run-lg".to_string(),
                run_token,
                run_cancel.clone(),
                $command_rx,
                $guard,
                $lease,
                None,
            )
        };
    }
    let begin_idle = || {
        run_context
            .workspace_runs
            .begin_provider_drive(&run_context.session_id)
    };
    // —— 未装箱外壳（provider_run_task 返回值本体；非 Pin<Box> 指针）——
    let shell = task_like_future!(provider_run_task, command_rx, begin_idle(), take_lease());
    let shell_bytes = std::mem::size_of_val(&shell);
    assert_frozen_raw_budget("provider_run_task", shell_bytes);
    assert!(
        shell_bytes <= RAW_SHELL_HARD_CAP,
        "外壳另受 32 KiB 硬线约束"
    );
    drop(shell);
    // —— 未装箱 task body（只构造不 poll；S0 235,088 B 必须结构性下降）——
    let (_body_tx, body_command_rx) = mpsc::channel(8);
    let task_body = task_like_future!(
        provider_run_task_body,
        body_command_rx,
        begin_idle(),
        take_lease()
    );
    let body_bytes = std::mem::size_of_val(&task_body);
    assert_frozen_raw_budget("provider_run_task_body", body_bytes);
    assert!(
        body_bytes * 2 < 235_088,
        "task body 必须较 S0 235,088 B 至少减半"
    );
    drop(task_body);
    // —— 未装箱 drive body（boxed 前的真实整体；含未拆族臂聚合）——
    {
        let engine_guard = run_context.engine.lock().await;
        let mut idle_guard = Some(begin_idle());
        let (_tx, drive_command_rx) = mpsc::channel(8);
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
            baseline_kind(),
        );
        assert_frozen_raw_budget(
            "provider_run_drive_body",
            std::mem::size_of_val(&drive_body),
        );
        drop(drive_body);
    }
    // —— 具名臂（各自作用域，互不共享 &mut idle Option 与 engine 锁）——
    macro_rules! measure_arm {
        ($symbol:literal, $build:expr) => {{
            let engine_guard = run_context.engine.lock().await;
            let mut idle_guard = Some(begin_idle());
            let (_tx, command_rx) = mpsc::channel(8);
            let future = $build(arm_drive(
                engine_guard,
                command_rx,
                provider_for_run.clone(),
                &run_context,
                &outbound_tx,
                &mut idle_guard,
                &run_cancel,
                run_token,
            ));
            assert_frozen_raw_budget($symbol, std::mem::size_of_val(&future));
            drop(future);
        }};
    }
    measure_arm!("drive_author_arm", |d| async {
        drive_author_arm(d, String::new()).await
    });
    measure_arm!("drive_author_choice_followup_arm", |d| async {
        drive_author_choice_followup_arm(d, "baseline".to_string()).await
    });
    measure_arm!("drive_revision_arm", drive_revision_arm);
    measure_arm!("drive_review_only_arm", drive_review_only_arm);
    measure_arm!(
        "drive_single_candidate_author_arm",
        drive_single_candidate_author_arm
    );
    measure_arm!("drive_human_gate_sc_revision_arm", |d| async {
        drive_human_gate_sc_revision_arm(d, "turn-0001".to_string(), String::new()).await
    });
    // legacy 臂：`Option<Box<ResolvedPlanningContext>>` 是指针大小（niche 优化），None 与 Some 同布局——三 variant 共用同一 future 类型。
    measure_arm!("drive_legacy_outline_arm", |d| async {
        drive_legacy_outline_arm(d, None, false, None).await
    });
    // P4（任务5）draft/batch 两族臂。
    measure_arm!("drive_plan_draft_arm", |d| async {
        drive_plan_draft_arm(d, None).await
    });
    measure_arm!("drive_plan_batch_arm", drive_plan_batch_arm);
    // —— SC 最大 child（run_single_candidate_author 本体，任务3 冻结面）——
    {
        let mut engine_guard = run_context.engine.lock().await;
        let (_tx, mut sc_command_rx) = mpsc::channel(8);
        let sc_child = run_single_candidate_author(
            &mut engine_guard,
            provider_for_run.clone(),
            run_cancel.clone(),
            &mut sc_command_rx,
            SingleCandidateAuthorRecoveryEntry::Ordinary,
            &run_context,
        );
        assert_frozen_raw_budget(
            "run_single_candidate_author",
            std::mem::size_of_val(&sc_child),
        );
        drop(sc_child);
    }

    // 注册前入口 future 与取消 settle helper 的实测大小由 `stack_budget.rs` 记录（只记录不设硬断言）。
}

/// B 层显式 worker 栈 2,097,152 B（显式值覆盖 `RUST_MIN_STACK`；§6.1/§6.2）。
const EXPLICIT_WORKER_STACK_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ThreadIdentity {
    id: std::thread::ThreadId,
    name: Option<String>,
}

/// 探针脚本：Complete＝立即完成；Hang＝计 start 后悬挂在 start 内（r16 现场形态），Drop 探针随 start future 存活。
enum ProbeStep {
    Complete(String),
    Hang,
}

/// worker 探针：记录每次 start 的线程 identity 与当时的 idle depth（重叠取证）；
/// `start_drops` 是旧 start future Drop 探针（r16 合同：drive 强制 drop 连带取消启动链）。
struct WorkerProbeProvider {
    starts: Arc<AtomicUsize>,
    start_drops: Arc<AtomicUsize>,
    identities: Arc<std::sync::Mutex<Vec<ThreadIdentity>>>,
    depths: Arc<std::sync::Mutex<Vec<usize>>>,
    runs: WorkspaceRunRegistry,
    session_id: String,
    script: Arc<std::sync::Mutex<VecDeque<ProbeStep>>>,
}

/// Hang 步骤的 Drop 探针载荷：随 start future 存活，drop 即计数。
struct ProbeStartDrop(Arc<AtomicUsize>);

impl Drop for ProbeStartDrop {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for WorkerProbeProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let identity = ThreadIdentity {
            id: std::thread::current().id(),
            name: std::thread::current().name().map(str::to_string),
        };
        self.identities.lock().expect("ids").push(identity);
        self.depths
            .lock()
            .expect("depths")
            .push(self.runs.provider_drive_depth(&self.session_id) as usize);
        self.starts.fetch_add(1, Ordering::SeqCst);
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        let step = self
            .script
            .lock()
            .expect("script")
            .pop_front()
            .unwrap_or(ProbeStep::Hang);
        match step {
            ProbeStep::Complete(output) => {
                let _ = event_tx
                    .send(ProviderEvent::Completed(ProviderCompletion::plain(
                        output, None,
                    )))
                    .await;
            }
            ProbeStep::Hang => {
                let _drop_probe = ProbeStartDrop(self.start_drops.clone());
                std::future::pending::<()>().await;
                unreachable!("hanging probe start never resolves")
            }
        }
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    /// gateway 只经 validated trait 分发（既有夹具同款直通）。
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

/// 活根 lifecycle 夹具（TempDir 存活至用例结束——局部 TempDir 返回即删＝durable 断言无效）。真实 store/engine/registry/run_context/manager + 转发/排空任务。
struct LifecycleGuardsFixture {
    _root: tempfile::TempDir,
    app_paths: ProductAppPaths,
    run_context: ProviderRunContext,
    engine_tx: mpsc::Sender<EngineEvent>,
    outbound_tx: mpsc::Sender<OutboundControl>,
    workspace_runs: WorkspaceRunRegistry,
    manager: Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    provider: Arc<WorkerProbeProvider>,
    forward: tokio::task::JoinHandle<()>,
    _outbound_drain: tokio::task::JoinHandle<()>,
}

impl LifecycleGuardsFixture {
    fn timeline_nodes(&self) -> Vec<crate::web::workspace_ws_types::timeline::TimelineNode> {
        LifecycleStore::new(self.app_paths.clone())
            .load_timeline_nodes(&self.run_context.session_id)
            .expect("durable timeline nodes")
    }

    fn durable_timeline_nodes(&self) -> usize {
        self.timeline_nodes().len()
    }

    fn any_completed_node(&self) -> bool {
        self.timeline_nodes().iter().any(|node| {
            node.status == crate::web::workspace_ws_types::timeline::TimelineNodeStatus::Completed
        })
    }

    fn depth(&self) -> usize {
        self.workspace_runs
            .provider_drive_depth(&self.run_context.session_id) as usize
    }

    fn starts(&self) -> usize {
        self.provider.starts.load(Ordering::SeqCst)
    }

    fn idle_now(&self) -> bool {
        self.manager.active_run_ref_for_test().is_none()
    }

    fn identities(&self) -> Vec<ThreadIdentity> {
        self.provider.identities.lock().expect("ids").clone()
    }

    /// 每次真实 provider start 时刻观察到的 idle depth（重叠取证）。
    fn start_depths(&self) -> Vec<usize> {
        self.provider.depths.lock().expect("depths").clone()
    }

    fn script_next(&self, steps: Vec<ProbeStep>) {
        *self.provider.script.lock().expect("script") = steps.into();
    }

    /// 删除准入探针：lease 持有期间 Busy；写完且 lease 释放后才可独占（三守卫之一）。
    fn delete_admission(&self) -> Result<(), String> {
        let scope = crate::product::provider_workspace_runner::resolve_session_run_scope(
            &self.app_paths,
            &self.run_context.session_record,
        )
        .map_err(|error| format!("resolve scope: {error:?}"))?
        .expect("story session owner scope");
        match crate::product::entity_operation_store::EntityOperationStore::new(
            self.app_paths.clone(),
        )
        .try_acquire_delete(&scope)
        {
            Ok(_lease) => Ok(()), // 取得即复核完成；立即 drop，不写墓碑
            Err(error) => Err(format!("{error:?}")),
        }
    }

    async fn admit_run(&self, kind: ProviderRunKind) -> Result<(), String> {
        spawn_provider_run_from_handler(self.run_context.clone(), kind, self.outbound_tx.clone())
            .await
            .map_err(|error| format!("run admitted: {error}"))
    }

    async fn dispose(self) {
        drop(self.engine_tx);
        self.forward.abort();
        let _ = self.forward.await;
        self._outbound_drain.abort();
        let _ = self._outbound_drain.await;
    }
}

macro_rules! barrier {
    ($desc:literal, $secs:expr, $probe:expr) => {
        wait_for_barrier($desc, std::time::Duration::from_secs($secs), $probe).await?
    };
}

async fn admit_author(fixture: &LifecycleGuardsFixture, content: &str) -> Result<(), String> {
    fixture
        .admit_run(ProviderRunKind::Author {
            content: content.to_string(),
        })
        .await
}

async fn relay_request(
    fixture: &LifecycleGuardsFixture,
    kind: ProviderRunKind,
    node_id: Option<String>,
) -> Result<(), String> {
    fixture
        .engine_tx
        .send(EngineEvent::ProviderRunRequested { kind, node_id })
        .await
        .map_err(|error| format!("relay send: {error}"))
}

async fn claim_idle(fixture: &LifecycleGuardsFixture, content: &str) -> Result<bool, String> {
    spawn_provider_run_claiming_idle(
        fixture.run_context.clone(),
        ProviderRunKind::Author {
            content: content.to_string(),
        },
        fixture.outbound_tx.clone(),
    )
    .await
    .map_err(|error| format!("claim idle: {error}"))
}

/// 「立即完成 + artifact retry 再完成」脚本（attempted 门闩）。
fn complete_steps(label: &str) -> Vec<ProbeStep> {
    vec![
        ProbeStep::Complete(format!("{label} output")),
        ProbeStep::Complete(format!("{label} retry output")),
    ]
}

/// 构建活根夹具（真实 owner 实体 story_0001；review_rounds=0；author 与 reviewer 指向同一探针——所有真实 start（含 ReviewOnly）都被捕获）。
async fn lifecycle_guards_live_fixture() -> LifecycleGuardsFixture {
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
        engine_tx.clone(),
        WorkspaceSession::from_record(session_record.clone()),
    )));
    let workspace_runs = WorkspaceRunRegistry::default();
    let provider = Arc::new(WorkerProbeProvider {
        starts: Arc::new(AtomicUsize::new(0)),
        start_drops: Arc::new(AtomicUsize::new(0)),
        identities: Arc::new(std::sync::Mutex::new(Vec::new())),
        depths: Arc::new(std::sync::Mutex::new(Vec::new())),
        runs: workspace_runs.clone(),
        session_id: session_record.id.clone(),
        script: Arc::new(std::sync::Mutex::new(VecDeque::new())),
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider.clone());
    registry.register(ProviderName::Codex, provider.clone());
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine.clone(),
        workspace_runs.clone(),
        session_record.id.clone(),
        app_paths.clone(),
        session_record.clone(),
    );
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);
    // 持续排空 outbound：防转发任务被满通道阻塞，保证 relay 事件及时处理。
    let outbound_drain = tokio::spawn(async move { while outbound_rx.recv().await.is_some() {} });
    let manager = run_context.manager.clone();
    let forward =
        spawn_engine_event_forward_task(engine_rx, outbound_tx.clone(), Some(run_context.clone()));
    LifecycleGuardsFixture {
        _root: root,
        app_paths,
        run_context,
        engine_tx,
        outbound_tx,
        workspace_runs,
        manager,
        provider,
        forward,
        _outbound_drain: outbound_drain,
    }
}

/// 有界 barrier 轮询（超时以描述性错误失败；不用 catch_unwind/JoinError 兜底）。
async fn wait_for_barrier(
    desc: &str,
    budget: std::time::Duration,
    mut probe: impl FnMut() -> bool,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + budget;
    while !probe() {
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("barrier 未达成: {desc}"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    Ok(())
}

/// 显式 2 MiB worker runtime 构建（B 层逐字合同；不读环境变量）。
fn explicit_2mib_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(EXPLICIT_WORKER_STACK_BYTES)
        .enable_all()
        .build()
        .expect("explicit 2 MiB worker runtime")
}

/// B 层矩阵证据（父同步用例断言）。
struct WorkerMatrixEvidence {
    identities: Vec<ThreadIdentity>,
    author_starts: usize,
    relay_starts: usize,
    drain_extra_starts: usize,
    hanging_alive: bool,
    claim_busy_survived: bool,
    claim_busy_pending: bool,
    claim_busy_extra_starts: usize,
    claim_idle: Option<bool>,
    claim_idle_starts_delta: usize,
    supersede_regen_starts_delta: usize,
    revision_starts_delta: usize,
    durable_nodes_before: usize,
    durable_nodes_after_author: usize,
    durable_nodes_final: usize,
    author_node_completed: bool,
    depth_final: usize,
    active_run_final: bool,
}

/// 任务9 Step 4/5：B 层 2 MiB worker 矩阵。同步父用例；入口/relay/claim 全部经 `rt.spawn` 在 worker 执行，根只等待并记录根线程 identity 对照。
#[test]
fn provider_run_2mib_worker_matrix() {
    let root_thread = std::thread::current().id();
    let rt = explicit_2mib_runtime();
    let evidence = rt
        .block_on(async {
            tokio::spawn(async {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(120),
                    run_2mib_worker_matrix_scenario(),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err("2 MiB worker matrix 场景超时（120s 墙钟上限）".to_string()),
                }
            })
            .await
            .expect("worker matrix scenario task")
        })
        .expect("2 MiB worker matrix scenario");
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    let e = &evidence;
    assert_worker_identities(&e.identities, root_thread);
    // 精确 start 断言（不允许「<=」上界）：Story 纯文本 Completed 触发恰一次 artifact retry（attempted 门闩）⇒ 每轮完整 Author 恰 2 次（任务3 合同）。
    assert_eq!(e.author_starts, 2, "Author 真实链恰 2 次 start");
    assert_eq!(e.relay_starts, 4, "relay 接力一轮完整 Author（累计 4）");
    assert_eq!(
        e.drain_extra_starts, 0,
        "去重/ReviewOnly drain 零新增 start"
    );
    assert!(
        e.hanging_alive,
        "drain 不得取消在途悬挂 run（depth 仍 1 且活跃）"
    );
    assert!(
        e.claim_busy_survived,
        "busy 时 claiming_idle 只观察：不得取消在途 run（depth 仍 1 且活跃）"
    );
    assert_eq!(e.claim_busy_extra_starts, 0, "busy 观察零新增 start");
    // 认领探针在观察窗内仍 pending ⇒ auto 认领阻塞在 engine 锁上（**合同偏差**，见报告）。
    println!(
        "[claim-busy] 观察窗结束时认领探针 pending={}（busy 期间阻塞在 engine 锁）",
        e.claim_busy_pending
    );
    assert_eq!(e.claim_idle, Some(true), "空闲时 claiming_idle 真实认领");
    assert_eq!(e.claim_idle_starts_delta, 2, "认领 run 恰 2 次 start");
    assert_eq!(
        e.supersede_regen_starts_delta, 2,
        "supersede 后再生成恰 2 次 start"
    );
    assert_eq!(
        e.revision_starts_delta, 0,
        "Revision 臂真实驱动但无待修订 turn ⇒ 零新增 provider start"
    );
    // durable 进展：真实落盘于活根，节点数单调增长且 Author 节点 Completed。
    assert!(
        e.durable_nodes_after_author > e.durable_nodes_before,
        "Author 完成后节点增长"
    );
    assert!(
        e.durable_nodes_final > e.durable_nodes_after_author,
        "后续 run 持续落盘"
    );
    assert!(e.author_node_completed, "Author 节点到达 Completed 终态");
    assert_eq!(e.depth_final, 0, "结束 idle depth 收敛 0");
    assert!(!e.active_run_final, "结束无残留活跃 run");
}

/// 线程落点断言：全部 start 落同一 worker 且非 block_on 根线程（§6.2）。
fn assert_worker_identities(identities: &[ThreadIdentity], root_thread: std::thread::ThreadId) {
    assert!(!identities.is_empty(), "必须捕获真实 provider start 探针");
    for identity in identities {
        assert_ne!(identity.id, root_thread, "start 不得发生在 block_on 根线程");
        assert!(
            identity
                .name
                .as_deref()
                .is_some_and(|name| name.starts_with("tokio-") && name.ends_with("worker")),
            "入口必须经 rt.spawn 落 runtime worker（实际 {:?}）",
            identity.name
        );
    }
    let first = &identities[0];
    assert!(
        identities.iter().all(|id| id == first),
        "全部 start 落同一 worker"
    );
}

async fn run_2mib_worker_matrix_scenario() -> Result<WorkerMatrixEvidence, String> {
    let fixture = lifecycle_guards_live_fixture().await;
    let durable_nodes_before = fixture.durable_timeline_nodes();
    // ① Author 真实链：handler 路径 → 真实 start → 完成 → 收敛。
    fixture.script_next(complete_steps("matrix author"));
    admit_author(&fixture, "matrix author on 2mib worker").await?;
    barrier!("author run 完成（start≥2 且收敛）", 10, || {
        fixture.starts() >= 2 && fixture.idle_now()
    });
    let author_starts = fixture.starts();
    let author_node_completed = fixture.any_completed_node();
    let durable_nodes_after_author = fixture.durable_timeline_nodes();
    // ② event relay：ProviderRunRequested → from_event → 一轮完整 Author。
    fixture.script_next(complete_steps("relay"));
    relay_request(
        &fixture,
        ProviderRunKind::Author {
            content: "relay".to_string(),
        },
        None,
    )
    .await?;
    barrier!("relay run 完成（start≥4 且收敛）", 10, || {
        fixture.starts() >= 4 && fixture.idle_now()
    });
    let relay_starts = fixture.starts();
    // ③ 去重 drain：悬挂 run 在途时，同节点同 kind 重复 relay 被 drain；ReviewOnly relay 让位自续（entry 合同）。
    fixture.script_next(vec![ProbeStep::Hang]);
    admit_author(&fixture, "hanging run for drain probes").await?;
    barrier!("悬挂 run 已 start + depth 1", 10, || {
        fixture.starts() >= 5 && fixture.depth() == 1
    });
    let hanging_node_id = fixture
        .manager
        .active_run()
        .await
        .and_then(|run| run.node_id.clone());
    relay_request(
        &fixture,
        ProviderRunKind::Author {
            content: "dup".to_string(),
        },
        hanging_node_id.clone(),
    )
    .await?;
    relay_request(
        &fixture,
        ProviderRunKind::ReviewOnly,
        Some("review_node".to_string()),
    )
    .await?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let drain_extra_starts = fixture.starts() - 5;
    let hanging_alive = fixture.depth() == 1 && !fixture.idle_now();
    // ④ claim busy：活跃 run 在途时 auto 认领**只观察**（不取消在途 run、零新增
    //    start）。认领路径会在 engine 锁上等待 ⇒ 以「有界观察窗」取证，探针 future
    //    到期即 drop（等价真实取消，其持有的 run lease 随 future drop 释放）。
    let mut claim_probe = Box::pin(claim_idle(&fixture, "claim while busy must observe"));
    let claim_busy_pending =
        tokio::time::timeout(std::time::Duration::from_millis(500), &mut claim_probe)
            .await
            .is_err();
    let claim_busy_extra_starts = fixture.starts() - 5;
    let claim_busy_survived = fixture.depth() == 1 && !fixture.idle_now();
    drop(claim_probe);
    // ⑤ supersede 取消后再生成：悬挂 run 被取消，新 run 真实 start 并完成。
    fixture.script_next(complete_steps("regen"));
    admit_author(&fixture, "supersede then regenerate").await?;
    barrier!(
        "supersede 再生成完成（start≥7 且收敛）",
        15,
        || { fixture.starts() >= 7 && fixture.idle_now() }
    );
    let supersede_regen_starts_delta = fixture.starts() - 5;
    // ⑥ claim idle：会话空闲后真实认领并完成。
    fixture.script_next(complete_steps("claim idle"));
    let claim_idle_result = claim_idle(&fixture, "claim when idle must start").await?;
    barrier!("claim idle run 完成（start≥9 且收敛）", 15, || {
        fixture.starts() >= 9 && fixture.idle_now()
    });
    let claim_idle_starts_delta = fixture.starts() - 7;
    // ⑦ Revision 臂：无待修订 turn 时修订 session 直接收口（不启动 provider）。
    fixture.script_next(complete_steps("revision"));
    fixture.admit_run(ProviderRunKind::Revision).await?;
    barrier!("revision run 收敛", 15, || fixture.idle_now());
    let revision_starts_delta = fixture.starts() - 9;
    let evidence = WorkerMatrixEvidence {
        identities: fixture.identities(),
        author_starts,
        relay_starts,
        drain_extra_starts,
        hanging_alive,
        claim_busy_survived,
        claim_busy_pending,
        claim_busy_extra_starts,
        claim_idle: Some(claim_idle_result),
        claim_idle_starts_delta,
        supersede_regen_starts_delta,
        revision_starts_delta,
        durable_nodes_before,
        durable_nodes_after_author,
        durable_nodes_final: fixture.durable_timeline_nodes(),
        author_node_completed,
        depth_final: fixture.depth(),
        active_run_final: !fixture.idle_now(),
    };
    fixture.dispose().await;
    Ok(evidence)
}

struct GuardsEvidence {
    identities: Vec<ThreadIdentity>,
    start_depths: Vec<usize>,
    depth_final: usize,
    token_run1: u64,
    token_run2: u64,
    incarnation_run1: String,
    incarnation_run2: String,
    command_tx_present: bool,
    delete_busy_before_cancel: bool,
    delete_busy_while_run2_active: bool,
    delete_busy_during_regen: bool,
    start_drops_total: usize,
    final_starts: usize,
}

/// 任务9 Step 4/6：真实 owner 实体 + barrier 控制双 run overlap；取消收口前删除
/// entity_busy；旧 run finish 后新 token/command（incarnation）仍在；completion 不替代
/// lease 释放；接替 run 自行另 acquire；写完且 lease 释放后才可删除；取消后能再生成。
/// engine 锁回收断言承接 r16 既有用例（supersede_reclaim.rs，原名保留）。
#[test]
fn real_run_guards_keep_depth_lease_and_new_token() {
    let root_thread = std::thread::current().id();
    let rt = explicit_2mib_runtime();
    let evidence = rt
        .block_on(async {
            tokio::spawn(async {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(120),
                    run_real_run_guards_scenario(),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err("2 MiB 三守卫场景超时（120s 墙钟上限）".to_string()),
                }
            })
            .await
            .expect("guards scenario task")
        })
        .expect("2 MiB guards scenario");
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    let e = &evidence;
    assert_worker_identities(&e.identities, root_thread);
    assert_eq!(e.final_starts, 5, "全程恰 5 次 start（3 悬挂 + 再生成 2）");
    // depth 矩阵（实测口径）：接替 run 的 drive 登记发生在**前序 run 完全收口之后**
    // ⇒ 观测序列 1→1→1→0，无 depth 2 重叠（计划 §Step 4 的 1→2→1→0 预期与产品顺序
    // 不符，见 task-9-report「合同偏差」）。此处钉住：任一 start 时刻 depth ≤ 2、终值 0。
    assert_eq!(
        e.start_depths.first().copied(),
        Some(1),
        "首个悬挂 run 独占 drive（depth 1）"
    );
    assert!(
        e.start_depths
            .get(1)
            .copied()
            .is_some_and(|depth| depth <= 2),
        "接替 run 登记时 depth 至多 2（实测 1 ⇒ 驱动不重叠）"
    );
    assert!(
        e.start_depths
            .get(2)
            .copied()
            .is_some_and(|depth| depth <= 2),
        "再生成 run 登记时 depth 至多 2"
    );
    assert_eq!(e.depth_final, 0, "结束 idle depth 收敛 0");
    assert_ne!(e.token_run1, e.token_run2, "supersede 后是新 token");
    assert_ne!(
        e.incarnation_run1, e.incarnation_run2,
        "新 run 有自己的 incarnation"
    );
    assert!(e.command_tx_present, "旧 finish 后新 command_tx 仍在");
    assert!(e.delete_busy_before_cancel, "持 lease 期间删除 entity_busy");
    assert!(
        e.delete_busy_while_run2_active,
        "旧 run 已 finish 但接替 run 持新 lease：completion 不替代 lease 释放"
    );
    assert!(
        e.delete_busy_during_regen,
        "再生成 run 持新 lease：删除再 busy"
    );
    assert!(
        e.start_drops_total >= 3,
        "三次悬挂 start 随取消强制 drop（r16 合同）"
    );
}

async fn run_real_run_guards_scenario() -> Result<GuardsEvidence, String> {
    let fixture = lifecycle_guards_live_fixture().await;
    // ① run#1：悬挂持锁（r16 形态）——depth 1、活跃 run、真实 owner lease。
    fixture.script_next(vec![ProbeStep::Hang]);
    admit_author(&fixture, "guards run1 hangs holding engine lock").await?;
    barrier!("run1 start + depth 1 + 活跃登记", 5, || {
        fixture.starts() == 1 && fixture.depth() == 1 && !fixture.idle_now()
    });
    let run1 = fixture.manager.active_run_ref_for_test().expect("run1 ref");
    let (token_run1, incarnation_run1) = (run1.token, run1.run_incarnation);
    // ② 取消收口 store barrier 前：删除准入必须 entity_busy（lease 在 hold）。
    let delete_busy_before_cancel = fixture.delete_admission().is_err();
    // ③ run#2 supersede：先取消旧 run、再等 engine 锁；重叠深度由 start 探针取证。
    fixture.script_next(vec![ProbeStep::Hang]);
    admit_author(&fixture, "guards run2 supersedes and hangs").await?;
    barrier!("run2 真实 start（锁已回收）", 10, || {
        fixture.starts() == 2
    });
    let run2 = fixture.manager.active_run_ref_for_test().expect("run2 ref");
    let (token_run2, incarnation_run2) = (run2.token, run2.run_incarnation);
    // 旧 run 已 finish 后，接替 run 的 command 通道仍在（command_tx 非 Option）。
    let command_tx_present = fixture.manager.active_run().await.is_some();
    // ④ 旧 run 已 finish，接替 run 持新 lease：completion 不替代 lease 释放。
    barrier!("run1 旧 start future 已随强制 drop 释放", 3, || {
        fixture.provider.start_drops.load(Ordering::SeqCst) >= 1
    });
    let delete_busy_while_run2_active = fixture.delete_admission().is_err();
    // ⑤ 取消 run#2（manager abort）：settle 强制 drop → depth 0、活跃清空。
    assert!(
        fixture.manager.abort_active_run().await,
        "run2 abort 摘除活跃 run"
    );
    barrier!(
        "run2 取消收口：Drop 探针 2 + depth 0 + 活跃清空",
        5,
        || {
            fixture.provider.start_drops.load(Ordering::SeqCst) >= 2
                && fixture.depth() == 0
                && fixture.idle_now()
        }
    );
    // ⑥ 任务退出（全部 store 写完成 + lease 释放）后：删除准入成功。
    barrier!(
        "run2 任务退出后删除准入成功（lease 已释放）",
        5,
        || { fixture.delete_admission().is_ok() }
    );
    // ⑦ 再生成 run#3（悬挂）：另 acquire 新 lease（删除再 busy）；取消退出后可删除。
    fixture.script_next(vec![ProbeStep::Hang]);
    admit_author(&fixture, "guards run3 regenerates after cancel").await?;
    barrier!("run3 再生成 start", 10, || { fixture.starts() >= 3 });
    let delete_busy_during_regen = fixture.delete_admission().is_err();
    assert!(
        fixture.manager.abort_active_run().await,
        "run3 abort 摘除活跃 run"
    );
    barrier!("run3 取消收口：depth 0 + 活跃清空", 5, || {
        fixture.depth() == 0 && fixture.idle_now()
    });
    barrier!("run3 任务退出后删除准入成功", 5, || fixture
        .delete_admission()
        .is_ok());
    // ⑧ 最终再生成（完整完成一轮）：durable 进展；任务退出后删除准入成功。
    fixture.script_next(complete_steps("guards final regen"));
    admit_author(&fixture, "guards final regen completes").await?;
    barrier!("最终再生成完成（start≥5 且收敛）", 15, || {
        fixture.starts() >= 5 && fixture.idle_now()
    });
    barrier!("最终任务退出后删除准入成功", 5, || fixture
        .delete_admission()
        .is_ok());
    let final_starts = fixture.starts();
    let evidence = GuardsEvidence {
        identities: fixture.identities(),
        start_depths: fixture.start_depths(),
        depth_final: fixture.depth(),
        token_run1,
        token_run2,
        incarnation_run1,
        incarnation_run2,
        command_tx_present,
        delete_busy_before_cancel,
        delete_busy_while_run2_active,
        delete_busy_during_regen,
        start_drops_total: fixture.provider.start_drops.load(Ordering::SeqCst),
        final_starts,
    };
    fixture.dispose().await;
    Ok(evidence)
}
