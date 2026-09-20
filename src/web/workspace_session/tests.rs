use super::journal::{EventJournal, JOURNAL_HARD_CAP, JOURNAL_TAIL};
use super::{WorkspaceSessionManager, WorkspaceSessionRegistry};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::lifecycle_store::CreateStorySpecInput;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{SingleCandidatePhase, WorkspaceType};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason, WorkItemPlanFlowKind};
use crate::product::workspace_engine::{EngineEvent, ProviderRunKind};
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;
use crate::web::workspace_ws_handler::OutboundControl;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;
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

/// F-06 锚：cursor 重订阅必须走 journal 增量回放（replay_after），只发 cursor 之后
/// 的增量帧、不发全量 session_state 基线；重订阅完成后直播事件仍可达。
#[tokio::test]
async fn resubscribe_replays_journal_events_without_full_baseline() {
    use crate::web::workspace_ws_types::WsProviderStatus;

    let manager = WorkspaceSessionManager::test_fixture("session_resubscribe_replay");
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<OutboundControl>(16);
    manager.register_attachment("connection", outbound_tx.clone());

    // 三条广播进 journal（attachment 仍 pending，不接收直播）
    manager
        .broadcast_test_event(WsProviderStatus::Running)
        .await; // seq 1
    manager
        .broadcast_test_event(WsProviderStatus::Aborted)
        .await; // seq 2
    manager
        .broadcast_test_event(WsProviderStatus::Running)
        .await; // seq 3

    manager.resubscribe(&outbound_tx, "connection", 1).await;

    let mut replayed = Vec::new();
    while let Ok(control) = outbound_rx.try_recv() {
        let OutboundControl::Text(json) = control else {
            panic!("cursor resubscribe must only send text frames");
        };
        replayed.push(json);
    }
    assert_eq!(
        replayed.len(),
        2,
        "cursor 回放只发增量帧，不得附带全量基线：{replayed:?}"
    );
    for (index, json) in replayed.iter().enumerate() {
        let value: serde_json::Value = serde_json::from_str(json).expect("replay JSON");
        assert_eq!(
            value["type"], "provider_status",
            "回放帧是原广播事件而非 session_state 基线"
        );
        assert_eq!(
            value["event_seq"],
            serde_json::json!(index as u64 + 2),
            "回放只含 cursor 之后的事件"
        );
    }

    // 重订阅后连接已激活：直播事件继续到达（事件驱动更新仍达）
    manager
        .broadcast_test_event(WsProviderStatus::Completed)
        .await;
    match outbound_rx.recv().await {
        Some(OutboundControl::Text(json)) => {
            let value: serde_json::Value = serde_json::from_str(&json).expect("live JSON");
            assert_eq!(value["type"], "provider_status");
            assert_eq!(value["event_seq"], serde_json::json!(4));
        }
        other => panic!("live event after cursor resubscribe expected, got {other:?}"),
    }
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

/// REQ-WCR-04：满队列 attachment 会被暂时降级；后续广播在队列恢复容量后必须以
/// 最新 session_state 基线恢复该连接，不能依赖可能已经丢失的 ResyncRequired 控制帧。
#[tokio::test]
async fn manager_recovers_degraded_attachment_with_session_state_baseline() {
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
        "满队列 attachment 必须被标为 degraded"
    );

    let first_slow = slow_rx.recv().await.expect("slow attachment first event");
    assert!(matches!(first_slow, OutboundControl::Text(json) if json.contains("provider_status")));
    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Completed)
        .await;
    assert!(
        !manager.attachment_is_degraded("slow"),
        "队列恢复容量后必须摘除 degraded 标记"
    );
    assert!(
        matches!(slow_rx.recv().await, Some(OutboundControl::Text(json)) if serde_json::from_str::<serde_json::Value>(&json).expect("recovery baseline JSON")["type"] == "session_state"),
        "恢复的 attachment 必须先收到 session_state 基线"
    );

    manager
        .broadcast_test_event(crate::web::workspace_ws_types::WsProviderStatus::Completed)
        .await;
    assert!(
        matches!(slow_rx.recv().await, Some(OutboundControl::Text(json)) if json.contains("provider_status")),
        "恢复后的 attachment 必须重新接收后续直播帧"
    );

    let fast_sequences = (0..4)
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
    assert_eq!(fast_sequences, vec![1, 2, 3, 4]);
}

/// F-09：门开启时，已经激活的两个 attachment 都必须收到全量 session_state；否则
/// 老 tab 的 phase/snapshot 仍停留在门开启前，新 tab 却会因 attach 快照而正确。
#[tokio::test]
async fn human_gate_open_rebuilds_session_state_for_existing_attachments() {
    let manager =
        WorkspaceSessionManager::test_fixture_with_event_router("session_human_gate_opened");
    let (existing_tx, mut existing_rx) = mpsc::channel(8);
    let (other_tx, mut other_rx) = mpsc::channel(8);
    manager.attach("existing", existing_tx).await;
    manager.attach("other", other_tx).await;

    {
        let engine = manager.engine();
        let mut engine = engine.lock().await;
        engine.session.workspace_type = WorkspaceType::WorkItemPlan;
        engine.session.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
        engine.session.single_candidate_phase = Some(SingleCandidatePhase::Approval);
        engine.session.human_gate_snapshot = Some(HumanGateSnapshot {
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: 0,
            manual_repairs_remaining: 1,
            trigger: HumanReason::NativeHumanRequired,
            resumable: false,
        });
    }
    manager
        .engine_tx()
        .send(EngineEvent::HumanGateOpened {
            stage: "human_confirm".to_string(),
        })
        .await
        .expect("send human gate opened event");

    for outbound_rx in [&mut existing_rx, &mut other_rx] {
        let state = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let Some(OutboundControl::Text(json)) = outbound_rx.recv().await else {
                    panic!("attachment closed before session_state");
                };
                let value: serde_json::Value =
                    serde_json::from_str(&json).expect("outbound session state JSON");
                if value["type"] == "session_state" {
                    break value;
                }
            }
        })
        .await
        .expect("human gate open must broadcast a session_state frame");
        assert_eq!(state["single_candidate_phase"], "approval");
        assert!(
            state["human_gate_snapshot"].is_object(),
            "session_state must carry the current human gate snapshot"
        );
    }
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

// ---------------------------------------------------------------------------
// F-24（choice 卡不送达用户）：挂起 provider choice 的可靠重投面
// ---------------------------------------------------------------------------

fn provider_choice_frame(id: &str) -> crate::web::workspace_ws_types::WsOutMessage {
    use crate::web::workspace_ws_types::{ChoiceOption, WsOutMessage};

    WsOutMessage::ChoiceRequest {
        id: id.to_string(),
        prompt: "验收口径歧义需要用户裁定".to_string(),
        options: vec![ChoiceOption {
            id: "option-a".to_string(),
            label: "按全局口径".to_string(),
            description: None,
        }],
        allow_multiple: false,
        allow_free_text: false,
        questions: Vec::new(),
        source: "provider_choice".to_string(),
    }
}

fn frames_contain_choice(frames: &[String], id: &str) -> bool {
    frames.iter().any(|json| {
        let value: serde_json::Value =
            serde_json::from_str(json).unwrap_or(serde_json::Value::Null);
        value["type"] == "choice_request" && value["id"] == id
    })
}

/// F-24 现场锚（0484）：满队列 attachment 降级期间广播的 choice 帧被 try_send 丢弃，
/// 恢复只补 session_state 基线（不含 choice）→ 用户全程在线也永远看不到卡。
/// 修复后：degraded 恢复必须在基线之外补发挂起 choice 帧。
#[tokio::test]
async fn degraded_attachment_recovery_redelivers_pending_provider_choice() {
    use crate::web::workspace_ws_types::WsProviderStatus;

    let manager = WorkspaceSessionManager::test_fixture("session_choice_degraded_redelivery");
    let (slow_tx, mut slow_rx) = mpsc::channel(1);
    manager.attach("slow", slow_tx).await;
    manager
        .broadcast_test_event(WsProviderStatus::Starting)
        .await;
    manager
        .broadcast_test_event(WsProviderStatus::Running)
        .await;
    assert!(
        manager.attachment_is_degraded("slow"),
        "满队列 attachment 必须被标为 degraded"
    );

    manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("active run for pending choice");
    manager.register_pending_choice_frame(provider_choice_frame("choice_degraded_1"));

    let _first = slow_rx.recv().await.expect("first queued event");
    manager
        .broadcast_test_event(WsProviderStatus::Completed)
        .await;
    assert!(!manager.attachment_is_degraded("slow"));

    let mut saw_baseline = false;
    let mut saw_choice = false;
    for _ in 0..4 {
        match tokio::time::timeout(std::time::Duration::from_millis(500), slow_rx.recv()).await {
            Ok(Some(OutboundControl::Text(json))) => {
                let value: serde_json::Value =
                    serde_json::from_str(&json).expect("recovery frame JSON");
                match value["type"].as_str() {
                    Some("session_state") => saw_baseline = true,
                    Some("choice_request") if value["id"] == "choice_degraded_1" => {
                        saw_choice = true;
                    }
                    _ => {}
                }
            }
            _ => break,
        }
    }
    assert!(saw_baseline, "degraded 恢复必须先发 session_state 基线");
    assert!(
        saw_choice,
        "degraded 恢复必须补发挂起 choice 卡（F-24：卡片丢失即 run 永久楔死）"
    );
}

/// F-24：初帧激活与 cursor 重订阅（snapshot 分支）都必须补发活跃 run 的挂起
/// choice——`pending_author_choice_request_message` 只覆盖 TextFallback 面。
#[tokio::test]
async fn attach_and_cursor_resubscribe_redeliver_pending_provider_choices() {
    let manager = WorkspaceSessionManager::test_fixture("session_choice_attach_redelivery");
    manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("active run");
    manager.register_pending_choice_frame(provider_choice_frame("choice_attach_1"));

    let (session_state, _text_fallback) = manager.attached_session_state().await;
    let frames = manager.activate_attachment_with_initial_frames(
        "conn-attach",
        session_state,
        None,
        manager.attach_baseline_seq(),
    );
    assert!(
        frames_contain_choice(&frames, "choice_attach_1"),
        "初帧激活必须补发挂起 provider choice：{frames:?}"
    );

    let (cursor_tx, mut cursor_rx) = mpsc::channel(16);
    manager.register_attachment("conn-cursor", cursor_tx.clone());
    manager.resubscribe(&cursor_tx, "conn-cursor", 9_999).await;
    let mut resubscribed = Vec::new();
    while let Ok(control) = cursor_rx.try_recv() {
        let OutboundControl::Text(json) = control else {
            panic!("resubscribe must only send text frames");
        };
        resubscribed.push(json);
    }
    assert!(
        frames_contain_choice(&resubscribed, "choice_attach_1"),
        "cursor snapshot 重订阅必须补发挂起 provider choice：{resubscribed:?}"
    );
}

// ---------------------------------------------------------------------------
// F-23（跨重启僵尸 run）：manager 创建面恢复
// ---------------------------------------------------------------------------

fn zombie_author_run_node() -> crate::web::workspace_ws_types::TimelineNode {
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::product::models::WorkspaceRolePermissionModes;
    use crate::web::workspace_ws_handler::ProviderName;
    use crate::web::workspace_ws_types::{
        ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
        WorkspaceStage as WsWorkspaceStage,
    };

    TimelineNode {
        node_id: "timeline_node_002".to_string(),
        node_type: TimelineNodeType::AuthorRun,
        agent: Some(ProviderName::ClaudeCode),
        stage: WsWorkspaceStage::Running,
        round: Some(1),
        status: TimelineNodeStatus::Active,
        title: "Author 生成".to_string(),
        summary: None,
        started_at: "2026-09-20T08:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 0,
            permission_modes: WorkspaceRolePermissionModes {
                author: ProviderPermissionMode::Auto,
                reviewer: ProviderPermissionMode::Auto,
            },
        },
        retry: None,
    }
}

/// F-23 现场锚（0482 三小时僵尸）：跨服务器重启后 durable 会话停在
/// running + active author_run 节点，无任何恢复臂触达——manager 创建时必须
/// 落 AbortedByDisconnect 终态并整流回 prepare_context（retry 面可恢复）。
#[tokio::test]
async fn manager_creation_recovers_stale_running_session_after_process_restart() {
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::lifecycle_store::{
        CreateStorySpecInput, CreateWorkspaceSessionInput, LifecycleStore,
    };
    use crate::product::models::WorkspaceType;
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
    use crate::product::workspace_engine::WorkspaceStage;
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;
    use crate::web::workspace_ws_handler::ProviderName;
    use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

    let root = tempfile::tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(app_paths.clone())
        .create(CreateProjectInput {
            name: "f23 zombie recovery".to_string(),
            description: None,
        })
        .expect("project");
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: project.id.clone(),
            name: "f23 repository".to_string(),
            path: root.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: Some("fake".to_string()),
            idempotency_key: "f23-zombie-recovery-repository".to_string(),
        })
        .expect("repository");
    let issue = IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: Some(repository.id.clone()),
            logical_codebase_id: None,
            title: "f23 zombie issue".to_string(),
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
            title: "f23 zombie story".to_string(),
            aggregate_codebase: None,
        })
        .expect("story");
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: project.id.clone(),
            issue_id: issue.id.clone(),
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
    lifecycle
        .save_timeline_nodes(&session_record.id, &[zombie_author_run_node()])
        .expect("persist zombie running timeline");

    let manager = WorkspaceSessionManager::create(
        &WebAppState::with_provider_registry(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            ProviderRegistry::new(),
        ),
        &session_record.id,
    )
    .await
    .expect("create manager for zombie session");

    let engine_arc = manager.engine();
    let engine = engine_arc.lock().await;
    assert_eq!(
        engine.current_stage(),
        WorkspaceStage::PrepareContext,
        "跨重启僵尸必须整流回 prepare_context，不得永久楔死在 running"
    );

    assert!(
        engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_id == "timeline_node_002"
                && node.status == TimelineNodeStatus::Failed),
        "僵尸 author_run 节点必须落失败终态：{:?}",
        engine
            .timeline_nodes
            .iter()
            .map(|node| (node.node_id.clone(), node.status.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::AbortedByDisconnect),
        "恢复必须追加 AbortedByDisconnect 标记节点（retry_interrupted_run 恢复面依赖它）"
    );
}
