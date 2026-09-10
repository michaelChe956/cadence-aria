use super::*;
use crate::cross_cutting::provider_adapter::structured_output_sentinel;
use crate::cross_cutting::streaming_provider::ProviderCompletion;
use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
use crate::product::models::WorkspaceType;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::single_candidate_provider_run::{ProviderRunFixture, single_candidate_markdown};

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
) -> (WorkspaceInboundContext, mpsc::Receiver<OutboundControl>) {
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(registry),
        engine: fixture.engine.clone(),
        current_run: fixture.current_run.clone(),
        workspace_runs: fixture.workspace_runs.clone(),
        session_id: fixture.record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths: fixture.app_paths.clone(),
        session_record: fixture.record.clone(),
    };
    let root = fixture.root_path();
    let (outbound_tx, outbound_rx) = mpsc::channel(64);
    (
        WorkspaceInboundContext {
            app_state: WebAppState::new(
                root.clone(),
                crate::web::runtime::WebRuntime::new_fake(root),
            ),
            engine: fixture.engine.clone(),
            run_context,
            outbound_tx,
            current_run: fixture.current_run.clone(),
            workspace_runs: fixture.workspace_runs.clone(),
            session_id: fixture.record.id.clone(),
        },
        outbound_rx,
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
    let drain_engine = tokio::spawn(async move {
        let mut rx = engine_rx;
        while let Some(event) = rx.recv().await {
            match &event {
                EngineEvent::ProviderRunRequested { kind, .. } => {
                    recorded.lock().await.push(format!("relay:{kind:?}"));
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
    let (context, mut outbound_rx) = sc_context_with_registry(&fixture, registry);
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
            tokio::task::yield_now().await;
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
        1,
        "首轮 SC author 只应驱动一次；返修重跑由接力 run 承担"
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
        fixture
            .workspace_runs
            .run(&fixture.record.id)
            .await
            .is_none(),
        "run 任务必须在让位接力后退场（registry 不得残留 run-1）"
    );

    let _ = abort_active_run(
        &fixture.current_run,
        &fixture.workspace_runs,
        &fixture.record.id,
    )
    .await;
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
    // token/id 用哨兵值，避免与进程级 NEXT_ACTIVE_RUN_TOKEN / 每 context run_id
    // 计数（并行测试首分配可能恰为 1）碰撞，使「注册替换」判定确定。
    let old_run = WorkspaceActiveRun {
        id: 4242,
        token: 424_242,
        node_id: Some("single-candidate-author-node".to_string()),
        cancel: CancellationToken::new(),
        command_tx: old_command_tx,
        pending_choice_ids: Arc::new(Mutex::new(std::collections::HashSet::new())),
    };
    let old_cancel = old_run.cancel.clone();
    fixture
        .workspace_runs
        .insert(fixture.record.id.clone(), old_run)
        .await;
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(registry),
        engine: fixture.engine.clone(),
        current_run: fixture.current_run.clone(),
        workspace_runs: fixture.workspace_runs.clone(),
        session_id: fixture.record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths: fixture.app_paths.clone(),
        session_record: fixture.record.clone(),
    };
    let (outbound_tx, outbound_rx) = mpsc::channel(8);
    let forward = spawn_engine_event_forward_task(
        engine_rx,
        outbound_tx,
        fixture.record.id.clone(),
        fixture.workspace_runs.clone(),
        Some(run_context),
    );

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
        .workspace_runs
        .run(&fixture.record.id)
        .await
        .expect("relay run must replace the in-flight run in the registry");
    assert_ne!(
        active.id, 4242,
        "registry 必须由 run-2 替换在途 run-1（哨兵 id 不得残留）"
    );

    let _ = abort_active_run(
        &fixture.current_run,
        &fixture.workspace_runs,
        &fixture.record.id,
    )
    .await;
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
