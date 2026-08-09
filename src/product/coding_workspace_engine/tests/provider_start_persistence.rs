use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use super::provider_execution_context::{CapturingProjectionProvider, review_plan_defect_output};
use super::*;

#[test]
fn permission_wait_timeout_preserves_interaction_without_retry_budget() {
    let outcome = classify_provider_failure(&CodingWorkspaceEngineError::ProviderStream(
        "permission timeout".to_string(),
    ));
    assert!(outcome.is_interaction_wait());
    assert!(!outcome.is_retryable());

    let outcome = classify_provider_failure(&CodingWorkspaceEngineError::ProviderStream(
        "choice timeout".to_string(),
    ));
    assert!(outcome.is_interaction_wait());
    assert!(!outcome.is_retryable());
}
use crate::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestData, ChoiceRequestSource, ProviderCompletion,
};
use crate::product::coding_models::CodingChoiceGateStatus;
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

struct CompletedInvocationProvider;

struct PartialThenFailedInvocationProvider;

struct ChoiceThenWaitInvocationProvider;

struct ChoiceThenTextInvocationProvider;

#[derive(Default)]
struct StartIoInvocationProvider {
    starts: AtomicUsize,
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

#[async_trait::async_trait]
impl StreamingProviderAdapter for CompletedInvocationProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    "completed invocation evidence".to_string(),
                    Some("completed-session".to_string()),
                )))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PartialThenFailedInvocationProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: "partial invocation evidence".to_string(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Failed {
                    message: "connection reset by peer".to_string(),
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceThenWaitInvocationProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, mut command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(provider_choice_request()))
                .await;
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = command_rx.recv() => {}
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceThenTextInvocationProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(provider_choice_request()))
                .await;
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: "must not continue before choice".to_string(),
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for StartIoInvocationProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Err(ProviderAdapterError::execution_failed(
            None,
            "",
            "Text file busy",
            0,
        ))
    }
}

#[tokio::test]
async fn completed_provider_invocation_persists_retrievable_raw_output_ref() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_invocation_completed".to_string()),
        )
        .expect("role run");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let provider = CompletedInvocationProvider;
    let (legacy_input, input) = provider_invocation_inputs(&attempt);
    let provider_name = ProviderName::Codex;
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let outcome = engine
        .run_provider_stream_invocation(CodingProviderStreamRun {
            attempt: &attempt,
            node_id: "coding_invocation_completed",
            role_run: Some(&role_run),
            provider: &provider,
            legacy_input: &legacy_input,
            input,
            provider_name: &provider_name,
            provider_role: CodingProviderRole::Coder,
            command_rx: &mut command_rx,
            allow_legacy_stream_fallback: false,
            timeout: None,
            timeout_reason_code: None,
            suppress_failure_side_effects: false,
            validated_input: None,
        })
        .await;

    assert!(matches!(
        outcome,
        ProviderInvocationOutcome::Completed(ProviderStreamOutcome { full_output, .. })
            if full_output == "completed invocation evidence"
    ));
    assert_role_run_raw_output(&store, &attempt, &role_run, "completed invocation evidence");
}

#[tokio::test]
async fn failed_provider_invocation_persists_in_memory_partial_output_to_role_run_ref() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_invocation_partial_failure".to_string()),
        )
        .expect("role run");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let provider = PartialThenFailedInvocationProvider;
    let (legacy_input, input) = provider_invocation_inputs(&attempt);
    let provider_name = ProviderName::Codex;
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let outcome = engine
        .run_provider_stream_invocation(CodingProviderStreamRun {
            attempt: &attempt,
            node_id: "coding_invocation_partial_failure",
            role_run: Some(&role_run),
            provider: &provider,
            legacy_input: &legacy_input,
            input,
            provider_name: &provider_name,
            provider_role: CodingProviderRole::Coder,
            command_rx: &mut command_rx,
            allow_legacy_stream_fallback: false,
            timeout: None,
            timeout_reason_code: None,
            suppress_failure_side_effects: false,
            validated_input: None,
        })
        .await;

    assert!(matches!(
        outcome,
        ProviderInvocationOutcome::RetryableTransport {
            failure: RetryableProviderFailure::ConnectionInterrupted,
            partial_output,
            ..
        } if partial_output == "partial invocation evidence"
    ));
    assert_role_run_raw_output(&store, &attempt, &role_run, "partial invocation evidence");
}

#[tokio::test]
async fn choice_request_then_timeout_stays_non_retryable_with_open_choice_gate() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_invocation_choice_timeout".to_string()),
        )
        .expect("role run");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let provider = ChoiceThenWaitInvocationProvider;
    let (legacy_input, input) = provider_invocation_inputs(&attempt);
    let provider_name = ProviderName::Codex;
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let invocation = engine.run_provider_stream_invocation(CodingProviderStreamRun {
        attempt: &attempt,
        node_id: "coding_invocation_choice_timeout",
        role_run: Some(&role_run),
        provider: &provider,
        legacy_input: &legacy_input,
        input,
        provider_name: &provider_name,
        provider_role: CodingProviderRole::Coder,
        command_rx: &mut command_rx,
        allow_legacy_stream_fallback: false,
        timeout: Some(Duration::from_secs(1)),
        timeout_reason_code: None,
        suppress_failure_side_effects: false,
        validated_input: None,
    });
    tokio::pin!(invocation);
    tokio::select! {
        biased;
        () = wait_for_open_provider_choice(&store, &attempt) => {}
        outcome = &mut invocation => {
            panic!("provider invocation completed before choice gate opened: {outcome:?}");
        }
    }
    let outcome = invocation.await;

    assert!(matches!(
        outcome,
        ProviderInvocationOutcome::NonRetryable {
            ref reason_code,
            interaction_wait: true,
            ..
        } if reason_code == "choice_timeout"
    ));
    assert_open_provider_choice(&store, &attempt);
    assert_role_run_raw_output(&store, &attempt, &role_run, "");
}

#[tokio::test]
async fn unresolved_provider_choice_stays_non_retryable_interaction_wait() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_invocation_choice_unresolved".to_string()),
        )
        .expect("role run");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let provider = ChoiceThenTextInvocationProvider;
    let (legacy_input, input) = provider_invocation_inputs(&attempt);
    let provider_name = ProviderName::Codex;
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let outcome = engine
        .run_provider_stream_invocation(CodingProviderStreamRun {
            attempt: &attempt,
            node_id: "coding_invocation_choice_unresolved",
            role_run: Some(&role_run),
            provider: &provider,
            legacy_input: &legacy_input,
            input,
            provider_name: &provider_name,
            provider_role: CodingProviderRole::Coder,
            command_rx: &mut command_rx,
            allow_legacy_stream_fallback: false,
            timeout: None,
            timeout_reason_code: None,
            suppress_failure_side_effects: false,
            validated_input: None,
        })
        .await;

    assert!(matches!(
        outcome,
        ProviderInvocationOutcome::NonRetryable {
            ref reason_code,
            interaction_wait: true,
            ..
        } if reason_code == "provider_choice_unresolved"
    ));
    assert_open_provider_choice(&store, &attempt);
    assert_role_run_raw_output(
        &store,
        &attempt,
        &role_run,
        "must not continue before choice",
    );
}

#[tokio::test]
async fn modern_start_io_invocation_keeps_typed_error_and_raw_output_ref() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_invocation_start_io".to_string()),
        )
        .expect("role run");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let provider = StartIoInvocationProvider::default();
    let (legacy_input, input) = provider_invocation_inputs(&attempt);
    let provider_name = ProviderName::Codex;
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let outcome = engine
        .run_provider_stream_invocation(CodingProviderStreamRun {
            attempt: &attempt,
            node_id: "coding_invocation_start_io",
            role_run: Some(&role_run),
            provider: &provider,
            legacy_input: &legacy_input,
            input,
            provider_name: &provider_name,
            provider_role: CodingProviderRole::Coder,
            command_rx: &mut command_rx,
            allow_legacy_stream_fallback: false,
            timeout: None,
            timeout_reason_code: None,
            suppress_failure_side_effects: false,
            validated_input: None,
        })
        .await;

    assert!(matches!(
        outcome,
        ProviderInvocationOutcome::RetryableTransport {
            failure: RetryableProviderFailure::StartIo,
            ref reason_code,
            ref message,
            ref partial_output,
        } if reason_code == "provider_start_io"
            && message.contains("ProviderExecutionFailed")
            && message.contains("Text file busy")
            && partial_output.is_empty()
    ));
    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    assert_role_run_raw_output(&store, &attempt, &role_run, "");
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
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
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
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

fn provider_invocation_inputs(
    attempt: &CodingExecutionAttempt,
) -> (AdapterInput, StreamingProviderInput) {
    let worktree = attempt.worktree_path.clone().expect("worktree path");
    let legacy_input = AdapterInput {
        provider_type: ProviderType::Codex,
        role: AdapterRole::Executor,
        worktree_path: Some(worktree.to_string_lossy().to_string()),
        provider_stream_log_dir: None,
        prompt: "provider invocation evidence".to_string(),
        context_files: Vec::new(),
        output_schema: "coding_workspace_markdown".to_string(),
        timeout: 30,
        max_retries: 0,
    };
    let input = streaming_input_from_adapter(
        &legacy_input,
        worktree,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
    );
    (legacy_input, input)
}

fn provider_choice_request() -> ChoiceRequestData {
    ChoiceRequestData {
        id: "provider_choice_0001".to_string(),
        prompt: "Choose the next action".to_string(),
        options: vec![ChoiceOptionData {
            id: "continue".to_string(),
            label: "Continue".to_string(),
            description: None,
        }],
        allow_multiple: false,
        allow_free_text: false,
        questions: Vec::new(),
        source: ChoiceRequestSource::ProviderChoice,
    }
}

fn assert_open_provider_choice(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt) {
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::WaitingForHuman);
    let gates = store
        .list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open choice gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].choice_id, "provider_choice_0001");
    assert_eq!(gates[0].status, CodingChoiceGateStatus::Open);
}

async fn wait_for_open_provider_choice(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    loop {
        let persisted = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("persisted attempt");
        let gates = store
            .list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open choice gates");
        if persisted.status == CodingAttemptStatus::WaitingForHuman
            && gates.iter().any(|gate| {
                gate.choice_id == "provider_choice_0001"
                    && gate.status == CodingChoiceGateStatus::Open
            })
        {
            return;
        }
        tokio::task::yield_now().await;
    }
}

fn assert_role_run_raw_output(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role_run: &CodingRoleRun,
    expected: &str,
) {
    let persisted = store
        .get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
        )
        .expect("persisted role run");
    assert_eq!(persisted.raw_provider_output_refs.len(), 1);
    let relative = persisted.raw_provider_output_refs[0]
        .strip_prefix("provider-raw/")
        .expect("provider raw output ref");
    let path = store
        .provider_raw_output_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .join(relative);
    assert_eq!(
        fs::read_to_string(path).expect("raw provider output"),
        expected
    );
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
