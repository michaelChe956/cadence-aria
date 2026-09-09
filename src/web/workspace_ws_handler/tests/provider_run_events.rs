use super::*;
use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
use crate::product::models::WorkspaceType;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn provider_run_request_event_starts_and_registers_provider_run_once() {
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
            reviewer_provider: ProviderName::Codex,
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
    let held_event_senders = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(PendingStartProvider {
            starts: starts.clone(),
            held_event_senders: held_event_senders.clone(),
        }),
    );
    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(registry),
        engine: engine.clone(),
        current_run: current_run.clone(),
        workspace_runs: workspace_runs.clone(),
        session_id: session_record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths,
        session_record: session_record.clone(),
    };
    let (outbound_tx, mut outbound_rx) = mpsc::channel(8);
    let forward = spawn_engine_event_forward_task(
        engine_rx,
        outbound_tx,
        session_record.id.clone(),
        workspace_runs.clone(),
        Some(run_context),
    );

    engine_tx
        .send(EngineEvent::ProviderRunRequested {
            kind: ProviderRunKind::Author {
                content: "start provider from engine event".to_string(),
            },
            node_id: None,
        })
        .await
        .expect("queue provider run request");

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if starts.load(Ordering::SeqCst) == 1
                && workspace_runs.run(&session_record.id).await.is_some()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider request must start and register one run");
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    let active_node_id = workspace_runs
        .run(&session_record.id)
        .await
        .expect("registered provider run")
        .node_id;

    engine_tx
        .send(EngineEvent::ProviderRunRequested {
            kind: ProviderRunKind::Author {
                content: "duplicate provider request".to_string(),
            },
            node_id: active_node_id,
        })
        .await
        .expect("queue duplicate provider run request");
    engine_tx
        .send(EngineEvent::StageChange {
            stage: "duplicate-request-drained".to_string(),
        })
        .await
        .expect("queue outbound ordering marker");
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let control = outbound_rx
                .recv()
                .await
                .expect("event forwarder must drain duplicate before the ordering marker");
            let OutboundControl::Text(text) = control else {
                continue;
            };
            let message = serde_json::from_str::<serde_json::Value>(&text)
                .expect("outbound control must be JSON");
            if message["stage"] == "duplicate-request-drained" {
                break;
            }
        }
    })
    .await
    .expect("stage-change ordering marker must reach the outbound channel");
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "same active timeline node must not be started twice"
    );

    let _ = abort_active_run(&current_run, &workspace_runs, &session_record.id).await;
    held_event_senders.lock().await.clear();
    drop(engine_tx);
    forward.abort();
    let _ = forward.await;
}

#[tokio::test]
async fn provider_run_request_event_replaces_an_active_run_for_a_new_timeline_node() {
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
            reviewer_provider: ProviderName::Codex,
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
    let held_event_senders = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(PendingStartProvider {
            starts: starts.clone(),
            held_event_senders: held_event_senders.clone(),
        }),
    );
    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
    let (old_command_tx, _old_command_rx) = mpsc::channel(1);
    let old_run = WorkspaceActiveRun {
        id: 0,
        token: 0,
        node_id: Some("old-timeline-node".to_string()),
        cancel: CancellationToken::new(),
        command_tx: old_command_tx,
        pending_choice_ids: Arc::new(Mutex::new(std::collections::HashSet::new())),
    };
    let old_cancel = old_run.cancel.clone();
    workspace_runs
        .insert(session_record.id.clone(), old_run)
        .await;
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(registry),
        engine: engine.clone(),
        current_run: current_run.clone(),
        workspace_runs: workspace_runs.clone(),
        session_id: session_record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths,
        session_record: session_record.clone(),
    };
    let (outbound_tx, _outbound_rx) = mpsc::channel(8);
    let forward = spawn_engine_event_forward_task(
        engine_rx,
        outbound_tx,
        session_record.id.clone(),
        workspace_runs.clone(),
        Some(run_context),
    );
    let new_node_id = engine.lock().await.active_timeline_node_id();

    engine_tx
        .send(EngineEvent::ProviderRunRequested {
            kind: ProviderRunKind::Author {
                content: "start provider for new timeline node".to_string(),
            },
            node_id: new_node_id.clone(),
        })
        .await
        .expect("queue new-node provider run request");

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if starts.load(Ordering::SeqCst) == 1
                && workspace_runs
                    .run(&session_record.id)
                    .await
                    .is_some_and(|run| run.node_id == new_node_id)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("new-node provider request must replace the old active run");
    assert!(
        old_cancel.is_cancelled(),
        "new node must cancel the old active run"
    );

    let _ = abort_active_run(&current_run, &workspace_runs, &session_record.id).await;
    held_event_senders.lock().await.clear();
    drop(engine_tx);
    forward.abort();
    let _ = forward.await;
}

/// 现场锚（claude×轻 rep1v11 session_0255 双 spawn 误杀）：SingleCandidate
/// author run 完成后 engine 进 CrossReview 并新建 ReviewerRun 节点，同时发出
/// ReviewOnly 接力事件（product/workspace_engine/single_candidate.rs 的
/// `request_provider_run(ReviewOnly)`）；在途 run 自身任务经 followups 宏内联
/// 驱动 reviewer 慢握手。接力请求的 node_id（ReviewerRun 节点）与在途 run
/// 注册时的 node_id（author 节点）不同，同节点去重不命中——若放行到
/// `spawn_provider_run_from_handler`，其无条件 supersede 会取消在途 run 的
/// token，杀死正在握手的 reviewer 会话（handshake failed: cancelled）并
/// 双驱动 review。本测试锚定：ReviewOnly 接力到达时会话已有活跃 run 在途 →
/// drain（不打断、不再起第二个 provider）。
#[tokio::test]
async fn review_only_relay_event_yields_to_in_flight_run_review_handoff() {
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
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
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
    let review_starts = Arc::new(AtomicUsize::new(0));
    let held_event_senders = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Codex,
        Arc::new(PendingStartProvider {
            starts: review_starts.clone(),
            held_event_senders: held_event_senders.clone(),
        }),
    );
    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
    // 在途 run：模拟 SC author run 在「author 完成 → reviewer 启动」交接窗口
    // （followups 即将/正在驱动 review 握手），注册节点是 author 节点。
    let (in_flight_command_tx, _in_flight_command_rx) = mpsc::channel(1);
    let in_flight_run = WorkspaceActiveRun {
        id: 1,
        token: 1,
        node_id: Some("single-candidate-author-node".to_string()),
        cancel: CancellationToken::new(),
        command_tx: in_flight_command_tx,
        pending_choice_ids: Arc::new(Mutex::new(std::collections::HashSet::new())),
    };
    let in_flight_cancel = in_flight_run.cancel.clone();
    workspace_runs
        .insert(session_record.id.clone(), in_flight_run)
        .await;
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(registry),
        engine: engine.clone(),
        current_run: current_run.clone(),
        workspace_runs: workspace_runs.clone(),
        session_id: session_record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths,
        session_record: session_record.clone(),
    };
    let (outbound_tx, mut outbound_rx) = mpsc::channel(8);
    let forward = spawn_engine_event_forward_task(
        engine_rx,
        outbound_tx,
        session_record.id.clone(),
        workspace_runs.clone(),
        Some(run_context),
    );

    engine_tx
        .send(EngineEvent::ProviderRunRequested {
            kind: ProviderRunKind::ReviewOnly,
            node_id: Some("reviewer-round-1-node".to_string()),
        })
        .await
        .expect("queue review-only relay event");
    engine_tx
        .send(EngineEvent::StageChange {
            stage: "review-relay-ordering-marker".to_string(),
        })
        .await
        .expect("queue outbound ordering marker");
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let control = outbound_rx
                .recv()
                .await
                .expect("event forwarder must stay alive");
            let OutboundControl::Text(text) = control else {
                continue;
            };
            let message = serde_json::from_str::<serde_json::Value>(&text)
                .expect("outbound control must be JSON");
            if message["stage"] == "review-relay-ordering-marker" {
                break;
            }
        }
    })
    .await
    .expect("stage-change ordering marker must reach the outbound channel");
    // 观察窗：若误杀存在（当前缺陷行为），在途 token 会被接力 spawn 取消。
    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        while !in_flight_cancel.is_cancelled() {
            tokio::task::yield_now().await;
        }
    })
    .await;

    assert!(
        !in_flight_cancel.is_cancelled(),
        "ReviewOnly 引擎接力不得 supersede 误杀在途 run（claude×轻 rep1v11 现场：\n\
         在途 run 的 followups 正驱动 reviewer 握手，接力 spawn 应 drain 让位）"
    );
    assert_eq!(
        review_starts.load(Ordering::SeqCst),
        0,
        "ReviewOnly 引擎接力不得在在途 run 存活时再起第二个 reviewer provider"
    );
    let active = workspace_runs
        .run(&session_record.id)
        .await
        .expect("in-flight run must stay registered");
    assert_eq!(
        active.token, 1,
        "registry 必须保留在途 run（不得被接力 run 替换）"
    );
    assert_eq!(
        active.node_id.as_deref(),
        Some("single-candidate-author-node"),
        "registry 保留的必须是在途 run 的 author 节点"
    );

    let _ = abort_active_run(&current_run, &workspace_runs, &session_record.id).await;
    held_event_senders.lock().await.clear();
    drop(engine_tx);
    forward.abort();
    let _ = forward.await;
}

/// 用户显式路径回归锚：UserMessage/决策等 handler 直调
/// `spawn_provider_run_from_handler` 的 supersede 语义不因引擎接力收窄而变化
/// ——新 run 启动必须取消在途 run（无论节点差异）。
#[tokio::test]
async fn handler_originated_spawn_still_supersedes_in_flight_run() {
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
            reviewer_provider: ProviderName::Codex,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("workspace session");
    let (engine_tx, _engine_rx) = mpsc::channel(8);
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
    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
    let (old_command_tx, _old_command_rx) = mpsc::channel(1);
    let old_run = WorkspaceActiveRun {
        id: 0,
        token: 0,
        node_id: Some("old-timeline-node".to_string()),
        cancel: CancellationToken::new(),
        command_tx: old_command_tx,
        pending_choice_ids: Arc::new(Mutex::new(std::collections::HashSet::new())),
    };
    let old_cancel = old_run.cancel.clone();
    workspace_runs
        .insert(session_record.id.clone(), old_run)
        .await;
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(registry),
        engine: engine.clone(),
        current_run: current_run.clone(),
        workspace_runs: workspace_runs.clone(),
        session_id: session_record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths,
        session_record: session_record.clone(),
    };
    let (outbound_tx, _outbound_rx) = mpsc::channel(8);

    spawn_provider_run_from_handler(
        run_context,
        ProviderRunKind::Author {
            content: "user message supersedes streaming run".to_string(),
        },
        outbound_tx,
    )
    .await
    .expect("handler-originated run must start");

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if starts.load(Ordering::SeqCst) == 1 && old_cancel.is_cancelled() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("handler-originated spawn must supersede the in-flight run");
    assert!(old_cancel.is_cancelled());

    let _ = abort_active_run(&current_run, &workspace_runs, &session_record.id).await;
    held_event_senders.lock().await.clear();
}

struct PendingStartProvider {
    starts: Arc<AtomicUsize>,
    held_event_senders: Arc<Mutex<Vec<mpsc::Sender<ProviderEvent>>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PendingStartProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let (event_tx, event_rx) = mpsc::channel(1);
        self.held_event_senders.lock().await.push(event_tx);
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
