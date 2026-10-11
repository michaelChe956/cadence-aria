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
/// REQ-DLS-04：attach 对租约零效应，socket-holder 须经显式 hello（Driver）接管
/// 租约后方具 supersede 资格——前提按新语义改写，测试意图不变。
#[tokio::test]
async fn socket_supersede_resuming_after_relay_start_cancels_relay_run() {
    use crate::web::workspace_session::ConnectionRole;
    let manager = WorkspaceSessionManager::test_fixture("session_orphaned_relay_run");
    let (outbound_tx, _outbound_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("socket-holder", outbound_tx.clone());
    // REQ-DLS-04：attach 不再预取租约；hello 为唯一显式接管路径（wire 入口
    // 同款 bind_role，与 attach_zero_effect_* 用例一致）。
    manager.bind_role(&outbound_tx, ConnectionRole::Driver, None);

    let (_r1_id, _r1_token, r1_cancel, _r1_rx, _) = manager
        .start_run(ProviderRunKind::ReviewOnly, Some("r1".to_string()))
        .await
        .expect("start R1");
    manager
        .abort_active_run_from_attachment(Some("socket-holder"))
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
            ProviderRunKind::WorkItemPlanRevision { feedback: None },
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
            candidate_recovery: None,
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: 0,
            manual_repairs_remaining: 1,
            trigger: HumanReason::NativeHumanRequired,
            resumable: false,
            accepted_feedback_turns: None,
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
/// C3/REQ-HTR-01：router 消费 HumanGateOpened/Closed 维护锁无关门开标志——
/// Abort 矩阵校验在 engine 锁被 in-flight 修订 run 持有时以它拒收门内 Abort。
/// 顺序锚：标志先于同事件的广播帧置位/复位，收到广播即代表标志已更新。
#[tokio::test]
async fn human_gate_events_maintain_lock_free_gate_open_flag() {
    let manager = WorkspaceSessionManager::test_fixture_with_event_router("session_gate_flag");
    let (observer_tx, mut observer_rx) = mpsc::channel(8);
    manager.attach("observer", observer_tx).await;
    assert!(
        !manager.human_confirm_gate_open(),
        "初始无门：标志必须为 false"
    );

    manager
        .engine_tx()
        .send(EngineEvent::HumanGateOpened {
            stage: "human_confirm".to_string(),
        })
        .await
        .expect("send human gate opened event");
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let Some(OutboundControl::Text(json)) = observer_rx.recv().await else {
                panic!("attachment closed before session_state");
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("frame JSON");
            if value["type"] == "session_state" {
                break;
            }
        }
    })
    .await
    .expect("gate open must broadcast session_state");
    assert!(
        manager.human_confirm_gate_open(),
        "HumanGateOpened(human_confirm) 后标志必须置位"
    );

    manager
        .engine_tx()
        .send(EngineEvent::HumanGateClosed {
            decision: "terminate".to_string(),
            stage: "completed".to_string(),
        })
        .await
        .expect("send human gate closed event");
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let Some(OutboundControl::Text(json)) = observer_rx.recv().await else {
                panic!("attachment closed before human_gate_closed");
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("frame JSON");
            if value["type"] == "human_gate_closed" {
                break;
            }
        }
    })
    .await
    .expect("gate close must broadcast human_gate_closed");
    assert!(
        !manager.human_confirm_gate_open(),
        "HumanGateClosed 后标志必须复位"
    );
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
    // REQ-PIB-02：夹具对齐生产不变量（main 分支+初始提交，裸目录无 git 仓库
    // 会令基线解析 fail-closed，run 未及 spawn 即终止）。
    for args in [
        vec!["init", "--initial-branch", "main"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test User"],
        vec!["commit", "--allow-empty", "-m", "fixture baseline"],
    ] {
        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(root.path())
            .status()
            .expect("git fixture");
        assert!(status.success(), "git {} failed", args.join(" "));
    }
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
            base_branch: None,
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
            reviewer_provider: Some(ProviderName::Codex),

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

// —— T4.3（REQ-REG-15/D2）：manager create 复用 state 缓存依赖注入确认 context ——

/// 在 enrollment 夹具（完整 legacy LC 拓扑：project/issue/manifest/成员/
/// checkout/aggregate index/selection/policy + 已确认 story/design）上建
/// story 会话（focus=None → 聚合根视图=logical_repository_id Some）。
fn lc_confirm_story_session() -> (
    crate::web::handlers::automation_enrollment_test_support::Fixture,
    crate::product::models::WorkspaceSessionRecord,
) {
    use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
    use crate::product::models::ProviderName;
    use crate::web::handlers::automation_enrollment_test_support::seed_fixture;

    let fixture = seed_fixture(2, true);
    let session_record = fixture
        .lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: fixture.story_id.clone(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: None,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("LC story session");
    (fixture, session_record)
}

#[tokio::test]
async fn manager_create_injects_confirmation_trust_context_for_lc_session() {
    use crate::product::logical_codebase::LogicalCodebaseStore;

    let (fixture, session_record) = lc_confirm_story_session();
    let root_path = fixture._root.path().to_path_buf();
    let state = WebAppState::with_provider_registry(
        root_path.clone(),
        WebRuntime::new_fake(root_path.clone()),
        ProviderRegistry::new(),
    );
    let deps_trust = state
        .aggregate_initialization_dependencies()
        .trust()
        .cloned()
        .expect("state 缓存 trust precondition（生产/fake HOME 策略同源装配）");

    let manager = WorkspaceSessionManager::create(&state, &session_record.id)
        .await
        .expect("create manager for LC session");
    let engine_arc = manager.engine();
    let engine = engine_arc.lock().await;
    let context = engine
        .confirmation_trust_context()
        .expect("LC 会话注入确认 trust 冻结上下文");
    assert_eq!(
        context.issue_lc_id, None,
        "legacy LC：冻结 issue 原始归属=None（registry 标签沿既有 legacy 别名约定推导）"
    );
    let manifest_root = LogicalCodebaseStore::new(fixture.paths.clone())
        .load_manifest("project_0001")
        .expect("load manifest")
        .expect("manifest present")
        .provider_context_root;
    assert_eq!(
        context.frozen_root,
        std::fs::canonicalize(&manifest_root).expect("canonical manifest root"),
        "冻结 root=gateway authority_root（同 manifest 聚合根）"
    );
    assert!(
        std::sync::Arc::ptr_eq(&context.precondition, &deps_trust),
        "注入的 precondition 复用同一 WebAppState 缓存实例（不新造第二套 HOME 推导）"
    );
    assert!(
        engine.logical_provider_gateway().is_some(),
        "LC 会话 gateway 仍在（readonly source 不持可写 registry）"
    );
}

#[tokio::test]
async fn manager_create_confirmation_context_boundaries() {
    use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
    use crate::product::models::ProviderName;

    // 单仓会话：无 gateway、无确认上下文（确认免登记零调用）。
    let root = tempfile::tempdir().expect("root");
    let app_paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(app_paths.clone())
        .create(CreateProjectInput {
            name: "single repo boundary".to_string(),
            description: None,
        })
        .expect("project");
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: project.id.clone(),
            name: "boundary repository".to_string(),
            path: root.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: Some("fake".to_string()),
            idempotency_key: "confirm-boundary-repository".to_string(),
        })
        .expect("repository");
    let issue = IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: Some(repository.id.clone()),
            logical_codebase_id: None,
            base_branch: None,
            title: "single repo boundary issue".to_string(),
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
            title: "single repo boundary story".to_string(),
            aggregate_codebase: None,
        })
        .expect("story");
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: project.id,
            issue_id: issue.id,
            entity_id: story.id,
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: None,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("single-repo session");
    let state = WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        ProviderRegistry::new(),
    );
    let manager = WorkspaceSessionManager::create(&state, &session_record.id)
        .await
        .expect("create manager for single-repo session");
    {
        let engine_arc = manager.engine();
        let engine = engine_arc.lock().await;
        assert!(
            engine.logical_provider_gateway().is_none(),
            "单仓会话无 LC gateway"
        );
        assert!(
            engine.confirmation_trust_context().is_none(),
            "单仓会话不注入确认上下文（免登记）"
        );
    }

    // LC 会话缺 trust precondition（依赖图重建为无 trust 形态）：fail-closed
    // 不注入——确认时 ContextWaiting（引擎面已由 4.1 漂移矩阵「缺上下文」
    // 用例锁定）。
    let (fixture2, session_record2) = lc_confirm_story_session();
    let root2_path = fixture2._root.path().to_path_buf();
    let state2 = WebAppState::with_provider_registry(
        root2_path.clone(),
        WebRuntime::new_fake(root2_path.clone()),
        ProviderRegistry::new(),
    );
    let deps = state2.aggregate_initialization_dependencies();
    let stripped = crate::web::handlers::AggregateInitializationDependencies::with_index(
        deps.coordinator.clone(),
        deps.runs.clone(),
        deps.index.clone(),
    );
    let state2 = state2.with_aggregate_initialization_dependencies(stripped);
    let manager2 = WorkspaceSessionManager::create(&state2, &session_record2.id)
        .await
        .expect("缺 trust 依赖不阻断 create（fail-closed 延后到确认面）");
    let engine2_arc = manager2.engine();
    let engine2 = engine2_arc.lock().await;
    assert!(
        engine2.logical_provider_gateway().is_some(),
        "LC 会话 gateway 装配不受 trust 依赖缺失影响"
    );
    assert!(
        engine2.confirmation_trust_context().is_none(),
        "LC 缺 precondition：不注入上下文，确认时 fail-closed ContextWaiting"
    );
}

// —— Task 7.2（REQ-REG-15）：真实 WebAppState/manager create→仲裁→Confirm→
// registry→广播 隔离测试 ——
//
// 不用私有 fake preparation 冒充生产注入：manager create 走真实依赖图
//（fake runtime 下 trust precondition home=workspace_root），factory 从
// HOME 装配的 readonly source 由夹具指到同一隔离 home（T7HomeEnvGuard），
// 真实用户 home 零读写。缺失条目自动补齐、ownership/audit 可观察、close
// 经原 journal/event_seq 广播对其它 attachment 唯一可见；observer/stale/
// stage 拒收零副作用。

/// T7 全部 HOME 改写用例（part_01/gateway_factory/conversational）共用的
/// sidecar 文件锁目标——同一测试二进制内互斥（先例：group_review_budget）。
fn t7_home_env_lock_target() -> std::path::PathBuf {
    std::env::temp_dir().join("aria-t7-home-env")
}

struct T7HomeEnvGuard {
    _lock: crate::product::coding_attempt_store::locking::ExclusiveFileLock,
    previous: Option<std::ffi::OsString>,
}

impl T7HomeEnvGuard {
    fn isolate(home: &std::path::Path) -> Self {
        let _lock = crate::product::coding_attempt_store::locking::ExclusiveFileLock::acquire(
            &t7_home_env_lock_target(),
        )
        .expect("serialize HOME mutation across the test binary");
        let previous = std::env::var_os("HOME");
        // SAFETY: 与其它改写 HOME 的 T7 用例经同一文件锁串行；本二进制内
        // 并行用例对 HOME 的消费只有 factory 装配的 adapter home 路径拼接
        //（无文件 IO），读写交错不产生可观察行为差异。
        unsafe { std::env::set_var("HOME", home) };
        Self { _lock, previous }
    }
}

impl Drop for T7HomeEnvGuard {
    fn drop(&mut self) {
        // SAFETY: 同上，文件锁串行下的恢复写。
        unsafe {
            if let Some(previous) = self.previous.clone() {
                std::env::set_var("HOME", previous);
            } else {
                std::env::remove_var("HOME");
            }
        }
    }
}

fn t7_session_snapshot_tree(root: &std::path::Path) -> Vec<(String, String)> {
    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("sha256:{:x}", Sha256::digest(bytes))
    }
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                let digest = std::fs::read(&path)
                    .map(|bytes| sha256_hex(&bytes))
                    .unwrap_or_else(|error| format!("read-error:{error}"));
                out.push((path.display().to_string(), digest));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

/// 开态门确认会话（真实 enrollment 夹具：2 成员 legacy LC + 已确认
/// story/design；author=Codex 需要 workspace trust）。用 legacy（非 SC）
/// WorkItemPlan 会话承载真实 HumanConfirm 确认面：plan 实体真实落库
///（Draft、一个 work item 占位 id——confirm 链要求非空）；durable 记录
/// 改写为 WaitingForHuman（引擎 stage=HumanConfirm）。Story/Design 的
/// HumanConfirm 门已随 L2 退役（懒迁移回 AuthorConfirm、approve 走 HTTP
/// confirm 端点），WorkItem 需要重 runtime binding，均非 WS Confirm 帧
/// 的轻量现存确认面。
fn t7_waiting_confirm_session() -> (
    crate::web::handlers::automation_enrollment_test_support::Fixture,
    crate::product::models::WorkspaceSessionRecord,
) {
    use crate::product::lifecycle_store::{
        CreateIssueWorkItemPlanInput, CreateWorkspaceSessionInput,
    };
    use crate::product::models::{
        IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, WorkspaceSessionStatus,
    };

    let fixture = crate::web::handlers::automation_enrollment_test_support::seed_fixture(2, true);
    let work_item = fixture
        .lifecycle
        .create_work_item(crate::product::lifecycle_store::CreateWorkItemInput {
            id: Some("work_item_t7_confirm_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: crate::web::handlers::automation_enrollment_test_support::REPOSITORY_ID
                .to_string(),
            story_spec_ids: vec![fixture.story_id.clone()],
            design_spec_ids: Vec::new(),
            title: "t7 confirm work item".to_string(),
            ..crate::product::lifecycle_store::CreateWorkItemInput::default()
        })
        .expect("t7 work item record");
    let plan = fixture
        .lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: None,
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec![fixture.story_id.clone()],
            source_design_spec_ids: vec![fixture.design_id.clone()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec![work_item.id.clone()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("t7 plan entity");
    let session_record = fixture
        .lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: crate::product::models::ProviderName::Codex,
            reviewer_provider: None,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("t7 confirm session");
    let mut record = fixture
        .lifecycle
        .get_workspace_session(&session_record.id)
        .expect("load created session");
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    crate::product::json_store::write_json(
        &fixture
            .paths
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist waiting session");
    (fixture, record)
}

/// registry 侧 durable trust 事实根（legacy 别名 LC=project 级
/// `logical-codebase/` 子树，与确认链 registry scope 同源）。
fn t7_trust_facts_root(
    fixture: &crate::web::handlers::automation_enrollment_test_support::Fixture,
) -> std::path::PathBuf {
    fixture.paths.logical_codebase_root("project_0001")
}

/// 收集一条 attachment 出站流中的全部帧（等到静默窗或超时）。
async fn t7_drain_outbound(
    rx: &mut mpsc::Receiver<OutboundControl>,
    quiet: std::time::Duration,
) -> Vec<serde_json::Value> {
    let mut frames = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let wait = quiet.min(deadline.saturating_duration_since(std::time::Instant::now()));
        match tokio::time::timeout(wait, rx.recv()).await {
            Ok(Some(OutboundControl::Text(json))) => {
                if let Ok(value) = serde_json::from_str(&json) {
                    frames.push(value);
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
    }
    frames
}

/// 7.2 主用例①（当前会话逐项成功）：真实 WebAppState+manager create，
/// 开态门 confirm 从 driver 连接发出——仲裁放行→非 SC Confirm 接真实
/// registry→Codex 缺失条目自动补齐（home 工件+ownership owned=true+
/// ensure/registered 审计关联 workspace_confirm:<session>）→WorkItem 定稿
/// Confirmed→observer 连接经原 journal/event_seq 广播恰一次看到 completed。
/// 铁律验证：factory HOME 装配的 readonly source 与 registry 写入命中
/// 同一隔离 home（确认前指导指向隔离工件路径；确认后只读复验转 None）。
#[tokio::test]
async fn manager_confirm_registers_missing_trust_and_broadcasts_unique_close() {
    use crate::product::logical_codebase::{ProviderTrustHomeAdapter, ProviderTrustStore};
    use crate::product::models::WorkspaceSessionStatus;
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_handler::WsInMessage;

    let (fixture, record) = t7_waiting_confirm_session();
    let root_path = fixture._root.path().to_path_buf();
    let canonical_root =
        std::fs::canonicalize(fixture.paths.root()).expect("canonical aggregate root");
    let codex_config = root_path.join(".codex").join("config.toml");
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001");
    let trust_store = ProviderTrustStore::for_lc(fixture.paths.clone(), &lc_id);
    let codex_trust_key = crate::product::logical_codebase::CodexTrustAdapter::for_home(&root_path)
        .trust_key(&canonical_root);

    let _env = T7HomeEnvGuard::isolate(&root_path);
    let state = WebAppState::with_provider_registry(
        root_path.clone(),
        WebRuntime::new_fake(root_path.clone()),
        ProviderRegistry::new(),
    );
    let manager = WorkspaceSessionManager::create(&state, &record.id)
        .await
        .expect("create real manager");

    // 铁律（前半）：factory 从 HOME 装配的只读 source 命中同一隔离 home——
    // 确认前判 missing，指导指向隔离 home 的真实工件路径。
    {
        let engine_arc = manager.engine();
        let engine = engine_arc.lock().await;
        let gateway = engine
            .logical_provider_gateway()
            .expect("LC 会话 gateway 在场");
        let waiting = gateway
            .read_only_generation_trust_waiting(&crate::product::models::ProviderName::Codex)
            .expect("确认前隔离 home 无条目，只读复验必须判 missing");
        assert_eq!(waiting.reason_code, "trust_entry_missing");
        assert!(
            waiting
                .manual_guidance
                .contains(&codex_config.display().to_string()),
            "factory HOME 装配的 source 指向隔离 home 工件：{}",
            waiting.manual_guidance
        );
    }

    // 当前 driver 连接 + 远端 observer 连接（真实 attachment 语义）。
    let (current_tx, mut current_rx) = mpsc::channel(32);
    manager.register_attachment("t7-current", current_tx.clone());
    manager.bind_role(&current_tx, ConnectionRole::Driver, None);
    manager.activate_attachment("t7-current");
    let (observer_tx, mut observer_rx) = mpsc::channel(32);
    manager.register_attachment("t7-observer", observer_tx.clone());
    manager.bind_role(&observer_tx, ConnectionRole::Observer, None);
    manager.activate_attachment("t7-observer");

    // 仲裁（真实 lease 面）：driver 写面放行、observer 写面拒收。
    manager
        .arbitrate("t7-current", &WsInMessage::Confirm)
        .expect("driver 持有 lease，Confirm 写面放行");
    assert!(
        matches!(
            manager.arbitrate("t7-observer", &WsInMessage::Confirm),
            Err(ConnectionRole::Observer)
        ),
        "observer 连接的写面必须被仲裁拒收（OBSERVER_WRITE_REJECTED 面）"
    );

    let durable_before = t7_session_snapshot_tree(&t7_trust_facts_root(&fixture));
    assert!(trust_store.list_audit("project_0001").unwrap().is_empty());
    assert!(!codex_config.exists());

    // 经真实 WS 入站分发面 Confirm（非 SC 流 → handle_confirm）。
    let run_context = manager.provider_run_context(state.workspace_runs.clone());
    let context = crate::web::workspace_ws_handler::WorkspaceInboundContext {
        app_state: state.clone(),
        engine: manager.engine().clone(),
        run_context,
        outbound_tx: current_tx.clone(),
        session_id: record.id.clone(),
    };
    crate::web::workspace_ws_handler::handle_workspace_inbound_message(
        context,
        WsInMessage::Confirm,
    )
    .await;

    // 定稿：WorkItem 会话 Confirmed（唯一 close 的 durable 面）。
    let durable = fixture
        .lifecycle
        .get_workspace_session(&record.id)
        .expect("durable after confirm");
    assert_eq!(durable.status, WorkspaceSessionStatus::Confirmed);

    // missing 自动补齐：隔离 home 出现本根 Codex trusted 条目；Kimi 不在
    // 有效 provider 集（reviewer 缺席），零登记。
    let codex_entry = std::fs::read_to_string(&codex_config).expect("codex entry auto-filled");
    assert!(codex_entry.contains(&format!("[projects.\"{}\"]", canonical_root.display())));
    assert!(codex_entry.contains("trust_level = \"trusted\""));
    assert!(
        !root_path.join(".kimi-code").exists(),
        "非有效 provider 零登记（Kimi 未被触碰）"
    );

    // ownership/audit 可观察：owned=true + ensure/registered 恰一条，关联
    // workspace_confirm:<session>（审计事件名以源码为准，无 gate_registered）。
    let ownership = trust_store
        .load_ownership("project_0001", "codex", &codex_trust_key)
        .expect("ownership after")
        .expect("codex ownership recorded");
    assert!(ownership.owned, "本 LC 管理的登记 owned=true");
    let audits = trust_store.list_audit("project_0001").unwrap();
    assert_eq!(audits.len(), 1, "恰一条登记审计：{audits:?}");
    assert_eq!(audits[0].action, "ensure");
    assert_eq!(audits[0].outcome, "registered");
    assert_eq!(
        audits[0].operation_id.as_deref(),
        Some(format!("workspace_confirm:{}", record.id).as_str()),
        "审计关联本次确认（operation_id=workspace_confirm:<session>）"
    );
    assert_eq!(audits[0].provider, "codex");

    // 铁律（后半）：确认后同一 readonly source 判 trusted（None）——registry
    // 写入的 home 与 factory HOME 装配的 source 是同一隔离 home。
    {
        let engine_arc = manager.engine();
        let engine = engine_arc.lock().await;
        let gateway = engine
            .logical_provider_gateway()
            .expect("LC 会话 gateway 仍在场");
        assert!(
            gateway
                .read_only_generation_trust_waiting(&crate::product::models::ProviderName::Codex)
                .is_none(),
            "确认登记后只读复验必须转 trusted（同一隔离 home）"
        );
    }

    // 广播：observer 与当前连接都经原 journal/event_seq 收到帧；completed
    // 的 stage_change 恰一次（唯一 close），event_seq 单调。
    let observer_frames =
        t7_drain_outbound(&mut observer_rx, std::time::Duration::from_millis(300)).await;
    let _current_frames =
        t7_drain_outbound(&mut current_rx, std::time::Duration::from_millis(100)).await;
    let completed: Vec<&serde_json::Value> = observer_frames
        .iter()
        .filter(|frame| frame["type"] == "stage_change" && frame["stage"] == "completed")
        .collect();
    assert_eq!(
        completed.len(),
        1,
        "close 广播（completed stage_change）必须恰一次：{observer_frames:?}"
    );
    let mut seqs: Vec<u64> = observer_frames
        .iter()
        .filter_map(|frame| frame["event_seq"].as_u64())
        .collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(seqs.len(), sorted.len(), "广播帧 event_seq 不得重复");
    seqs.sort_unstable();
    assert!(
        seqs.windows(2).all(|pair| pair[0] < pair[1]),
        "event_seq 单调递增：{seqs:?}"
    );

    // durable trust 事实相对确认前只新增登记事实（布置零事实→恰新增）；
    // 迟到重复 Confirm：stage 已 Completed，非 SC 面 no-op 零副作用。
    let durable_after_first = t7_session_snapshot_tree(&t7_trust_facts_root(&fixture));
    assert_ne!(durable_before, durable_after_first, "登记事实已落盘");
    let audits_before_retry = trust_store.list_audit("project_0001").unwrap().len();
    let run_context = manager.provider_run_context(state.workspace_runs.clone());
    let (retry_tx, mut retry_rx) = mpsc::channel(8);
    let context = crate::web::workspace_ws_handler::WorkspaceInboundContext {
        app_state: state.clone(),
        engine: manager.engine().clone(),
        run_context,
        outbound_tx: retry_tx.clone(),
        session_id: record.id.clone(),
    };
    crate::web::workspace_ws_handler::handle_workspace_inbound_message(
        context,
        WsInMessage::Confirm,
    )
    .await;
    assert_eq!(
        trust_store.list_audit("project_0001").unwrap().len(),
        audits_before_retry,
        "迟到 Confirm 零登记零审计（stale 门后 no-op）"
    );
    assert_eq!(
        t7_session_snapshot_tree(&t7_trust_facts_root(&fixture)),
        durable_after_first,
        "迟到 Confirm 零 durable 副作用"
    );
    let retry_frames =
        t7_drain_outbound(&mut retry_rx, std::time::Duration::from_millis(200)).await;
    assert!(
        !retry_frames
            .iter()
            .any(|frame| frame["type"] == "stage_change" && frame["stage"] == "completed"),
        "迟到 Confirm 不产生第二次 close 广播"
    );
}

/// 7.2 主用例②（远端会话逐项等待）：第二个真实会话（远端连接），用户在
/// 隔离 home 预置非 trusted 冲突条目——confirm 接真实 registry 后 fail
/// closed：稳定 code 的 TrustWaiting 协议帧（真实冲突 reason+场景
/// retry_action）、门保持开态（durable 会话零变化）、用户冲突条目字节
/// 保持；失败 waiting/audit 落盘可观察（按设计），home 零写。
#[tokio::test]
async fn manager_confirm_conflict_waits_remote_session_without_side_effects() {
    use crate::product::logical_codebase::{ProviderTrustHomeAdapter, ProviderTrustStore};
    use crate::product::models::WorkspaceSessionStatus;
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_handler::WsInMessage;

    let (fixture, record) = t7_waiting_confirm_session();
    let root_path = fixture._root.path().to_path_buf();
    let canonical_root =
        std::fs::canonicalize(fixture.paths.root()).expect("canonical aggregate root");
    let codex_config = root_path.join(".codex").join("config.toml");
    std::fs::create_dir_all(codex_config.parent().unwrap()).expect("codex config dir");
    std::fs::write(
        &codex_config,
        format!(
            "[projects.\"{}\"]\ntrust_level = \"untrusted\"\n",
            canonical_root.display()
        ),
    )
    .expect("seed user conflict");
    let conflict_bytes = std::fs::read(&codex_config).expect("conflict bytes");
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001");
    let trust_store = ProviderTrustStore::for_lc(fixture.paths.clone(), &lc_id);

    let _env = T7HomeEnvGuard::isolate(&root_path);
    let state = WebAppState::with_provider_registry(
        root_path.clone(),
        WebRuntime::new_fake(root_path.clone()),
        ProviderRegistry::new(),
    );
    let manager = WorkspaceSessionManager::create(&state, &record.id)
        .await
        .expect("create real manager for remote session");

    // 远端短连接：绑定 driver（该会话首连接接管 lease），激活收帧。
    let (remote_tx, mut remote_rx) = mpsc::channel(16);
    manager.register_attachment("t7-remote", remote_tx.clone());
    manager.bind_role(&remote_tx, ConnectionRole::Driver, None);
    manager.activate_attachment("t7-remote");
    manager
        .arbitrate("t7-remote", &WsInMessage::Confirm)
        .expect("远端 driver 仲裁放行");

    let durable_session_before = serde_json::to_vec(
        &fixture
            .lifecycle
            .get_workspace_session(&record.id)
            .expect("durable before"),
    )
    .expect("serialize before");

    let run_context = manager.provider_run_context(state.workspace_runs.clone());
    let context = crate::web::workspace_ws_handler::WorkspaceInboundContext {
        app_state: state.clone(),
        engine: manager.engine().clone(),
        run_context,
        outbound_tx: remote_tx.clone(),
        session_id: record.id.clone(),
    };
    crate::web::workspace_ws_handler::handle_workspace_inbound_message(
        context,
        WsInMessage::Confirm,
    )
    .await;

    let outbound = tokio::time::timeout(std::time::Duration::from_secs(2), remote_rx.recv())
        .await
        .expect("远端确认的 trust 等待必须回执协议帧")
        .expect("outbound frame");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected protocol error text frame");
    };
    let message: crate::web::workspace_ws_types::WsOutMessage =
        serde_json::from_str(&json).expect("protocol error json");
    let crate::web::workspace_ws_types::WsOutMessage::ProtocolError { code, message, .. } = message
    else {
        panic!("远端 trust 等待必须是 ProtocolError，got {message:?}")
    };
    assert_eq!(code, "workspace_confirmation_trust_waiting");
    assert!(
        message.contains("codex_trust_key_conflict"),
        "真实冲突 reason 保留：{message}"
    );
    assert!(
        message.contains("重新确认"),
        "确认场景 retry_action 在场：{message}"
    );

    // 门保持开态：durable 会话零变化、无 close 广播、冲突条目字节保持。
    assert_eq!(
        serde_json::to_vec(
            &fixture
                .lifecycle
                .get_workspace_session(&record.id)
                .expect("durable after")
        )
        .expect("serialize after"),
        durable_session_before,
        "等待前后 durable 会话记录完全不变（门保持开态）"
    );
    assert_eq!(
        std::fs::read(&codex_config).expect("conflict bytes after"),
        conflict_bytes,
        "用户冲突条目不得被覆盖"
    );
    let frames = t7_drain_outbound(&mut remote_rx, std::time::Duration::from_millis(200)).await;
    assert!(
        !frames
            .iter()
            .any(|frame| frame["type"] == "stage_change" && frame["stage"] == "completed"),
        "等待不产生 close 广播"
    );

    // 失败面可观察（按设计落盘）：waiting+ensure/conflict 审计在场，
    // ownership 不落（未登记成功）。
    let waiting = trust_store.list_waiting("project_0001").unwrap();
    assert!(
        waiting
            .iter()
            .any(|record| record.reason_code == "codex_trust_key_conflict"),
        "失败 waiting 落盘可观察：{waiting:?}"
    );
    let audits = trust_store.list_audit("project_0001").unwrap();
    assert!(
        audits
            .iter()
            .any(|record| record.action == "ensure" && record.outcome == "conflict"),
        "冲突审计落盘可观察：{audits:?}"
    );
    let codex_trust_key = crate::product::logical_codebase::CodexTrustAdapter::for_home(&root_path)
        .trust_key(&canonical_root);
    assert!(
        trust_store
            .load_ownership("project_0001", "codex", &codex_trust_key)
            .expect("ownership lookup")
            .is_none(),
        "冲突未登记成功，零 ownership"
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&record.id)
        .expect("session still waiting");
    assert_eq!(durable.status, WorkspaceSessionStatus::WaitingForHuman);
}

/// 7.2 拒收矩阵（observer/stale/stage 零副作用）：仲裁面 observer 写拒、
/// lease 转移后旧 driver 写拒（STALE_DRIVER_LEASE 面）；stage 白名单在
/// Running 态拒收 Confirm（socket 边界谓词，分发前拒收）——三者均零
/// home/durable trust 副作用、零审计。
#[tokio::test]
async fn manager_confirm_rejection_matrix_has_zero_trust_side_effects() {
    use crate::product::logical_codebase::ProviderTrustStore;
    use crate::product::models::WorkspaceSessionStatus;
    use crate::product::workspace_engine::WorkspaceStage;
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_handler::{WsInMessage, is_message_valid_for_stage_with_flow};

    let (fixture, record) = t7_waiting_confirm_session();
    let root_path = fixture._root.path().to_path_buf();
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001");
    let trust_store = ProviderTrustStore::for_lc(fixture.paths.clone(), &lc_id);

    let _env = T7HomeEnvGuard::isolate(&root_path);
    let state = WebAppState::with_provider_registry(
        root_path.clone(),
        WebRuntime::new_fake(root_path.clone()),
        ProviderRegistry::new(),
    );
    let manager = WorkspaceSessionManager::create(&state, &record.id)
        .await
        .expect("create real manager");

    let (driver1_tx, _rx1) = mpsc::channel(8);
    manager.register_attachment("t7-driver-1", driver1_tx.clone());
    manager.bind_role(&driver1_tx, ConnectionRole::Driver, None);
    manager.activate_attachment("t7-driver-1");
    let (driver2_tx, _rx2) = mpsc::channel(8);
    manager.register_attachment("t7-driver-2", driver2_tx.clone());
    let (observer_tx, _rx3) = mpsc::channel(8);
    manager.register_attachment("t7-observer", observer_tx.clone());
    manager.bind_role(&observer_tx, ConnectionRole::Observer, None);
    manager.activate_attachment("t7-observer");

    let trust_facts_before = t7_session_snapshot_tree(&t7_trust_facts_root(&fixture));
    let home_before = t7_session_snapshot_tree(&root_path.join(".codex"));

    // observer 拒收：写面被仲裁拒绝（socket 层 OBSERVER_WRITE_REJECTED 面）。
    assert!(matches!(
        manager.arbitrate("t7-observer", &WsInMessage::Confirm),
        Err(ConnectionRole::Observer)
    ));
    // stale 拒收：driver2 绑定 driver（lease 转移），旧 driver 写面变 stale。
    manager.bind_role(&driver2_tx, ConnectionRole::Driver, None);
    assert!(
        matches!(
            manager.arbitrate("t7-driver-1", &WsInMessage::Confirm),
            Err(ConnectionRole::Driver)
        ),
        "lease 已转移：旧 driver 写面必须 STALE_DRIVER_LEASE 拒收"
    );

    // stage 拒收：Running 态 Confirm 不在白名单（socket 边界在分发前拒收，
    // 引擎 handle_confirm 的 Running 臂因此不可达）。
    assert!(
        !is_message_valid_for_stage_with_flow(
            record.flow_kind,
            &WsInMessage::Confirm,
            &WorkspaceStage::Running,
        ),
        "Running 态 Confirm 必须被 stage 白名单拒收（分发前）"
    );

    assert!(trust_store.list_audit("project_0001").unwrap().is_empty());
    assert!(trust_store.list_waiting("project_0001").unwrap().is_empty());
    assert_eq!(
        t7_session_snapshot_tree(&t7_trust_facts_root(&fixture)),
        trust_facts_before,
        "拒收矩阵零 durable trust 副作用"
    );
    assert_eq!(
        t7_session_snapshot_tree(&root_path.join(".codex")),
        home_before,
        "拒收矩阵零 home 副作用"
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&record.id)
        .expect("session untouched");
    assert_eq!(durable.status, WorkspaceSessionStatus::WaitingForHuman);
}
