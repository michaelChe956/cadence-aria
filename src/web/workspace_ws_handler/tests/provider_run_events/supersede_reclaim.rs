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
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(HangingStartProvider {
            starts: starts.clone(),
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

    drop(engine_tx);
    forward.abort();
    let _ = forward.await;
}
