//! S0（任务1）：从 provider_run_events.rs 整体迁入的 r16 现场死锁回归
//! （原文件 1120–1257 行，测试名/HangingStartProvider/注释/1s 断言原样
//! 保留；迁移仅为腾出 events 注册行，行为零变化——设计 §6.4）。

use super::*;

// ---------------------------------------------------------------------------
// r16 现场死锁回归:spawned run 任务持有 engine 锁期间悬停在 select 之外的
// 裸 await(现场形态:provider 启动/SC 链),supersede 取消旧 run 后,新 run
// 的 engine.lock().await 永久排队,WS/HTTP 全部冻结。
// ---------------------------------------------------------------------------

struct HangingStartProvider {
    starts: Arc<AtomicUsize>,
    /// S2（任务2）旧 start Drop 探针：drive 被取消强制 drop 时，悬停中的
    /// provider start future 一并 Drop（r16 合同——guard 释放必须连带取消
    /// 在途 provider 启动链，不许「内部 spawn child 后仅丢 JoinHandle」）。
    start_drops: Arc<AtomicUsize>,
}

/// S2 旧 start Drop 探针载荷：随 start future 存活，drop 即计数。
struct StartDropProbe(Arc<AtomicUsize>);

impl Drop for StartDropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingStartProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // 已计 start 后悬挂:模拟 provider 启动链卡死(select 外裸 await,
        // 取消 token 不可见)。
        self.starts.fetch_add(1, Ordering::SeqCst);
        // S2：Drop 探针随本 future 存活——drive 被强制 drop 时在此计数。
        let _drop_probe = StartDropProbe(self.start_drops.clone());
        std::future::pending::<()>().await;
        unreachable!("hanging provider start never resolves")
    }

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

#[tokio::test]
async fn supersede_reclaims_engine_lock_from_run_hanging_outside_select() {
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
    let (engine_tx, engine_rx) = mpsc::channel(8);
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle,
        engine_tx.clone(),
        WorkspaceSession::from_record(session_record.clone()),
    )));
    let starts = Arc::new(AtomicUsize::new(0));
    // S2（任务2）：旧 start Drop 探针（设计 §6.4 r16 行增强）。
    let start_drops = Arc::new(AtomicUsize::new(0));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(HangingStartProvider {
            starts: starts.clone(),
            start_drops: start_drops.clone(),
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
    let manager = run_context.manager.clone();
    let forward =
        spawn_engine_event_forward_task(engine_rx, outbound_tx.clone(), Some(run_context.clone()));

    // run#1:handler 路径启动,计 start 后悬停在 engine 锁内。
    spawn_provider_run_from_handler(
        run_context.clone(),
        ProviderRunKind::Author {
            content: "first run hangs in provider start".to_string(),
        },
        outbound_tx.clone(),
    )
    .await
    .expect("first run starts");
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if starts.load(Ordering::SeqCst) == 1 && manager.active_run().await.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first run must register and hang holding the engine lock");

    // supersede:r16 现场同款(取消旧 run 后新 run 排队 engine 锁)。整体有界:
    // 修前 spawn 本身悬在 engine.lock().await(现场死锁形态),修后 1s 内
    // 完成 supersede 且新 run 计 start。
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        spawn_provider_run_from_handler(
            run_context.clone(),
            ProviderRunKind::Author {
                content: "supersede must reclaim the engine lock".to_string(),
            },
            outbound_tx.clone(),
        )
        .await
        .expect("supersede run starts");
        loop {
            if starts.load(Ordering::SeqCst) >= 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("supersede cancel must reclaim the engine lock so the new run starts");

    // S2（任务2）：旧 start Drop 探针——新 run 已取到 engine 锁并真实
    // start（starts>=2），证明旧 drive（含悬停的旧 provider start future）
    // 已被取消兜底强制 drop，而非留作孤儿继续持锁。
    assert!(
        start_drops.load(Ordering::SeqCst) >= 1,
        "旧 run 的 provider start future 必须随 drive 强制 drop 一并 Drop（r16 合同）"
    );

    drop(engine_tx);
    forward.abort();
    let _ = forward.await;
}

// ---------------------------------------------------------------------------
// S2（任务2）：取消 settle 两条分支的 barrier 精确覆盖（不用 sleep 猜
// 时序——设计 §3.4/§6.4）。grace 分支：drive 在 250ms 窗口内自行收口
// （自有尾部释放 engine guard）；forced 分支：drive 悬停在窗口外，超时
// drop 必须先于兜底重取锁释放 engine guard——兜底经
// fail_cancelled_human_gate_revision_turn 重取 engine 锁，若 drop 未先
// 释放 guard 则死锁并被外层超时捕获。
// ---------------------------------------------------------------------------

/// settle 分支用例夹具：真实 engine/manager/生命周期存储（provider 注册
/// 表空置——本用例直驱 settle helper，不 spawn run）。
#[allow(clippy::type_complexity)]
fn cancel_settle_fixture() -> (
    Arc<Mutex<WorkspaceEngine>>,
    Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    mpsc::Sender<OutboundControl>,
    String,
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
    let (engine_tx, _engine_rx) = mpsc::channel(64);
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle,
        engine_tx,
        WorkspaceSession::from_record(session_record.clone()),
    )));
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(ProviderRegistry::new()),
        engine.clone(),
        workspace_runs,
        session_record.id.clone(),
        app_paths,
        session_record,
    );
    let (outbound_tx, _outbound_rx) = mpsc::channel::<OutboundControl>(64);
    (
        engine,
        run_context.manager.clone(),
        outbound_tx,
        run_context.session_id,
    )
}

#[tokio::test]
async fn cancel_settle_grace_and_forced_drop_release_engine() {
    // —— 分支一（grace）：取消后 drive 在 250ms 窗口内自行收口 ——
    {
        let (engine, manager, outbound_tx, session_id) = cancel_settle_fixture();
        let run_cancel = CancellationToken::new();
        // barrier（2 方）：drive 与本测试会合——证明 drive 已进入「取消后
        // 窗口内自收口」路径，时机由 barrier 决定而非 sleep。
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let tail_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let guard_released = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let drive = {
            let engine = engine.clone();
            let run_cancel = run_cancel.clone();
            let barrier = barrier.clone();
            let tail_ran = tail_ran.clone();
            let guard_released = guard_released.clone();
            async move {
                // 与真实 drive 同构：按值持有 engine guard。
                let engine_guard = engine.lock().await;
                run_cancel.cancelled().await;
                barrier.wait().await;
                // drive 自有尾部：先释放 engine guard，再收口尾部标记。
                drop(engine_guard);
                guard_released.store(true, Ordering::SeqCst);
                tail_ran.store(true, Ordering::SeqCst);
            }
        };
        run_cancel.cancel();
        let settle_engine = engine.clone();
        let settle_cancel = run_cancel.clone();
        let settle_outbound = outbound_tx.clone();
        let settle_session_id = session_id.clone();
        let settle_task = tokio::spawn(async move {
            crate::web::workspace_ws_handler::run::drive_until_cancelled_settles(
                drive,
                &settle_cancel,
                None,
                &settle_engine,
                &settle_outbound,
                &settle_session_id,
                "run-grace",
                &manager,
                9901,
            )
            .await;
        });
        // 与 drive 会合：窗口内放行其自收口。
        barrier.wait().await;
        tokio::time::timeout(std::time::Duration::from_secs(1), settle_task)
            .await
            .expect("grace settle must return promptly（drive 窗口内自收口）")
            .expect("grace settle task must not panic");
        assert!(
            tail_ran.load(Ordering::SeqCst),
            "grace 分支 drive 自有尾部必须执行"
        );
        assert!(
            guard_released.load(Ordering::SeqCst),
            "grace 分支 engine guard 必须由 drive 自有尾部释放"
        );
        tokio::time::timeout(std::time::Duration::from_millis(500), async {
            let _ = engine.lock().await;
        })
        .await
        .expect("engine lock must be re-acquirable after grace settle");
    }

    // —— 分支二（forced）：窗口超时 drop drive，guard 释放必须先于兜底
    //    重取锁（Some(turn) 使兜底真实走 engine.lock()——顺序错了即死锁）——
    {
        let (engine, manager, outbound_tx, session_id) = cancel_settle_fixture();
        let run_cancel = CancellationToken::new();
        let drive = {
            let engine = engine.clone();
            async move {
                // r16 现场形态：按值持有 engine guard，悬停在 select 外裸
                // await（永不完成）。
                let _engine_guard = engine.lock().await;
                std::future::pending::<()>().await;
            }
        };
        run_cancel.cancel();
        let settle_engine = engine.clone();
        let settle_cancel = run_cancel.clone();
        let settle_outbound = outbound_tx.clone();
        let settle_session_id = session_id.clone();
        let settle_task = tokio::spawn(async move {
            crate::web::workspace_ws_handler::run::drive_until_cancelled_settles(
                drive,
                &settle_cancel,
                // 不存在的 turn：fail_cancelled_* 先 engine.lock() 再幂等跳过
                // （Ok(false) 不回显帧）——锁未先被释放则死锁、被超时捕获。
                Some("turn-missing-order-probe".to_string()),
                &settle_engine,
                &settle_outbound,
                &settle_session_id,
                "run-forced",
                &manager,
                9902,
            )
            .await;
        });
        // 窗口 250ms + 兜底收口，整体必须显著有界。
        tokio::time::timeout(std::time::Duration::from_secs(2), settle_task)
            .await
            .expect("forced-drop settle must stay bounded（250ms 窗口 + 兜底收口）")
            .expect("forced-drop settle task must not panic");
        tokio::time::timeout(std::time::Duration::from_millis(500), async {
            let _ = engine.lock().await;
        })
        .await
        .expect("engine lock must be re-acquirable after forced drop");
    }
}
