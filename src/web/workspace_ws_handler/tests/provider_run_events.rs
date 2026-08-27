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
