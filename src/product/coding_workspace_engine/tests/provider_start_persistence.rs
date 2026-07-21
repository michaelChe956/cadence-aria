use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::provider_execution_context::{CapturingProjectionProvider, review_plan_defect_output};
use super::*;
use crate::cross_cutting::streaming_provider::ProviderCompletion;
use crate::product::work_item_projection::ReviewerWorkItemProjection;
use tokio::sync::Notify;

#[derive(Default)]
struct ProviderStartProbe {
    cancelled: AtomicBool,
    completion_delivered: AtomicBool,
}

struct ProviderStartPersistenceFixture {
    _root: tempfile::TempDir,
    store: CodingAttemptStore,
    engine: CodingWorkspaceEngine,
    attempt: CodingExecutionAttempt,
    reviewer_projection: ReviewerWorkItemProjection,
}

struct ModernProviderStartPersistenceFailure {
    store: CodingAttemptStore,
    attempt: CodingExecutionAttempt,
    output: String,
    probe: Arc<ProviderStartProbe>,
}

struct LegacyProviderStartPersistenceFailure {
    store: CodingAttemptStore,
    attempt: CodingExecutionAttempt,
    output: String,
    probe: Arc<ProviderStartProbe>,
}

#[derive(Default)]
struct ParentCancellationProbe {
    entered: Notify,
    cancelled: AtomicBool,
}

struct ParentCancellationProvider {
    probe: Arc<ParentCancellationProbe>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ModernProviderStartPersistenceFailure {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        poison_reviewer_event_log(&self.store, &self.attempt);
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        spawn_modern_output(event_tx, self.output.clone(), cancel, self.probe.clone());
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for LegacyProviderStartPersistenceFailure {
    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        poison_reviewer_event_log(&self.store, &self.attempt);
        let (chunk_tx, chunk_rx) = mpsc::channel(1);
        spawn_legacy_output(chunk_tx, self.output.clone(), cancel, self.probe.clone());
        Ok(chunk_rx)
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ParentCancellationProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.probe.entered.notify_one();
        cancel.cancelled().await;
        self.probe.cancelled.store(true, Ordering::SeqCst);
        Err(ProviderAdapterError::command_missing(
            "provider cancelled by parent token",
        ))
    }
}

#[tokio::test]
async fn coding_plan_repair_modern_provider_start_persistence_failure_aborts_before_output() {
    let fixture = provider_start_persistence_fixture().await;
    let probe = Arc::new(ProviderStartProbe::default());
    let provider = ModernProviderStartPersistenceFailure {
        store: fixture.store.clone(),
        attempt: fixture.attempt.clone(),
        output: review_plan_defect_output(),
        probe: probe.clone(),
    };

    let result = fixture
        .engine
        .execute_code_review(&fixture.attempt, &provider)
        .await;

    assert_provider_start_persistence_failure(
        result,
        &fixture.reviewer_projection,
        &probe,
        "modern",
    )
    .await;
}

#[tokio::test]
async fn coding_plan_repair_legacy_provider_start_persistence_failure_aborts_before_output() {
    let fixture = provider_start_persistence_fixture().await;
    let probe = Arc::new(ProviderStartProbe::default());
    let provider = LegacyProviderStartPersistenceFailure {
        store: fixture.store.clone(),
        attempt: fixture.attempt.clone(),
        output: review_plan_defect_output(),
        probe: probe.clone(),
    };

    let result = fixture
        .engine
        .execute_code_review(&fixture.attempt, &provider)
        .await;

    assert_provider_start_persistence_failure(
        result,
        &fixture.reviewer_projection,
        &probe,
        "legacy",
    )
    .await;
}

#[tokio::test]
async fn coding_provider_stream_uses_engine_cancellation_child_token() {
    let fixture = provider_start_persistence_fixture().await;
    let cancellation = CancellationToken::new();
    let (event_tx, _event_rx) = mpsc::channel(64);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx)
            .with_cancellation(cancellation.clone());
    let probe = Arc::new(ParentCancellationProbe::default());
    let provider = ParentCancellationProvider {
        probe: probe.clone(),
    };

    let execute = engine.execute_code_review(&fixture.attempt, &provider);
    let cancel_parent = async {
        tokio::time::timeout(Duration::from_millis(250), probe.entered.notified())
            .await
            .expect("provider did not receive cancellation token");
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(execute, cancel_parent);

    assert!(result.is_err(), "cancelled provider run must stop");
    assert!(
        probe.cancelled.load(Ordering::SeqCst),
        "provider must observe engine parent cancellation"
    );
}

async fn provider_start_persistence_fixture() -> ProviderStartPersistenceFixture {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    init_test_git_repo(&worktree);
    let head = git_stdout(&worktree, &["rev-parse", "HEAD"]);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: head.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let mut attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    attempt.head_commit = Some(head);
    attempt.stage = CodingExecutionStage::Coding;
    store.save_coding_attempt(&attempt).expect("save attempt");
    let (event_tx, _event_rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let coder = CapturingProjectionProvider::new(
        serde_json::json!({"plan_defect_findings": []}).to_string(),
    );
    let attempt = engine
        .execute_coding(&attempt, &coder, &CodingExecutionContext::default())
        .await
        .expect("seed completed coding provider run");
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("reviewable change");
    let unit_run = store
        .get_active_unit_run(&attempt)
        .expect("active unit run");
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("plan lineage");
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &unit_run.projection_bundle_id)
        .expect("projection bundle");

    ProviderStartPersistenceFixture {
        _root: root,
        store,
        engine,
        attempt,
        reviewer_projection: bundle.reviewer_projection,
    }
}

fn poison_reviewer_event_log(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt) {
    let role_run = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs")
        .into_iter()
        .find(|run| {
            run.role == CodingProviderRole::CodeReviewer
                && run.status == CodingRoleRunStatus::Running
        })
        .expect("running reviewer role run");
    let path = store.role_run_event_log_path(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        &role_run.id,
    );
    fs::remove_file(&path).expect("remove provider prompt event log");
    fs::create_dir(&path).expect("poison provider start event path");
}

fn spawn_modern_output(
    event_tx: mpsc::Sender<ProviderEvent>,
    output: String,
    cancel: CancellationToken,
    probe: Arc<ProviderStartProbe>,
) {
    let cancel_probe = probe.clone();
    let observed_cancel = cancel.clone();
    tokio::spawn(async move {
        observed_cancel.cancelled().await;
        cancel_probe.cancelled.store(true, Ordering::SeqCst);
    });
    tokio::spawn(async move {
        if event_tx
            .send(ProviderEvent::TextDelta {
                content: output.clone(),
            })
            .await
            .is_err()
        {
            return;
        }
        if event_tx
            .send(ProviderEvent::Completed(ProviderCompletion::plain(
                output, None,
            )))
            .await
            .is_ok()
        {
            probe.completion_delivered.store(true, Ordering::SeqCst);
        }
    });
}

fn spawn_legacy_output(
    chunk_tx: mpsc::Sender<StreamChunk>,
    output: String,
    cancel: CancellationToken,
    probe: Arc<ProviderStartProbe>,
) {
    let cancel_probe = probe.clone();
    let observed_cancel = cancel.clone();
    tokio::spawn(async move {
        observed_cancel.cancelled().await;
        cancel_probe.cancelled.store(true, Ordering::SeqCst);
    });
    tokio::spawn(async move {
        if chunk_tx
            .send(StreamChunk::Text(output.clone()))
            .await
            .is_err()
        {
            return;
        }
        if chunk_tx
            .send(StreamChunk::Done {
                full_output: output,
            })
            .await
            .is_ok()
        {
            probe.completion_delivered.store(true, Ordering::SeqCst);
        }
    });
}

async fn assert_provider_start_persistence_failure(
    result: Result<CodeReviewReport, CodingWorkspaceEngineError>,
    reviewer_projection: &ReviewerWorkItemProjection,
    probe: &ProviderStartProbe,
    mode: &str,
) {
    match result {
        Ok(report) => panic!(
            "{mode} ProviderStart persistence failure was swallowed; provider output reached {:?}",
            code_review_flow_decision(&report, reviewer_projection)
        ),
        Err(error) => assert!(
            error.to_string().contains("role-run-events")
                && error.to_string().contains("Is a directory"),
            "unexpected {mode} ProviderStart error: {error}"
        ),
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        while !probe.cancelled.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{mode} provider cancellation was not observed"));
    assert!(
        !probe.completion_delivered.load(Ordering::SeqCst),
        "{mode} provider completion was consumed after ProviderStart persistence failed"
    );
}
