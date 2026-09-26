use super::*;
use crate::cross_cutting::provider_adapter::structured_output_sentinel;
use crate::cross_cutting::streaming_provider::ProviderCompletion;
use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
use crate::product::models::WorkspaceType;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::single_candidate_provider_run::{ProviderRunFixture, single_candidate_markdown};
use crate::web::workspace_session::ActiveRun;

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
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine.clone(),
        workspace_runs.clone(),
        session_record.id.clone(),
        app_paths,
        session_record.clone(),
    );
    let (outbound_tx, mut outbound_rx) = mpsc::channel(8);
    let manager = run_context.manager.clone();
    let forward = spawn_engine_event_forward_task(engine_rx, outbound_tx, Some(run_context));

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
            if starts.load(Ordering::SeqCst) == 1 && manager.active_run().await.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider request must start and register one run");
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    let active_node_id = manager
        .active_run()
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

    let _ = manager.abort_active_run().await;
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
    let (old_command_tx, _old_command_rx) = mpsc::channel(1);
    let old_cancel = CancellationToken::new();
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine.clone(),
        workspace_runs.clone(),
        session_record.id.clone(),
        app_paths,
        session_record.clone(),
    );
    let manager = run_context.manager.clone();
    manager
        .test_set_active_run(ActiveRun {
            kind: ProviderRunKind::ReviewOnly,
            id: 0,
            token: 0,
            run_incarnation: uuid::Uuid::new_v4().to_string(),
            node_id: Some("old-timeline-node".to_string()),
            cancel: old_cancel.clone(),
            command_tx: old_command_tx,
            pending_choices: Arc::new(std::sync::Mutex::new(Vec::new())),
            lease_epoch: 0,
        })
        .await;
    let (outbound_tx, _outbound_rx) = mpsc::channel(8);
    let forward = spawn_engine_event_forward_task(engine_rx, outbound_tx, Some(run_context));
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
                && manager
                    .active_run()
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

    let _ = manager.abort_active_run().await;
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
    // 在途 run：模拟 SC author run 在「author 完成 → reviewer 启动」交接窗口
    // （followups 即将/正在驱动 review 握手），注册节点是 author 节点。
    let (in_flight_command_tx, _in_flight_command_rx) = mpsc::channel(1);
    let in_flight_cancel = CancellationToken::new();
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine.clone(),
        workspace_runs.clone(),
        session_record.id.clone(),
        app_paths,
        session_record.clone(),
    );
    let manager = run_context.manager.clone();
    manager
        .test_set_active_run(ActiveRun {
            kind: ProviderRunKind::ReviewOnly,
            id: 1,
            token: 1,
            run_incarnation: uuid::Uuid::new_v4().to_string(),
            node_id: Some("single-candidate-author-node".to_string()),
            cancel: in_flight_cancel.clone(),
            command_tx: in_flight_command_tx,
            pending_choices: Arc::new(std::sync::Mutex::new(Vec::new())),
            lease_epoch: 0,
        })
        .await;
    let (outbound_tx, mut outbound_rx) = mpsc::channel(8);
    let forward = spawn_engine_event_forward_task(engine_rx, outbound_tx, Some(run_context));

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
    let active = manager
        .active_run()
        .await
        .expect("in-flight run must stay registered");
    assert_eq!(
        active.token, 1,
        "manager 必须保留在途 run（不得被接力 run 替换）"
    );
    assert_eq!(
        active.node_id.as_deref(),
        Some("single-candidate-author-node"),
        "manager 保留的必须是在途 run 的 author 节点"
    );

    let _ = manager.abort_active_run().await;
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
    let (old_command_tx, _old_command_rx) = mpsc::channel(1);
    let old_cancel = CancellationToken::new();
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine.clone(),
        workspace_runs,
        session_record.id.clone(),
        app_paths,
        session_record.clone(),
    );
    let manager = run_context.manager.clone();
    manager
        .test_set_active_run(ActiveRun {
            kind: ProviderRunKind::ReviewOnly,
            id: 0,
            token: 0,
            run_incarnation: uuid::Uuid::new_v4().to_string(),
            node_id: Some("old-timeline-node".to_string()),
            cancel: old_cancel.clone(),
            command_tx: old_command_tx,
            pending_choices: Arc::new(std::sync::Mutex::new(Vec::new())),
            lease_epoch: 0,
        })
        .await;
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

    let _ = manager.abort_active_run().await;
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

// ---------------------------------------------------------------------------
// SC 修订接力顶杀在途 run 根修（oracle 候选6）
//
// CrossReview 对 SingleCandidate 流二义：phase=Evaluate 待驱评审（在途 run 的
// followups 自续 reviewer），phase=Generate 表示 policy 路由已把返修 author
// 重跑委托给 ProviderRunRequested{WorkItemPlanSingleCandidateAuthor} 接力（唯一
// 接续路径）。缺 phase 维度时 followups 会误燃第二轮 reviewer，接力 spawn 的
// supersede 随即取消在途 token 杀死该误燃握手（现场：claude policy session:
// handshake failed: cancelled）。以下三锚分别钉死：误燃不可复现、接力仍 supersede
// （候选1/5 不得回潮）、Evaluate 放行侧不被收窄。
// ---------------------------------------------------------------------------

fn sc_context_with_registry(
    fixture: &ProviderRunFixture,
    registry: ProviderRegistry,
) -> (
    WorkspaceInboundContext,
    mpsc::Receiver<OutboundControl>,
    ProviderRunContext,
    mpsc::Sender<OutboundControl>,
) {
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    run_context.manager = fixture.manager.clone();
    let root = fixture.root_path();
    let (outbound_tx, outbound_rx) = mpsc::channel(64);
    (
        WorkspaceInboundContext {
            app_state: WebAppState::new(
                root.clone(),
                crate::web::runtime::WebRuntime::new_fake(root),
            ),
            engine: fixture.engine.clone(),
            run_context: run_context.clone(),
            outbound_tx: outbound_tx.clone(),
            session_id: fixture.record.id.clone(),
        },
        outbound_rx,
        run_context,
        outbound_tx,
    )
}

/// SC author：首次 start 立即产出合法 markdown；后续 start 挂起（事件端存活但
/// 永不完成），模拟接力 run 的在途 provider。
struct ScAuthorStubProvider {
    output: String,
    starts: Arc<AtomicUsize>,
    held: Mutex<Vec<mpsc::Sender<ProviderEvent>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScAuthorStubProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        if start == 1 {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    self.output.clone(),
                    None,
                )))
                .await;
        } else {
            self.held.lock().await.push(event_tx);
        }
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

/// SC reviewer：首次 start 立即完成 revise verdict；后续 start 挂起（复现误燃
/// 第二轮握手——事件端存活但永不完成）。
struct ScReviseReviewProvider {
    starts: Arc<AtomicUsize>,
    held: Mutex<Vec<mpsc::Sender<ProviderEvent>>>,
}

fn sc_repairable_revise_output() -> String {
    let payload = serde_json::json!({
        "verdict": "revise",
        "review_scope": "outline",
        "generation_round_id": "round-1",
        "summary": "SC 机械契约缺口，需 author 返修",
        "findings": [{
            "severity": "must_fix",
            "message": "candidate contract gap requires author repair",
            "evidence": "single candidate IR",
            "required_action": "regenerate the candidate markdown",
            "category": "contract_gap",
            "class_hint": "repairable",
        }],
    });
    payload.to_string()
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScReviseReviewProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        if start == 1 {
            let contract = input.structured_output_contract.clone();
            let output = match contract.as_ref() {
                Some(contract) => structured_output_sentinel(
                    &contract.nonce,
                    &serde_json::from_str(&sc_repairable_revise_output())
                        .expect("review fixture payload"),
                ),
                None => sc_repairable_revise_output(),
            };
            let completion = ProviderCompletion::from_output(output, contract.as_ref(), None);
            let _ = event_tx.send(ProviderEvent::Completed(completion)).await;
        } else {
            self.held.lock().await.push(event_tx);
        }
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

fn review_enabled_provider_config() -> ProviderConfigSnapshot {
    ProviderConfigSnapshot {
        author: ProviderName::ClaudeCode,
        reviewer: Some(ProviderName::Codex),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    }
}

/// fixture 建会话时 review_rounds=0；`start_generation` 只持久化 provider，不写
/// review_rounds，而 `reserve_single_candidate_author_start` 会从 durable record
/// 重建 engine session——若不先落盘 review_rounds，SC author 完成会走「未启用
/// reviewer」快速路径，整个 review 腿不可达。生产由建会话输入携带，本测试直接
/// 落盘等价状态。
fn persist_review_rounds(fixture: &ProviderRunFixture, rounds: u32) {
    let mut record = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("load workspace session");
    record.review_rounds = rounds;
    let path = fixture
        .app_paths
        .issue_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id));
    crate::product::json_store::write_json(&path, &record).expect("persist review rounds");
}

/// 红锚1（误燃）：SC author run 完成 → followups 内联驱一轮 reviewer 得 revise
/// verdict → policy 路由打回 Generate 并发射 WorkItemPlanSingleCandidateAuthor
/// 接力。修复前 followups 只按 stage==CrossReview 判定，会立即误燃第二轮
/// reviewer（启动计数 2）；修复后在委托态 break，reviewer 恰 1 次、run 任务退场、
/// 接力事件仍按全名 kind 发出。
#[tokio::test]
async fn single_candidate_revise_route_does_not_misfire_a_second_followup_review() {
    let (fixture, engine_rx) =
        ProviderRunFixture::new_with_engine_rx(WorkItemPlanFlowKind::SingleCandidate);
    persist_review_rounds(&fixture, 1);
    let relay_kinds = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorded = relay_kinds.clone();
    let relay_spawn = Arc::new(Mutex::new(
        Option::<(ProviderRunContext, mpsc::Sender<OutboundControl>)>::None,
    ));
    let spawn_slot = relay_spawn.clone();
    let drain_engine = tokio::spawn(async move {
        let mut rx = engine_rx;
        while let Some(event) = rx.recv().await {
            match event {
                EngineEvent::ProviderRunRequested { kind, node_id } => {
                    recorded.lock().await.push(format!("relay:{kind:?}"));
                    // 与生产 forward 任务同构（mapping.rs spawn_engine_event_forward_task）：
                    // 每个事件 tokio::spawn 独立派发，接收循环绝不内联 await spawn——
                    // 否则 spawn 等 engine 锁（在途 run 持有）与 run 后续事件发送
                    // 阻塞互锁。接力事件必须真正 spawn run，「事件已发射」才是
                    // 真启动（k3 P0 审查发现的假绿）。ReviewOnly 事件不 spawn——
                    // 生产语义里它在有在途 run 时由 drain 分支让位（在途 run 的
                    // followups 自续评审，本测试 run-1 正是内联驱动了 reviewer），
                    // 串行 drain 下补 spawn 只会双驱动。
                    if matches!(kind, ProviderRunKind::WorkItemPlanSingleCandidateAuthor)
                        && let Some((run_context, outbound_tx)) = spawn_slot.lock().await.clone()
                    {
                        tokio::spawn(async move {
                            let _ = spawn_provider_run_from_event(
                                run_context,
                                kind,
                                node_id,
                                outbound_tx,
                            )
                            .await;
                        });
                    }
                }
                EngineEvent::StageChange { stage } => {
                    recorded.lock().await.push(format!("stage:{stage}"));
                }
                EngineEvent::Error { message } => {
                    recorded.lock().await.push(format!("error:{message}"));
                }
                _ => {}
            }
        }
    });

    let author_starts = Arc::new(AtomicUsize::new(0));
    let author = Arc::new(ScAuthorStubProvider {
        output: single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        starts: author_starts.clone(),
        held: Mutex::new(Vec::new()),
    });
    let review_starts = Arc::new(AtomicUsize::new(0));
    let reviewer = Arc::new(ScReviseReviewProvider {
        starts: review_starts.clone(),
        held: Mutex::new(Vec::new()),
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, author);
    registry.register(ProviderName::Codex, reviewer);
    let (context, mut outbound_rx, run_context, outbound_tx) =
        sc_context_with_registry(&fixture, registry);
    *relay_spawn.lock().await = Some((run_context, outbound_tx));
    let outbound_errors = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorded_outbound = outbound_errors.clone();
    let drain_outbound = tokio::spawn(async move {
        while let Some(control) = outbound_rx.recv().await {
            let OutboundControl::Text(text) = control else {
                continue;
            };
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                && value["type"] == "error"
            {
                recorded_outbound
                    .lock()
                    .await
                    .push(value["message"].to_string());
            }
        }
    });

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: review_enabled_provider_config(),
            reviewer_enabled: true,
        },
    )
    .await;

    // 等修订接力事件出现：revise 路由已完成委托，followups 已回到循环判定。
    let relay_observed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if relay_kinds
                .lock()
                .await
                .iter()
                .any(|kind| kind == "relay:WorkItemPlanSingleCandidateAuthor")
            {
                break;
            }
            // sleep 而非纯 yield_now 自旋：current_thread 运行时下永真自旋会
            // 饿死计时器，令本 timeout 永不触发（挂死而非失败）——与下方
            // relay_started 循环同一教训（PIB 终收：夹具基线 fail-closed 提前
            // 终止 run 时，relay 事件不出现，此处曾 300s 挂死而非 3s 失败）。
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    if relay_observed.is_err() {
        let kinds = relay_kinds.lock().await.clone();
        let errors = outbound_errors.lock().await.clone();
        let (phase, stage) = {
            let engine = fixture.engine.lock().await;
            (
                format!("{:?}", engine.session.single_candidate_phase),
                format!("{:?}", engine.session.stage),
            )
        };
        panic!(
            "SC revise route must emit the WorkItemPlanSingleCandidateAuthor relay; \
             author_starts={} review_starts={} phase={phase} stage={stage} events={kinds:?} outbound_errors={errors:?}",
            author_starts.load(Ordering::SeqCst),
            review_starts.load(Ordering::SeqCst),
        );
    }
    // k3 P0 真启动锚：接力事件必须实际启动 provider（author 第二次 start），
    // 而不是只发射事件后死在 reserve 键碰撞上（修复前 phase==Generate 启发式
    // 复用 run1 的 :0 键 → already reserved → Message 失败 + Error 广播）。
    let relay_started = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if author_starts.load(Ordering::SeqCst) >= 2 {
                break;
            }
            // sleep 而非纯 yield_now 自旋：current_thread 运行时下永真自旋会
            // 饿死计时器，令本 timeout 永不触发（挂死而非失败）。
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    if relay_started.is_err() {
        let kinds = relay_kinds.lock().await.clone();
        let errors = outbound_errors.lock().await.clone();
        let (phase, stage) = {
            let engine = fixture.engine.lock().await;
            (
                format!("{:?}", engine.session.single_candidate_phase),
                format!("{:?}", engine.session.stage),
            )
        };
        panic!(
            "SC author relay run must actually start the provider (second author start); \
             author_starts={} review_starts={} phase={phase} stage={stage} events={kinds:?} outbound_errors={errors:?}",
            author_starts.load(Ordering::SeqCst),
            review_starts.load(Ordering::SeqCst),
        );
    }
    // 让修复前的误燃窗口（第二轮 reviewer start）有机会发生。
    let _ = tokio::time::timeout(std::time::Duration::from_millis(400), async {
        while review_starts.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;

    assert_eq!(
        review_starts.load(Ordering::SeqCst),
        1,
        "委托返修后 followups 不得误燃第二轮 reviewer（修复前此处为 2：候选已裁决 \
         revise，第二轮握手随后被接力 supersede 取消，现场 handshake failed: cancelled）"
    );
    assert_eq!(
        author_starts.load(Ordering::SeqCst),
        2,
        "首轮恰一次 + 接力 run 真启动恰一次；接力是唯一接续路径"
    );
    assert!(
        relay_kinds
            .lock()
            .await
            .iter()
            .any(|kind| kind == "relay:WorkItemPlanSingleCandidateAuthor"),
        "委托仍必须发射全名 kind 的 SC author 接力事件（唯一接续路径）"
    );
    assert!(
        outbound_errors.lock().await.is_empty(),
        "接力链路不得产生任何 Error 广播（键碰撞死亡/败者虚假失败都会在此现形）：{:?}",
        outbound_errors.lock().await
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session after relay start");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Generate),
        "接力 run 在途时 durable phase 必须停留 Generate"
    );
    assert!(
        durable
            .provider_start_ledger
            .iter()
            .any(|entry| entry.provider_start_idempotency_key
                == format!("single_candidate_author:{}:1", durable.id)
                && entry.started),
        "预领的接力键 :1 必须已在 ledger（k3 P0 原子预领）"
    );
    assert!(
        durable
            .repair_reservation
            .as_ref()
            .is_some_and(|reservation| {
                reservation.provider_start_idempotency_key
                    == format!("single_candidate_author:{}:1", durable.id)
                    && reservation.state
                        == crate::product::work_item_plan_policy::RepairReservationState::ProviderStarted
            }),
        "接力 run 真启动后预领 reservation 必须推进为 ProviderStarted"
    );
    assert!(
        fixture.manager.active_run().await.is_some(),
        "接力 run 的在途 provider 由 manager 持有（run-1 已被 supersede 退场）"
    );
    // timeline 检查读 durable 副本：接力 run 在途驱动 held provider 时持有 engine
    // 锁，此刻取内存锁会永久阻塞（挂死而非失败）；abort 则会制造取消足迹污染
    // 断言。durable timeline 由引擎在节点变更时同步落盘，口径与内存一致。
    let durable_nodes = fixture
        .lifecycle
        .load_timeline_nodes(&fixture.record.id)
        .expect("load durable timeline nodes");
    assert_eq!(
        durable_nodes
            .iter()
            .filter(|node| {
                node.status == crate::web::workspace_ws_types::TimelineNodeStatus::Failed
            })
            .count(),
        0,
        "接力链路不得产生虚假 Failed 节点（k3 P0 键碰撞死亡 / P2 败者虚假失败）"
    );
    let _ = fixture.manager.abort_active_run().await;

    drain_engine.abort();
    drain_outbound.abort();
}

/// 红锚2（handoff 钉死，防候选1/5回潮）：在途 run 存在时，注入 SC author 接力
/// （node_id=ReviewerRun 节点，与在途 run 注册节点不同 → 同节点去重不命中）仍
/// 必须走 handler supersede：取消在途 token 并由 run-2 注册替换。候选6 只收窄
/// followups 自续循环，不得把 engine 接力扩面成 drain。
#[tokio::test]
async fn single_candidate_author_relay_still_supersedes_in_flight_run() {
    let (fixture, engine_rx) =
        ProviderRunFixture::new_with_engine_rx(WorkItemPlanFlowKind::SingleCandidate);
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
    let (old_command_tx, _old_command_rx) = mpsc::channel(1);
    let old_cancel = CancellationToken::new();
    fixture
        .manager
        .test_set_active_run(ActiveRun {
            kind: ProviderRunKind::ReviewOnly,
            id: 4242,
            token: 424_242,
            run_incarnation: uuid::Uuid::new_v4().to_string(),
            node_id: Some("single-candidate-author-node".to_string()),
            cancel: old_cancel.clone(),
            command_tx: old_command_tx,
            pending_choices: Arc::new(std::sync::Mutex::new(Vec::new())),
            lease_epoch: 0,
        })
        .await;
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    run_context.manager = fixture.manager.clone();
    let (outbound_tx, outbound_rx) = mpsc::channel(8);
    let forward = spawn_engine_event_forward_task(engine_rx, outbound_tx, Some(run_context));

    fixture
        .engine_tx
        .send(EngineEvent::ProviderRunRequested {
            kind: ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
            node_id: Some("reviewer-run-node".to_string()),
        })
        .await
        .expect("queue SC author relay");

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if starts.load(Ordering::SeqCst) == 1 && old_cancel.is_cancelled() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("SC author relay must still supersede the in-flight run");
    assert!(
        old_cancel.is_cancelled(),
        "SC 作者接力必须仍取消在途 run（候选6 不得把接力扩面成 drain）"
    );
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    let active = fixture
        .manager
        .active_run()
        .await
        .expect("relay run must replace the in-flight run in the manager");
    assert_ne!(
        active.id, 4242,
        "manager 必须由 run-2 替换在途 run-1（哨兵 id 不得残留）"
    );

    let _ = fixture.manager.abort_active_run().await;
    held_event_senders.lock().await.clear();
    drop(outbound_rx);
    forward.abort();
    let _ = forward.await;
}

/// 正向锚：stage=CrossReview 且 phase=Evaluate（待评审）时守卫必须放行 followups
/// 驱 reviewer；legacy 臂完全不受 SC 委托守卫影响。
#[tokio::test]
async fn single_candidate_followup_review_guard_only_breaks_on_delegated_generate() {
    let sc = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    {
        let mut engine = sc.engine.lock().await;
        engine.session.single_candidate_phase =
            Some(crate::product::models::SingleCandidatePhase::Evaluate);
        assert!(
            !engine.sc_author_rerun_delegated(),
            "phase=Evaluate 表示待驱评审，followups 必须放行 reviewer"
        );
        engine.session.single_candidate_phase =
            Some(crate::product::models::SingleCandidatePhase::Generate);
        assert!(
            engine.sc_author_rerun_delegated(),
            "phase=Generate 表示 author 重跑已委托接力，followups 必须让位"
        );
    }
    let legacy = ProviderRunFixture::new(WorkItemPlanFlowKind::Legacy);
    {
        let mut engine = legacy.engine.lock().await;
        engine.session.single_candidate_phase =
            Some(crate::product::models::SingleCandidatePhase::Generate);
        assert!(
            !engine.sc_author_rerun_delegated(),
            "legacy 流必须零改动——委托守卫仅限 SingleCandidate"
        );
    }
}
// ---------------------------------------------------------------------------
// F2（P0 真实链 workspace_session_0018 现场）：无附件 REST 反馈链的 SC 委托
// 返修接力在 engine/provider run 事件路由层被静默吞没。
//
// 现场：REST human-actions feedback → spawn HumanGateScManualRevision（注册
// 节点=仍开启的 human_confirm 门节点）→ 修订完成 → 复评 revise → policy 委托
// ProviderRunRequested{WorkItemPlanSingleCandidateAuthor}（emit 时活动节点仍=
// 同一门节点）→ from_event「同节点去重」命中 → drain（tracing::debug 零可见）
// → followups 同时按 phase=Generate 让位 → 双方都退出、无人驱动重跑 →
// durable 停留 (running, generate)：无门、无 run、零事件，会话永久卡 running。
// ---------------------------------------------------------------------------

/// F2 红锚1（drain 修复）：修订 run 注册节点与委托接力请求节点同为门节点时，
/// 同节点去重不得吞掉不同 kind 的接力——它是 0018 现场唯一被委托的接续路径。
#[tokio::test]
async fn sc_delegated_rerun_relay_is_not_drained_by_gate_node_kind_collision() {
    let (fixture, engine_rx) =
        ProviderRunFixture::new_with_engine_rx(WorkItemPlanFlowKind::SingleCandidate);
    persist_review_rounds(&fixture, 1);
    // 只 drain 不接力：委托接力由本测试手动发起（确定性复现 0018 碰撞）。
    let mut engine_rx = engine_rx;
    let drain_engine = tokio::spawn(async move {
        while engine_rx.recv().await.is_some() {}
    });

    let author_starts = Arc::new(AtomicUsize::new(0));
    let author = Arc::new(ScSequenceAuthorProvider {
        outputs: vec![single_candidate_markdown(&fixture.story_id, &fixture.design_id)],
        starts: author_starts.clone(),
        held: Mutex::new(Vec::new()),
    });
    let review_starts = Arc::new(AtomicUsize::new(0));
    let reviewer = Arc::new(ScSequenceReviewProvider {
        outputs: vec![sc_pass_review_output()],
        starts: review_starts.clone(),
        held: Mutex::new(Vec::new()),
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, author);
    registry.register(ProviderName::Codex, reviewer);
    let (context, _outbound_rx, run_context, outbound_tx) =
        sc_context_with_registry(&fixture, registry);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: review_enabled_provider_config(),
            reviewer_enabled: true,
        },
    )
    .await;

    // 等 SC Approval 门开（pass → EnterHumanGate/审批门，含 human gate snapshot）。
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let stage = {
                let engine = fixture.engine.lock().await;
                engine.session().stage.clone()
            };
            if stage == WorkspaceStage::HumanConfirm {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("SC approval gate must open after the passing review");

    // REST feedback 等价链：apply → TurnOpened → handler spawn（注册节点=门节点）。
    let effect = apply_human_gate_feedback(
        fixture.engine.clone(),
        HumanGateFeedbackInput {
            command_id: "f2-red-collision".to_string(),
            feedback: "请修订当前候选的验证计划".to_string(),
        },
    )
    .await;
    let HumanGateFeedbackEffect::TurnOpened { turn, prompt, .. } = effect else {
        panic!("human gate feedback must open a revision turn");
    };
    let gate_node = {
        let engine = fixture.engine.lock().await;
        engine
            .active_timeline_node_id()
            .expect("gate node must be active")
    };
    spawn_provider_run_from_handler(
        run_context.clone(),
        ProviderRunKind::HumanGateScManualRevision {
            turn_id: turn.turn_id,
            prompt,
        },
        outbound_tx.clone(),
    )
    .await
    .expect("revision run spawn must succeed");

    // 修订 run 在途（held provider 永不完成），注册节点必须是门节点（0018 现场）。
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if author_starts.load(Ordering::SeqCst) >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("revision run must reach its held author provider");
    let active = fixture
        .manager
        .active_run()
        .await
        .expect("held revision run must be registered");
    assert_eq!(
        active.node_id.as_deref(),
        Some(gate_node.as_str()),
        "修订 run 注册节点必须仍是 human_confirm 门节点（0018 现场前置）"
    );
    let superseded_token = active.token;
    let superseded_cancel = active.cancel.clone();

    // 委托接力：同节点、不同 kind（WorkItemPlanSingleCandidateAuthor）。
    let (relay_outbound_tx, _relay_outbound_rx) = mpsc::channel(8);
    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
        Some(gate_node),
        relay_outbound_tx,
    )
    .await
    .expect("delegated rerun relay must report spawn success");

    // 修复前：同节点去重把接力静默 drain（Ok(()) 零回执）——在途修订 run 既不被
    // supersede、也不注册接力 run，重跑永不启动（0018 卡死形态）。修复后：接力
    // 放行到 from_handler，取消在途 token 并登记新 run。
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let replaced = fixture
                .manager
                .active_run()
                .await
                .is_some_and(|run| run.token != superseded_token);
            if replaced && superseded_cancel.is_cancelled() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect(
        "delegated rerun relay must supersede the in-flight revision run and register          the rerun (silent drain leaves both untouched)",
    );
    assert!(
        superseded_cancel.is_cancelled(),
        "被接替的在途修订 run 必须收到取消（drain 路径不会取消任何 token）"
    );

    let _ = fixture.manager.abort_active_run().await;
    drain_engine.abort();
}

/// F2 红锚2（spawn 失败可见性）：委托态（SC phase=Generate、无在途 run、durable
/// running）会话的接力 spawn 失败必须有可见结果——回落人工门（durable
/// WaitingForHuman + 门节点摘要携带原因），不得静默滞留 running（0018 卡
/// 20+ 分钟实证）。
#[tokio::test]
async fn sc_delegated_rerun_relay_failure_falls_back_to_human_gate_durable() {
    let (fixture, engine_rx) =
        ProviderRunFixture::new_with_engine_rx(WorkItemPlanFlowKind::SingleCandidate);
    persist_review_rounds(&fixture, 1);
    let mut engine_rx = engine_rx;
    let drain_engine = tokio::spawn(async move {
        while engine_rx.recv().await.is_some() {}
    });

    let author_starts = Arc::new(AtomicUsize::new(0));
    let author = Arc::new(ScSequenceAuthorProvider {
        outputs: vec![
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
            // 修订候选必须与首轮不同（candidate 哈希变更），否则复评落回同一
            // review cycle 触发 scope 违规（真实链的 provider 输出天然不同）。
            single_candidate_markdown(&fixture.story_id, &fixture.design_id)
                .replacen("### Notes", "### Notes\n- 修订轮：收紧验证计划。", 1),
        ],
        starts: author_starts.clone(),
        held: Mutex::new(Vec::new()),
    });
    let review_starts = Arc::new(AtomicUsize::new(0));
    let reviewer = Arc::new(ScSequenceReviewProvider {
        outputs: vec![sc_pass_review_output(), sc_repairable_revise_output()],
        starts: review_starts.clone(),
        held: Mutex::new(Vec::new()),
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, author);
    registry.register(ProviderName::Codex, reviewer);
    let (context, _outbound_rx, run_context, outbound_tx) =
        sc_context_with_registry(&fixture, registry);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: review_enabled_provider_config(),
            reviewer_enabled: true,
        },
    )
    .await;

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let stage = {
                let engine = fixture.engine.lock().await;
                engine.session().stage.clone()
            };
            if stage == WorkspaceStage::HumanConfirm {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("SC approval gate must open after the passing review");

    let effect = apply_human_gate_feedback(
        fixture.engine.clone(),
        HumanGateFeedbackInput {
            command_id: "f2-red-fallback".to_string(),
            feedback: "请修订当前候选的验证计划".to_string(),
        },
    )
    .await;
    let HumanGateFeedbackEffect::TurnOpened { turn, prompt, .. } = effect else {
        panic!("human gate feedback must open a revision turn");
    };
    spawn_provider_run_from_handler(
        run_context,
        ProviderRunKind::HumanGateScManualRevision {
            turn_id: turn.turn_id,
            prompt,
        },
        outbound_tx,
    )
    .await
    .expect("revision run spawn must succeed");

    // 修订完成 → 复评 revise → policy 委托 → phase=Generate；修订 run 退场。
    // 这就是 0018 的卡死前置态：durable (running, generate)、无在途 run、门未开。
    let delegation_landed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let (phase, status, has_run) = {
                let engine = fixture.engine.lock().await;
                let durable = fixture
                    .lifecycle
                    .get_workspace_session(&fixture.record.id)
                    .expect("reload session");
                (
                    engine.session().single_candidate_phase.clone(),
                    durable.status.clone(),
                    fixture.manager.active_run().await.is_some(),
                )
            };
            if phase == Some(crate::product::models::SingleCandidatePhase::Generate)
                && status == crate::product::models::WorkspaceSessionStatus::Running
                && !has_run
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    if delegation_landed.is_err() {
        let engine = fixture.engine.lock().await;
        let durable = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("reload session");
        let has_run = fixture.manager.active_run().await.is_some();
        panic!(
            "delegation must durably land; phase={:?} stage={:?} status={:?} has_run={has_run} author_starts={} review_starts={} diagnostics={:?}",
            engine.session().single_candidate_phase,
            engine.session().stage,
            durable.status,
            author_starts.load(Ordering::SeqCst),
            review_starts.load(Ordering::SeqCst),
            durable.policy_diagnostics,
        );
    }

    // 无附件接力 spawn 失败（author provider 不在 registry）：0018 现场的
    // 「run 未启动且错误不可见」输入形态。
    let mut relay_registry = ProviderRegistry::new();
    relay_registry.register(ProviderName::Codex, Arc::new(PendingStartProvider {
        starts: Arc::new(AtomicUsize::new(0)),
        held_event_senders: Arc::new(Mutex::new(Vec::new())),
    }));
    let mut relay_context = ProviderRunContext::test_fixture(
        Arc::new(relay_registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    relay_context.manager = fixture.manager.clone();
    let (relay_outbound_tx, _relay_outbound_rx) = mpsc::channel(8);
    let relay_node = {
        let engine = fixture.engine.lock().await;
        engine.active_timeline_node_id()
    };
    let relay_result = spawn_provider_run_from_event(
        relay_context,
        ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
        relay_node,
        relay_outbound_tx,
    )
    .await;
    assert!(
        relay_result.is_err(),
        "relay spawn must surface the provider-unavailable failure"
    );

    // 修复前：错误只进丢弃通道，会话永久滞留 running（红锚）；修复后：回落
    // 人工门，durable WaitingForHuman + 新门节点摘要携带失败原因。
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let durable = fixture
                .lifecycle
                .get_workspace_session(&fixture.record.id)
                .expect("reload session");
            if durable.status == crate::product::models::WorkspaceSessionStatus::WaitingForHuman
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("delegated relay failure must durably fall back to the human gate");
    let stage = {
        let engine = fixture.engine.lock().await;
        engine.session().stage.clone()
    };
    assert_eq!(
        stage,
        WorkspaceStage::HumanConfirm,
        "接力失败回落后引擎必须停在可操作的人工门"
    );
    let durable_nodes = fixture
        .lifecycle
        .load_timeline_nodes(&fixture.record.id)
        .expect("load durable timeline nodes");
    let gate_summary = durable_nodes
        .iter()
        .rev()
        .find(|node| {
            node.node_type == crate::web::workspace_ws_types::TimelineNodeType::HumanConfirm
                && node.status == crate::web::workspace_ws_types::TimelineNodeStatus::Active
        })
        .and_then(|node| node.summary.clone())
        .unwrap_or_default();
    assert!(
        gate_summary.contains("返修接力"),
        "回落门节点摘要必须携带失败原因，got: {gate_summary}"
    );

    drain_engine.abort();
}

/// SC author 桩：按序产出 outputs，随后挂起（事件端存活但永不完成）。
struct ScSequenceAuthorProvider {
    outputs: Vec<String>,
    starts: Arc<AtomicUsize>,
    held: Mutex<Vec<mpsc::Sender<ProviderEvent>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScSequenceAuthorProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        if let Some(output) = self.outputs.get(start - 1).cloned() {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output, None,
                )))
                .await;
        } else {
            self.held.lock().await.push(event_tx);
        }
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

/// SC reviewer 桩：按序产出 verdict JSON（结构化契约哨兵对齐），随后挂起。
struct ScSequenceReviewProvider {
    outputs: Vec<String>,
    starts: Arc<AtomicUsize>,
    held: Mutex<Vec<mpsc::Sender<ProviderEvent>>>,
}

fn sc_pass_review_output() -> String {
    serde_json::json!({
        "verdict": "pass",
        "review_scope": "outline",
        "generation_round_id": "round-1",
        "summary": "候选自洽，等待人工审批",
        "findings": [],
    })
    .to_string()
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScSequenceReviewProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        if let Some(payload) = self.outputs.get(start - 1).cloned() {
            let contract = input.structured_output_contract.clone();
            let output = match contract.as_ref() {
                Some(contract) => structured_output_sentinel(
                    &contract.nonce,
                    &serde_json::from_str(&payload).expect("review fixture payload"),
                ),
                None => payload,
            };
            let completion = ProviderCompletion::from_output(output, contract.as_ref(), None);
            let _ = event_tx.send(ProviderEvent::Completed(completion)).await;
        } else {
            self.held.lock().await.push(event_tx);
        }
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
