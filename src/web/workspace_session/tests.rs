use super::journal::{EventJournal, JOURNAL_HARD_CAP, JOURNAL_TAIL};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{WorkspaceSessionManager, WorkspaceSessionRegistry};
use crate::product::workspace_engine::ProviderRunKind;
use crate::web::workspace_ws_handler::OutboundControl;
use tokio::sync::mpsc;

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
    assert!(journal.replay_after(0).is_none(), "cursor before left edge");

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
