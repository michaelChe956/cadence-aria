use super::journal::{EventJournal, JOURNAL_HARD_CAP, JOURNAL_TAIL};
use super::{WorkspaceSessionManager, WorkspaceSessionRegistry};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::lifecycle_store::CreateStorySpecInput;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::product::workspace_engine::ProviderRunKind;
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;
use crate::web::workspace_ws_handler::OutboundControl;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn resubscribe_snapshot_includes_connection_id_before_socket_decoration() {
    let manager = WorkspaceSessionManager::test_fixture("session_resubscribe_connection_id");
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("connection", outbound_tx.clone());

    manager.resubscribe(&outbound_tx, "connection", 1).await;

    let Some(OutboundControl::Text(snapshot)) = outbound_rx.recv().await else {
        panic!("resubscribe should send a snapshot");
    };
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot).expect("snapshot JSON");
    assert_eq!(snapshot["type"], "session_state");
    assert_eq!(snapshot["connection_id"], "connection");
}

#[tokio::test]
async fn start_run_supersedes_active_run_with_token_equality_guard() {
    let manager = WorkspaceSessionManager::test_fixture("session_arb");
    let (_id_a, token_a, cancel_a, _rx_a, _) = manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("run a");
    assert!(manager.active_run().await.is_some());

    let (_id_b, token_b, _cancel_b, _rx_b, _) = manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("run b");
    tokio::time::timeout(std::time::Duration::from_millis(500), cancel_a.cancelled())
        .await
        .expect("run a token cancelled by supersede");
    assert_ne!(token_a, token_b);
    assert_eq!(manager.active_run().await.unwrap().token, token_b);

    manager.finish_run(token_a).await;
    assert_eq!(manager.active_run().await.unwrap().token, token_b);

    manager.finish_run(token_b).await;
    assert!(manager.active_run().await.is_none());
}

/// socket supersede 先取消 R1 后会等待 engine 锁；等待期间内部 relay 可登记 R2。
/// socket 恢复登记 R3 时必须取消 R2，否则 R2 将脱离 manager 继续驱动同一 engine。
#[tokio::test]
async fn socket_supersede_resuming_after_relay_start_cancels_relay_run() {
    let manager = WorkspaceSessionManager::test_fixture("session_orphaned_relay_run");
    let (outbound_tx, _outbound_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("socket-holder", outbound_tx);
    let lease_epoch = manager
        .lease_epoch_for_connection("socket-holder")
        .expect("registered holder lease epoch");

    let (_r1_id, _r1_token, r1_cancel, _r1_rx, _) = manager
        .start_run(ProviderRunKind::ReviewOnly, Some("r1".to_string()))
        .await
        .expect("start R1");
    manager
        .abort_active_run_from_attachment(Some("socket-holder"), Some(lease_epoch))
        .await
        .expect("socket supersede cancels R1 before engine lock");
    tokio::time::timeout(std::time::Duration::from_millis(500), r1_cancel.cancelled())
        .await
        .expect("socket supersede cancelled R1");

    let (_r2_id, r2_token, r2_cancel, _r2_rx, _) = manager
        .start_run(
            ProviderRunKind::WorkItemPlanRevision { feedback: None },
            Some("r2".to_string()),
        )
        .await
        .expect("relay starts R2 while socket waits for engine lock");
    let (_r3_id, r3_token, _r3_cancel, _r3_rx, _) = manager
        .start_run_from_attachment(
            Some("socket-holder"),
            Some(lease_epoch),
            Some("r3".to_string()),
        )
        .await
        .expect("socket resumes and starts R3");

    tokio::time::timeout(std::time::Duration::from_millis(500), r2_cancel.cancelled())
        .await
        .expect("R3 must cancel relay R2 instead of orphaning it");
    assert_eq!(
        manager.active_run().await.expect("active R3").token,
        r3_token,
        "R3 remains the single manager-owned active run"
    );
    manager.finish_run(r2_token).await;
    assert_eq!(
        manager
            .active_run()
            .await
            .expect("R3 survives R2 finish")
            .token,
        r3_token
    );
    manager.abort_active_run().await;
}

#[tokio::test]
async fn abort_active_run_cancels_token_and_clears_state() {
    let manager = WorkspaceSessionManager::test_fixture("session_abort");
    let (_, _token, cancel, _rx, _) = manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("run");

    assert!(manager.abort_active_run().await);
    tokio::time::timeout(std::time::Duration::from_millis(500), cancel.cancelled())
        .await
        .expect("abort cancels token");
    assert!(manager.active_run().await.is_none());
    assert!(
        !manager.abort_active_run().await,
        "无 run 时 abort 返回 false"
    );
}

#[tokio::test]
async fn registry_get_or_create_runs_factory_once_under_concurrency() {
    let registry = WorkspaceSessionRegistry::default();
    let factory_calls = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for _ in 0..16 {
        let registry = registry.clone();
        let factory_calls = factory_calls.clone();
        handles.push(tokio::spawn(async move {
            registry
                .get_or_create("session_x", move || {
                    let factory_calls = factory_calls.clone();
                    async move {
                        factory_calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        Ok(WorkspaceSessionManager::test_fixture("session_x"))
                    }
                })
                .await
        }));
    }

    let first = handles.remove(0).await.expect("task").expect("manager");
    for handle in handles {
        let manager = handle.await.expect("task").expect("manager");
        assert!(Arc::ptr_eq(&first, &manager));
    }
    assert_eq!(factory_calls.load(Ordering::SeqCst), 1);
    assert_eq!(registry.session_ids().await, vec!["session_x".to_string()]);
}

#[tokio::test]
async fn registry_creates_different_sessions_without_waiting_for_slow_factory() {
    let registry = WorkspaceSessionRegistry::default();
    let factory_entered = Arc::new(tokio::sync::Notify::new());
    let factory_release = Arc::new(tokio::sync::Notify::new());
    let slow_registry = registry.clone();
    let slow_entered = factory_entered.clone();
    let slow_release = factory_release.clone();
    let slow = tokio::spawn(async move {
        slow_registry
            .get_or_create("session_slow", move || async move {
                slow_entered.notify_one();
                slow_release.notified().await;
                Ok(WorkspaceSessionManager::test_fixture("session_slow"))
            })
            .await
    });
    factory_entered.notified().await;

    let fast = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        registry.get_or_create("session_fast", || async {
            Ok(WorkspaceSessionManager::test_fixture("session_fast"))
        }),
    )
    .await
    .expect("慢 factory 不得阻塞其他 session")
    .expect("fast manager");
    assert_eq!(fast.session_id, "session_fast");

    factory_release.notify_one();
    slow.await.expect("slow task").expect("slow manager");
    assert_eq!(
        registry.session_ids().await,
        vec!["session_fast".to_string(), "session_slow".to_string()]
    );
}

#[tokio::test]
async fn registry_session_ids_are_sorted_and_remove_is_idempotent() {
    let registry = WorkspaceSessionRegistry::default();

    registry.remove("missing").await;
    assert!(registry.session_ids().await.is_empty());
}

/// `finish_run` 与连接关闭可任意交错：仅匹配的 token 可清理活动 run，关闭只会
/// 摘除 attachment，最终状态必须可回收。
#[tokio::test]
async fn finish_run_and_connection_close_interleaving_clears_run_and_attachment() {
    for _ in 0..100 {
        let manager = WorkspaceSessionManager::test_fixture("session_finish_close");
        let (outbound_tx, _outbound_rx) = mpsc::channel::<OutboundControl>(1);
        manager.register_attachment("connection", outbound_tx);
        let (_run_id, token, _cancel, _command_rx, _node_id) = manager
            .start_run(ProviderRunKind::ReviewOnly, None)
            .await
            .expect("start run");

        tokio::join!(
            manager.finish_run(token),
            manager.handle_connection_closed("connection")
        );

        let cleared = tokio::time::timeout(std::time::Duration::from_millis(500), async {
            loop {
                if manager.active_run().await.is_none() && manager.is_recyclable() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(
            cleared.is_ok(),
            "交错后不得遗留 active_run 或 attachment，manager 必须可回收"
        );
    }
}

/// 防止 journal 因窗口边界、终态裁剪或 hard cap 而对 cursor 做不诚实回放。
#[test]
fn journal_replays_window_and_degrades_when_truncated() {
    let mut journal = EventJournal {
        run_active: true,
        ..EventJournal::default()
    };
    for seq in 1..=10 {
        journal.push(seq, format!("{{\"event_seq\":{seq}}}"));
    }

    let replay = journal
        .replay_after(3)
        .expect("cursor within journal window");
    assert_eq!(replay.len(), 7);
    assert!(replay[0].contains("\"event_seq\":4"));
    assert!(
        journal
            .replay_after(10)
            .expect("cursor at latest sequence")
            .is_empty()
    );
    let from_attach_baseline = journal
        .replay_after(0)
        .expect("attach baseline precedes first event");
    assert_eq!(
        from_attach_baseline.len(),
        10,
        "baseline replays the full first window"
    );

    journal.mark_run_terminal();
    assert_eq!(journal.entries.len(), 10);
    for seq in 11..=JOURNAL_TAIL + 20 {
        journal.push(seq, String::new());
    }
    assert_eq!(journal.entries.len(), JOURNAL_TAIL as usize);
    assert!(journal.oldest_seq().expect("tail has entries") > 11);
    assert!(
        journal.replay_after(11).is_none(),
        "tail window moved left edge"
    );

    journal.run_active = true;
    for seq in 2_000..=JOURNAL_HARD_CAP + 2_000 {
        journal.push(seq, String::new());
    }
    assert!(journal.truncated, "hard cap must record irreversible loss");
    assert!(
        journal.replay_after(JOURNAL_HARD_CAP + 2_000).is_none(),
        "truncated journal must require a snapshot even at latest cursor"
    );
}

/// 防止 router 退化为单 attachment 发送，或在每个连接上分配不同的 event_seq。
#[tokio::test]
async fn manager_broadcast_stamps_monotonic_seq_per_session() {
    let manager = WorkspaceSessionManager::test_fixture("session_event_seq");
    let (tx_a, mut rx_a) = mpsc::channel(16);
    let (tx_b, mut rx_b) = mpsc::channel(16);
    manager.attach("conn-a", tx_a).await;
    manager.attach("conn-b", tx_b).await;
    for status in [
        crate::web::workspace_ws_types::WsProviderStatus::Starting,
        crate::web::workspace_ws_types::WsProviderStatus::Running,
        crate::web::workspace_ws_types::WsProviderStatus::Completed,
    ] {
        manager.broadcast_test_event(status).await;
    }

    let received_a = (0..3)
        .map(
            |_| match rx_a.try_recv().expect("attachment A receives broadcast") {
                OutboundControl::Text(json) => serde_json::from_str::<serde_json::Value>(&json)
                    .expect("stamped broadcast json")["event_seq"]
                    .as_u64()
                    .expect("event_seq")
                    .to_owned(),
                control => panic!("unexpected broadcast control: {control:?}"),
            },
        )
        .collect::<Vec<_>>();
    let received_b = (0..3)
        .map(
            |_| match rx_b.try_recv().expect("attachment B receives broadcast") {
                OutboundControl::Text(json) => serde_json::from_str::<serde_json::Value>(&json)
                    .expect("stamped broadcast json")["event_seq"]
                    .as_u64()
                    .expect("event_seq")
                    .to_owned(),
                control => panic!("unexpected broadcast control: {control:?}"),
            },
        )
        .collect::<Vec<_>>();
    assert_eq!(received_a, vec![1, 2, 3]);
    assert_eq!(
        received_b, received_a,
        "fan-out must preserve stamped event identity"
    );
}

/// REQ-WCR-04：某个 attachment 的有界队列满时，仅该连接被降级；其他 attachment
/// 继续逐事件接收，router 从不等待慢连接。
#[tokio::test]
async fn manager_degrades_only_full_attachment_without_backpressure() {
    let manager = WorkspaceSessionManager::test_fixture("session_slow_attachment");
    let (slow_tx, mut slow_rx) = mpsc::channel(1);
    let (fast_tx, mut fast_rx) = mpsc::channel(8);
    manager.attach("slow", slow_tx).await;
    manager.attach("fast", fast_tx).await;

    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Starting)
        .await;
    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Running)
        .await;
    assert!(
        manager.attachment_is_degraded("slow"),
        "满队列 attachment 必须被标为 degraded，之后不再接收直播帧"
    );

    let first_slow = slow_rx.recv().await.expect("slow attachment first event");
    assert!(matches!(first_slow, OutboundControl::Text(json) if json.contains("provider_status")));
    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Completed)
        .await;
    assert!(
        matches!(slow_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "满队列时重同步控制帧允许被一次性丢弃，降级 attachment 不得继续直播"
    );

    let fast_sequences = (0..3)
        .map(|_| {
            match fast_rx
                .try_recv()
                .expect("fast attachment receives every event")
            {
                OutboundControl::Text(json) => serde_json::from_str::<serde_json::Value>(&json)
                    .expect("broadcast JSON")["event_seq"]
                    .as_u64()
                    .expect("event sequence"),
                other => panic!("unexpected fast attachment control: {other:?}"),
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(fast_sequences, vec![1, 2, 3]);
}
// F-1 回归（P2 终审 p38-p2-final-review-k3.md §6）：「无活动 run 且无订阅者」窗口的
// 接力 ProviderRunRequested 不得被 continue 丢弃——必须以 throwaway outbound 通道
// spawn（恢复链 recover_outline_run 同款），run 持续至真实终态。修复前本测试超时必红。
#[tokio::test]
async fn provider_run_requested_without_attachments_spawns_throwaway_run() {
    use crate::cross_cutting::provider_adapter::ProviderAdapterError;
    use crate::cross_cutting::streaming_provider::{
        ProviderEvent, ProviderSession, StreamChunk, StreamingProviderAdapter,
        StreamingProviderInput,
    };
    use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
    use crate::product::models::ProviderName;
    use crate::product::models::WorkspaceType;

    // 与 provider_run_events.rs:467-497 PendingStartProvider 同款：start 计数并把事件
    // 端扣在手里（run 挂起不完成），保证 active_run 断言窗口确定。
    struct HeldStartProvider {
        starts: Arc<AtomicUsize>,
        event_tx: StdMutex<Option<mpsc::Sender<ProviderEvent>>>,
    }
    #[async_trait::async_trait]
    impl StreamingProviderAdapter for HeldStartProvider {
        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            let (event_tx, event_rx) = mpsc::channel(1);
            *self
                .event_tx
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(event_tx);
            let (command_tx, _command_rx) = mpsc::channel(1);
            Ok(ProviderSession {
                native_session_id: None,
                events: event_rx,
                commands: command_tx,
            })
        }
        async fn run_streaming(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
            _cancel: CancellationToken,
        ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
            unreachable!("workspace runs use start")
        }
    }

    let root = tempfile::tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(app_paths.clone())
        .create(CreateProjectInput {
            name: "throwaway run test".to_string(),
            description: None,
        })
        .expect("project");
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: project.id.clone(),
            name: "fixture repository".to_string(),
            path: root.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: Some("fake".to_string()),
            idempotency_key: "throwaway-run-test-repository".to_string(),
        })
        .expect("repository");
    let issue = IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: Some(repository.id.clone()),
            logical_codebase_id: None,
            title: "throwaway run issue".to_string(),
            description: None,
            change_id: None,
        })
        .expect("issue");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: project.id.clone(),
            issue_id: issue.id.clone(),
            repository_id: repository.id,
            title: "throwaway run story".to_string(),
            aggregate_codebase: None,
        })
        .expect("story");
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: project.id,
            issue_id: issue.id,
            entity_id: story.id,
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("workspace session");
    let starts = Arc::new(AtomicUsize::new(0));
    let mut provider_registry = ProviderRegistry::new();
    provider_registry.register(
        ProviderName::ClaudeCode,
        Arc::new(HeldStartProvider {
            starts: starts.clone(),
            event_tx: StdMutex::new(None),
        }),
    );
    let manager = WorkspaceSessionManager::create(
        &WebAppState::with_provider_registry(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            provider_registry,
        ),
        &session_record.id,
    )
    .await
    .expect("create workspace session manager");
    manager
        .engine()
        .lock()
        .await
        .request_provider_run(ProviderRunKind::Author {
            content: "relay without subscribers".to_string(),
        })
        .await;

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if starts.load(Ordering::SeqCst) >= 1 && manager.active_run().await.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("无订阅者时接力 run 必须仍被 spawn（F-1：修复前事件被 continue 丢弃，本断言超时必红）");
}
