use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use super::provider_execution_context::{CapturingProjectionProvider, review_plan_defect_output};
use super::*;

mod choice_receipt_gate;

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

// 组 3：429 失败轮经引擎分类为 attempt 级可重试（upstream 429/5xx 类）。
// 错误取组 1 透传形态：adapter 以 parse_error 原样携带上游 429 文案（
// "exceeded retry limit, last status: 429 Too Many Requests"，spike T2-resume-writes）。
#[test]
fn provider_retry_classifier_maps_turn_failed_429_to_retryable_upstream() {
    let error = CodingWorkspaceEngineError::ProviderAdapter(ProviderAdapterError::parse_error(
        "exceeded retry limit, last status: 429 Too Many Requests",
        "",
        "",
    ));
    let classification = classify_provider_failure(&error);
    assert!(
        classification.is_retryable(),
        "429 turn failure must be attempt-level retryable: {classification:?}"
    );
    match classification {
        crate::product::coding_workspace_engine::provider_retry::ProviderFailureClassification::Retryable {
            failure,
            ..
        } => assert!(
            matches!(
                failure,
                crate::product::coding_workspace_engine::provider_retry::RetryableProviderFailure::Upstream5xx { status: 429 }
            ),
            "expected upstream 429 classification, got {failure:?}"
        ),
        other => panic!("expected retryable classification, got {other:?}"),
    }
}

#[derive(Default)]
struct ProviderStartProbe {
    cancelled: AtomicBool,
    completion_delivered: AtomicBool,
}

pub(super) struct ProviderStartPersistenceFixture {
    _root: tempfile::TempDir,
    pub(super) store: CodingAttemptStore,
    engine: CodingWorkspaceEngine,
    pub(super) attempt: CodingExecutionAttempt,
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
            native_session_id: None,
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
            native_session_id: None,
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
            native_session_id: None,
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
            native_session_id: None,
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
            native_session_id: None,
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

pub(super) async fn provider_start_persistence_fixture() -> ProviderStartPersistenceFixture {
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
            target_snapshot: None,
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let mut attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running attempt");
    attempt.head_commit = Some(head);
    attempt.stage = CodingExecutionStage::Coding;
    store
        .write_coding_attempt_for_test(&attempt)
        .expect("save attempt");
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
        working_directory: None,
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

// ---- Task 3.2（REQ-ENV-09/D7）：coding reviewer 策略会话的 durable sink 接线 ----

#[tokio::test]
async fn coding_policy_review_input_carries_run_bound_durable_audit_sink() {
    let fixture = provider_start_persistence_fixture().await;
    let provider = CapturingProjectionProvider::new(review_plan_defect_output());

    let result = fixture
        .engine
        .execute_code_review(&fixture.attempt, &provider)
        .await;

    // 批2 Task 11（T20 前中间态）：CodeReviewer wire 面全开（共享工厂恒
    // None，纪律=ReviewerVerify+窗口合同，真窗归 T20）；engine 仍必须在
    // provider 启动前绑定 run-bound durable sink（执行审计不随去 deny 丢失）。
    let input = provider.input();
    assert!(
        input.tool_policy.is_none(),
        "review input must not carry deny policy (T20 intermediate)"
    );
    assert!(
        input.audit_sink.is_some(),
        "review run must carry the run-bound durable audit sink"
    );
    assert!(result.is_ok(), "review flow must not regress: {result:?}");
}

// ---- F3 Task 4.1（REQ-ENV-09/GC10）：审计通道严格分离回归 ----

/// 递归收集目录下全部文件路径。
fn collect_files_recursive(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Coder（非策略）run 的执行审计只落既有 role-run-events（execution_event_audit
/// 通道）；批2 Task 13 起 reviewer wire 面去 deny 后的 durable run 事件文件由
/// codex 合同驱动 provider_start 补齐（T20 剩余=ReviewerVerify 真窗/审批
/// durable 面）。两通道无交叉记录。
#[tokio::test]
async fn coding_coder_and_policy_runs_keep_audit_channels_strictly_separated() {
    use crate::cross_cutting::codex_provider::CodexProvider;
    use crate::cross_cutting::streaming_provider::ProviderVersionSupplier;

    let (root, store, attempt) = running_attempt_with_worktree();
    let aria_root = root.path().join(".aria");
    let tool_policy_partition = aria_root.join("tool-policy-run-audit");
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let event_drain = tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    // —— Coder（非策略）run：完整 provider 调用，execution 审计照常落盘 ——
    let coder_role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("audit_isolation_coder".to_string()),
        )
        .expect("coder role run");
    let provider = CompletedInvocationProvider;
    let (legacy_input, input) = provider_invocation_inputs(&attempt);
    let provider_name = ProviderName::Codex;
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    let outcome = engine
        .run_provider_stream_invocation(CodingProviderStreamRun {
            attempt: &attempt,
            node_id: "audit_isolation_coder",
            role_run: Some(&coder_role_run),
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
    assert!(
        matches!(outcome, ProviderInvocationOutcome::Completed(_)),
        "coder run must complete: {outcome:?}"
    );
    assert_role_run_raw_output(
        &store,
        &attempt,
        &coder_role_run,
        "completed invocation evidence",
    );
    // Coder run 不得产生任何 tool-policy durable 记录（分区目录不存在）。
    assert!(
        !tool_policy_partition.exists(),
        "non-policy coder runs must never create the tool-policy-run-audit partition"
    );

    // —— 策略 reviewer run：真实 CodexProvider（wire fixture）走 engine 接线 ——
    let reviewer_role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some("audit_isolation_policy_reviewer".to_string()),
        )
        .expect("reviewer role run");
    // 批2 Task 11（T20 前中间态）：reviewer run 不再是 deny 策略会话——
    // 改用无三联动前置的 completed-only wire fixture（thread/start 无策略
    // 前置）；审计通道分离断言不变（sink 由 engine 按 Reviewer 角色分配）。
    let fixture = {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/provider/codex_app_server_completed_only_fixture.sh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&path)
                .unwrap_or_else(|error| panic!("fixture metadata {}: {error}", path.display()))
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions)
                .unwrap_or_else(|error| panic!("chmod fixture: {error}"));
        }
        path
    };
    let supplier: ProviderVersionSupplier =
        std::sync::Arc::new(|| Ok("codex 0.124.0-isolation-fixture".to_string()));
    let policy_provider = CodexProvider::new(fixture).with_version_supplier(supplier);
    let worktree = attempt.worktree_path.clone().expect("worktree path");
    // 批2 Task 11（T20 前中间态）：reviewer wire 面全开（无 deny）；审计
    // 通道分离语义不变——sink 由 engine 按 Reviewer 角色分配。
    let policy_input = StreamingProviderInput {
        working_directory: None,
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
        provider_type: ProviderType::Codex,
        role: AdapterRole::Reviewer,
        prompt: "audit isolation policy prompt".to_string(),
        working_dir: worktree,
        workspace_session_id: Some("ws-coding-audit-isolation".to_string()),
        resume_provider_session_id: None,
        permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: 60,
    };
    let policy_legacy_input = AdapterInput {
        working_directory: None,
        provider_type: ProviderType::Codex,
        role: AdapterRole::Reviewer,
        worktree_path: policy_input
            .working_dir
            .clone()
            .to_str()
            .map(str::to_string),
        provider_stream_log_dir: None,
        prompt: policy_input.prompt.clone(),
        context_files: Vec::new(),
        output_schema: "coding_workspace_markdown".to_string(),
        timeout: 60,
        max_retries: 0,
    };
    let (_policy_command_tx, mut policy_command_rx) = mpsc::channel(1);
    let policy_outcome = engine
        .run_provider_stream_invocation(CodingProviderStreamRun {
            attempt: &attempt,
            node_id: "audit_isolation_policy_reviewer",
            role_run: Some(&reviewer_role_run),
            provider: &policy_provider,
            legacy_input: &policy_legacy_input,
            input: policy_input,
            provider_name: &provider_name,
            provider_role: CodingProviderRole::CodeReviewer,
            command_rx: &mut policy_command_rx,
            allow_legacy_stream_fallback: false,
            timeout: None,
            timeout_reason_code: None,
            suppress_failure_side_effects: false,
            validated_input: None,
        })
        .await;
    assert!(
        matches!(policy_outcome, ProviderInvocationOutcome::Completed(ref outcome)
            if outcome.full_output == "Codex completed-only chunk"),
        "reviewer run must complete via the wire fixture: {policy_outcome:?}"
    );

    // 策略通道（T13 合同驱动）：reviewer run 携带 engine 分配的 run-bound
    // sink → adapter 落 provider_start 首行（无策略 digest=空串，
    // sandbox/approval 与实际启动参数同源）。
    // （`role-run-seq.jsonl` 是 seq 分配高水位 marker，不是事件文件，排除。）
    let partition_dir = tool_policy_partition.join("ws-coding-audit-isolation");
    let mut partition_files = Vec::new();
    collect_files_recursive(&partition_dir, &mut partition_files);
    let run_files: Vec<_> = partition_files
        .iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name != "role-run-seq.jsonl")
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    // 批2 Task 13（合同驱动 durable 分区，B2T11 裁定缺口补齐）：无策略
    // reviewer run 的 durable provider_start 首行在位；ReviewerVerify 真窗
    // 接线与审批 durable 面归 T20。
    assert_eq!(
        run_files.len(),
        1,
        "contract-bound reviewer run must write exactly one durable run file: {run_files:?}"
    );
    let run_lines = std::fs::read_to_string(&run_files[0]).unwrap();
    let first_line = run_lines.lines().next().unwrap();
    let first: serde_json::Value = serde_json::from_str(first_line).unwrap();
    assert_eq!(
        first["event_type"], "provider_start",
        "reviewer run file must start with provider_start: {first_line}"
    );
    assert_eq!(first["role"], "reviewer");
    assert_eq!(first["sandbox"], "danger-full-access");
    assert_eq!(first["approval_policy"], "never");
    assert_eq!(
        first["tool_policy_canonical_digest"], "",
        "non-policy contract run carries no tool-policy canonical digest"
    );
    assert!(
        partition_dir.join("role-run-seq.jsonl").exists(),
        "engine must still allocate the reviewer tool-policy role run seq (audit kept post-deny)"
    );

    // 执行通道：role-run-events（execution_event_audit）不含任何 tool-policy
    // durable canonical 事件（策略事件不串入执行审计）。注意执行通道有既有的
    // 生命周期事件 `"event_type":"provider_start"`（CodingRoleRunEventType），与
    // durable 分区同名不同载体；用 tool-policy DTO 独有键与专属事件类型作标记。
    let events_root =
        store.role_run_events_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
    let mut execution_files = Vec::new();
    collect_files_recursive(&events_root, &mut execution_files);
    assert!(
        !execution_files.is_empty(),
        "coder run must keep producing execution_event_audit records"
    );
    for file in &execution_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            for marker in [
                "\"tool_policy_canonical_digest\"",
                "\"event_type\":\"approval_decision\"",
                "\"event_type\":\"protocol_warning\"",
                "\"event_type\":\"session_terminated\"",
            ] {
                assert!(
                    !content.contains(marker),
                    "execution audit channel must not carry tool-policy events: {} contains {marker}",
                    file.display()
                );
            }
        }
    }
    drop(engine);
    event_drain.await.expect("event drain");
}

// ---- N4（REQ-CS-04/REQ-ENV-10/11）：Logical envelope 接线回归 ----

/// N4：逻辑 launch envelope 的 root cwd/写根边界——Coder 角色 cwd 冻结为
/// authority_root（readable 随 root 化），唯一 writable root 是所选 target 派生
/// worktree（root 不可写），PolicyTarget 匹配冻结 logical/checkout 身份；reviewer
/// 角色可写根恒空且 root 仍不可写。真实 provider 进入由 controller 统一终验。
#[test]
fn plan_coding_n4_logical_launch_keeps_root_cwd_and_only_target_writable() {
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::coding_models::AttemptTargetSnapshot;
    use crate::product::coding_models::provider_config::CodingRoleProviderConfigSnapshot;
    use crate::product::logical_codebase::policy::SessionPolicyAction;
    use crate::product::logical_codebase::{
        GatewayRunAudit, LogicalRepositoryId, RepositoryCheckoutId,
    };

    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");

    // 冻结 target snapshot：logical/checkout 身份 + canonical path（成员 worktree
    // 位于聚合根之下，与 build_gateway_with_registry 的 cwd authority 契约一致）。
    let mut logical = attempt.clone();
    logical.target_snapshot = Some(AttemptTargetSnapshot {
        // 两个互不相同的非零 UUID：使下方 PolicyTarget 身份断言具串位判别力
        // （nil 值下实现若将 checkout_id 串位填入 logical 槽断言仍会通过）。
        logical_repository_id: LogicalRepositoryId(uuid::Uuid::new_v4()),
        checkout_id: RepositoryCheckoutId(uuid::Uuid::new_v4()),
        physical_repository_id: "repository_0001".to_string(),
        canonical_path: worktree.clone(),
        git_dir_identity: "git-dir-identity".to_string(),
        revision: None,
        policy_digest: String::new(),
        membership_revision: 1,
        captured_at: "2026-10-13T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    });
    let attempt_path = store
        .paths()
        .issue_lifecycle_root(&logical.project_id, &logical.issue_id)
        .join("coding-attempts")
        .join(format!("{}.json", logical.id));
    crate::product::json_store::write_json(&attempt_path, &logical)
        .expect("persist logical attempt");

    store
        .update_role_provider_config_snapshot(
            &logical.project_id,
            &logical.issue_id,
            &logical.id,
            CodingRoleProviderConfigSnapshot {
                coder: crate::product::models::ProviderName::ClaudeCode,
                code_reviewer: Some(crate::product::models::ProviderName::ClaudeCode),
                internal_reviewer: Some(crate::product::models::ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
        )
        .expect("seed role provider config");

    let gateway = super::provider_gateway_validated_input::build_gateway_with_registry(
        &store.paths(),
        &logical.project_id,
        Arc::new(ProviderRegistry::new()),
        Arc::new(GatewayRunAudit::new()),
    );
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx)
        .with_logical_provider_gateway(Arc::new(gateway));

    // Coder：root cwd + 唯一 target 写根。
    let policy = engine
        .resolve_launch_policy_for_role(&logical, CodingProviderRole::Coder, &worktree)
        .expect("coder launch policy resolves")
        .expect("logical attempt + gateway must produce policy");
    let envelope = policy.envelope();
    assert_eq!(envelope.action, SessionPolicyAction::CodingTargetWrite);
    assert_eq!(
        envelope.working_directory, envelope.authority_root,
        "session cwd stays the LC authority root (REQ-ENV-10)"
    );
    assert_eq!(
        envelope.readable_roots,
        vec![envelope.authority_root.clone()],
        "readable roots follow the root cwd"
    );
    assert_eq!(
        envelope.writable_roots,
        vec![worktree.clone()],
        "Coder has exactly one writable root: the derived target worktree"
    );
    assert_ne!(envelope.authority_root, worktree);
    assert!(
        !envelope.writable_roots.contains(&envelope.authority_root),
        "root cwd must not become writable (REQ-ENV-11)"
    );
    let snapshot = logical.target_snapshot.as_ref().expect("snapshot");
    assert_eq!(
        envelope.target.worktree, worktree,
        "PolicyTarget worktree is the derived target worktree"
    );
    assert_eq!(
        envelope.target.logical_repository_id,
        snapshot.logical_repository_id.0.to_string(),
        "PolicyTarget keeps the frozen logical identity"
    );
    assert_eq!(
        envelope.target.checkout_id,
        snapshot.checkout_id.0.to_string(),
        "PolicyTarget keeps the frozen checkout identity"
    );

    // reviewer：可写根恒空，cwd 仍为 root 且 root 不可写。
    for role in [
        CodingProviderRole::CodeReviewer,
        CodingProviderRole::InternalReviewer,
    ] {
        let policy = engine
            .resolve_launch_policy_for_role(&logical, role, &worktree)
            .expect("reviewer launch policy resolves")
            .expect("reviewer policy present");
        let envelope = policy.envelope();
        assert_eq!(envelope.action, SessionPolicyAction::ReviewReadOnly);
        assert_eq!(
            envelope.working_directory, envelope.authority_root,
            "reviewer session cwd stays the authority root"
        );
        assert!(envelope.writable_roots.is_empty());
        assert!(
            !envelope.writable_roots.contains(&envelope.authority_root),
            "root cwd stays non-writable for reviewer roles"
        );
    }

    drop(root);
}
