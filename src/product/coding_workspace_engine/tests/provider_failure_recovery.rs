use super::*;
use crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const WORK_ITEM_ID: &str = "work_item_0001";
const NODE_ID: &str = "coding_node_0009";

struct TransportFailuresThenSuccessProvider {
    failures_before_success: usize,
    starts: AtomicUsize,
    inputs: Mutex<Vec<StreamingProviderInput>>,
    output: String,
    first_failure_worktree_update: Option<(PathBuf, String)>,
}

#[derive(Default)]
struct PermissionTimeoutProvider {
    starts: AtomicUsize,
}

struct CancelledProvider {
    starts: AtomicUsize,
    started: Arc<Notify>,
}

impl CancelledProvider {
    fn new() -> Self {
        Self {
            starts: AtomicUsize::new(0),
            started: Arc::new(Notify::new()),
        }
    }
}

enum RetryBoundaryMutation {
    Abort {
        store: CodingAttemptStore,
        attempt: CodingExecutionAttempt,
    },
    PlanRepair {
        store: CodingAttemptStore,
        attempt: CodingExecutionAttempt,
    },
    OwnerChange {
        lifecycle: LifecycleStore,
        attempt: CodingExecutionAttempt,
    },
    RoleRunCreateFailure {
        store: CodingAttemptStore,
        attempt: CodingExecutionAttempt,
    },
}

struct RetryBoundaryMutationProvider {
    starts: AtomicUsize,
    mutation: Mutex<Option<RetryBoundaryMutation>>,
}

impl RetryBoundaryMutationProvider {
    fn new(mutation: RetryBoundaryMutation) -> Self {
        Self {
            starts: AtomicUsize::new(0),
            mutation: Mutex::new(Some(mutation)),
        }
    }
}

impl TransportFailuresThenSuccessProvider {
    fn new(failures_before_success: usize, output: impl Into<String>) -> Self {
        Self {
            failures_before_success,
            starts: AtomicUsize::new(0),
            inputs: Mutex::new(Vec::new()),
            output: output.into(),
            first_failure_worktree_update: None,
        }
    }

    fn with_first_failure_worktree_update(
        mut self,
        path: PathBuf,
        content: impl Into<String>,
    ) -> Self {
        self.first_failure_worktree_update = Some((path, content.into()));
        self
    }

    fn recorded_inputs(&self) -> Vec<StreamingProviderInput> {
        self.inputs.lock().expect("provider inputs").clone()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for TransportFailuresThenSuccessProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start_no = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        let structured_output_contract = input.structured_output_contract.clone();
        self.inputs.lock().expect("provider inputs").push(input);
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel(2);
        let failures_before_success = self.failures_before_success;
        let output = self.output.clone();
        if start_no == 1
            && let Some((path, content)) = self.first_failure_worktree_update.as_ref()
        {
            fs::write(path, content).expect("update reviewer diff after first failure");
        }
        tokio::spawn(async move {
            let event = if start_no <= failures_before_success {
                ProviderEvent::Failed {
                    message: "connection reset by peer".to_string(),
                }
            } else {
                ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::from_output(
                        output,
                        structured_output_contract.as_ref(),
                        Some(format!("provider-session-{start_no}")),
                    ),
                )
            };
            let _ = event_tx.send(event).await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RetryBoundaryMutationProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let mutation = self.mutation.lock().expect("retry mutation").take();
        if let Some(mutation) = mutation {
            match mutation {
                RetryBoundaryMutation::Abort { store, attempt } => {
                    let (tx, _rx) = mpsc::channel(4);
                    CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx)
                        .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
                        .await
                        .expect("abort during provider run");
                }
                RetryBoundaryMutation::PlanRepair { store, attempt } => {
                    store
                        .update_attempt_status(
                            &attempt.project_id,
                            &attempt.issue_id,
                            &attempt.id,
                            CodingAttemptStatus::AwaitingPlanAmendment,
                        )
                        .expect("enter plan repair during provider run");
                }
                RetryBoundaryMutation::OwnerChange { lifecycle, attempt } => {
                    lifecycle
                        .release_issue_worktree_lock(
                            &attempt.project_id,
                            &attempt.issue_id,
                            &attempt.work_item_id,
                            &attempt.id,
                        )
                        .expect("release original owner");
                    lifecycle
                        .try_acquire_issue_worktree_lock(
                            &attempt.project_id,
                            &attempt.issue_id,
                            &attempt.work_item_id,
                            "coding_attempt_other",
                        )
                        .expect("acquire changed owner");
                }
                RetryBoundaryMutation::RoleRunCreateFailure { store, attempt } => {
                    let prior = store
                        .latest_role_run(
                            &attempt.project_id,
                            &attempt.issue_id,
                            &attempt.id,
                            CodingExecutionStage::Coding,
                            CodingProviderRole::Coder,
                        )
                        .expect("initial coder role run")
                        .expect("initial coder role run exists");
                    let mut conflicting_retry = prior.clone();
                    conflicting_retry.id = "coding_role_run_9999".to_string();
                    conflicting_retry.status = CodingRoleRunStatus::Failed;
                    conflicting_retry.retry_metadata =
                        Some(crate::product::coding_models::CodingRoleRunRetryMetadata {
                            cycle_id: prior.id.clone(),
                            attempt_no: 2,
                            prior_run_id: Some(prior.id.clone()),
                        });
                    conflicting_retry.node_id = None;
                    conflicting_retry.reason_code =
                        Some("test_injected_retry_conflict".to_string());
                    conflicting_retry.completed_at = Some("2026-08-07T00:00:00Z".to_string());
                    store
                        .save_role_run(&attempt.project_id, &attempt.issue_id, &conflicting_retry)
                        .expect("inject automatic retry role-run create conflict");
                }
            }
        }
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
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
impl StreamingProviderAdapter for PermissionTimeoutProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::PermissionTimeout {
                    permission_id: "permission_1".to_string(),
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
impl StreamingProviderAdapter for CancelledProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _keep_stream_open = event_tx;
            std::future::pending::<()>().await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

fn configure_dirty_shared_worktree(
    _root: &tempfile::TempDir,
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    let worktree = attempt.worktree_path.as_ref().expect("attempt worktree");
    init_test_git_repo(worktree);
    fs::write(worktree.join("dirty.rs"), "pub fn dirty() {}\n").expect("dirty shared worktree");
    let lifecycle = LifecycleStore::new(store.paths());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            branch_name: attempt.branch_name.clone(),
            worktree_path: worktree.clone(),
            base_branch: attempt.base_branch.clone(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.work_item_id,
            &attempt.id,
        )
        .expect("shared worktree lock");
}

fn assert_cancelled_provider_run_has_no_gate(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role: CodingProviderRole,
) {
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("aborted attempt")
            .status,
        CodingAttemptStatus::Aborted
    );
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs");
    let run = runs
        .iter()
        .find(|run| run.role == role)
        .expect("cancelled provider role run");
    assert_eq!(run.status, CodingRoleRunStatus::Aborted);
    assert_eq!(run.reason_code.as_deref(), Some("abort_attempt"));
    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Failed);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
}

#[tokio::test]
async fn coder_permission_timeout_keeps_existing_failure_gate_without_retrying() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = PermissionTimeoutProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect_err("permission timeout stays on the existing coder failure path");

    assert!(
        error
            .to_string()
            .contains("Permission request permission_1 timed out")
    );
    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("coder_provider_interrupted")
    );
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder failure gate");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("coder_provider_interrupted")
    );
    let node = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder timeline")
        .into_iter()
        .find(|node| node.stage == CodingExecutionStage::Coding)
        .expect("coder node");
    assert_eq!(node.status, CodingTimelineNodeStatus::Failed);
}

#[tokio::test]
async fn reviewer_permission_timeout_keeps_existing_failure_gate_without_retrying() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = PermissionTimeoutProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("permission timeout stays on the existing reviewer failure path");

    assert!(
        error
            .to_string()
            .contains("Permission request permission_1 timed out")
    );
    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reviewer role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("code_review_provider_interrupted")
    );
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reviewer failure gate");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("code_review_provider_interrupted")
    );
    let node = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reviewer timeline")
        .into_iter()
        .find(|node| node.stage == CodingExecutionStage::CodeReview)
        .expect("reviewer node");
    assert_eq!(node.status, CodingTimelineNodeStatus::Failed);
}

#[tokio::test]
async fn cancelled_coder_run_finalizes_current_run_and_node_without_retry_or_gate() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let provider = CancelledProvider::new();
    let (tx, _rx) = mpsc::channel(16);
    let cancellation = CancellationToken::new();
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx)
        .with_cancellation(cancellation.clone());
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    let context = CodingExecutionContext::default();

    let execute =
        engine.execute_coding_with_commands(&attempt, &provider, &context, &mut command_rx);
    let cancel = async {
        tokio::time::timeout(Duration::from_millis(250), provider.started.notified())
            .await
            .expect("coder provider did not start");
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(execute, cancel);
    let error = result.expect_err("cancelled coder invocation stops without an automatic retry");

    assert!(
        matches!(error, CodingWorkspaceEngineError::Aborted),
        "unexpected cancellation error: {error:?}"
    );
    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("aborted attempt")
            .status,
        CodingAttemptStatus::Aborted
    );
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Aborted);
    assert_eq!(runs[0].reason_code.as_deref(), Some("abort_attempt"));
    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Failed);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
}

#[tokio::test]
async fn cancelled_reviewer_run_finalizes_current_run_and_node_without_retry_or_gate() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    let provider = CancelledProvider::new();
    let (tx, _rx) = mpsc::channel(16);
    let cancellation = CancellationToken::new();
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx)
        .with_cancellation(cancellation.clone());
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let execute = engine.execute_code_review_with_commands(&attempt, &provider, &mut command_rx);
    let cancel = async {
        tokio::time::timeout(Duration::from_millis(250), provider.started.notified())
            .await
            .expect("reviewer provider did not start");
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(execute, cancel);
    let error = result.expect_err("cancelled reviewer invocation stops without an automatic retry");

    assert!(matches!(error, CodingWorkspaceEngineError::Aborted));
    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("aborted attempt")
            .status,
        CodingAttemptStatus::Aborted
    );
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reviewer runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Aborted);
    assert_eq!(runs[0].reason_code.as_deref(), Some("abort_attempt"));
    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reviewer nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Failed);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
}

#[tokio::test]
async fn cancelled_coder_run_with_dirty_shared_worktree_finalizes_without_manual_gate() {
    let (root, store, attempt) = running_attempt_with_worktree();
    configure_dirty_shared_worktree(&root, &store, &attempt);
    let provider = CancelledProvider::new();
    let (tx, _rx) = mpsc::channel(16);
    let cancellation = CancellationToken::new();
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx)
        .with_cancellation(cancellation.clone());
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    let context = CodingExecutionContext::default();

    let execute =
        engine.execute_coding_with_commands(&attempt, &provider, &context, &mut command_rx);
    let cancel = async {
        tokio::time::timeout(Duration::from_millis(250), provider.started.notified())
            .await
            .expect("coder provider did not start");
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(execute, cancel);

    assert!(matches!(
        result.expect_err("cancelled coder must not open a dirty-worktree gate"),
        CodingWorkspaceEngineError::Aborted
    ));
    assert_cancelled_provider_run_has_no_gate(&store, &attempt, CodingProviderRole::Coder);
}

#[tokio::test]
async fn cancelled_reviewer_run_with_dirty_shared_worktree_finalizes_without_manual_gate() {
    let (root, store, attempt) = running_attempt_with_worktree();
    configure_dirty_shared_worktree(&root, &store, &attempt);
    let worktree = attempt.worktree_path.clone().expect("worktree");
    let provider = CancelledProvider::new();
    let (tx, _rx) = mpsc::channel(16);
    let cancellation = CancellationToken::new();
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx)
        .with_cancellation(cancellation.clone());
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let execute = engine.execute_code_review_with_commands(&attempt, &provider, &mut command_rx);
    let cancel = async {
        tokio::time::timeout(Duration::from_millis(250), provider.started.notified())
            .await
            .expect("reviewer provider did not start");
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(execute, cancel);

    assert!(matches!(
        result.expect_err("cancelled reviewer must not open a dirty-worktree gate"),
        CodingWorkspaceEngineError::Aborted
    ));
    assert_eq!(
        worktree,
        *attempt
            .worktree_path
            .as_ref()
            .expect("worktree remains configured")
    );
    assert_cancelled_provider_run_has_no_gate(&store, &attempt, CodingProviderRole::CodeReviewer);
}

#[tokio::test]
async fn automatic_retry_role_run_create_failure_leaves_no_orphan_timeline_node() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let provider =
        RetryBoundaryMutationProvider::new(RetryBoundaryMutation::RoleRunCreateFailure {
            store: store.clone(),
            attempt: attempt.clone(),
        });
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect_err("injected retry role-run creation conflict stops the retry cycle");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::Store(ProductStoreError::Conflict {
            kind: "coding_role_run_retry_metadata",
            ..
        })
    ));
    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes");
    assert_eq!(
        nodes.len(),
        1,
        "failed retry creation must not leave an orphan node"
    );
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Failed);
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs");
    let initial = runs
        .iter()
        .find(|run| run.id == "coding_role_run_0001")
        .expect("initial coder run");
    assert_eq!(initial.status, CodingRoleRunStatus::Failed);
    assert_eq!(
        initial.reason_code.as_deref(),
        Some("provider_connection_interrupted")
    );
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
}

#[tokio::test]
async fn reviewer_initial_invocation_preserves_persisted_provider_session() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::CodeReviewer,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "reviewer-session-before-retry".to_string(),
                updated_at: "2026-08-07T00:00:00Z".to_string(),
                last_node_id: Some("coding_node_0001".to_string()),
            }],
        )
        .expect("seed reviewer conversation");
    let provider = TransportFailuresThenSuccessProvider::new(1, "not structured review output");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let _ = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await;

    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 2);
    assert_eq!(
        inputs[0].resume_provider_session_id.as_deref(),
        Some("reviewer-session-before-retry")
    );
    assert_eq!(inputs[1].resume_provider_session_id, None);
}

#[tokio::test]
async fn abort_during_transport_failure_creates_no_automatic_retry_records() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let provider = RetryBoundaryMutationProvider::new(RetryBoundaryMutation::Abort {
        store: store.clone(),
        attempt: attempt.clone(),
    });
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let _ = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect_err("abort stops the retry cycle");

    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("aborted attempt")
            .status,
        CodingAttemptStatus::Aborted
    );
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("provider_retry_attempt_state_changed")
    );
    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Failed);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
}

#[tokio::test]
async fn plan_repair_during_transport_failure_creates_no_automatic_retry_records() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let provider = RetryBoundaryMutationProvider::new(RetryBoundaryMutation::PlanRepair {
        store: store.clone(),
        attempt: attempt.clone(),
    });
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let _ = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect_err("plan repair stops the retry cycle");

    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("plan repair attempt")
            .status,
        CodingAttemptStatus::AwaitingPlanAmendment
    );
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("provider_retry_attempt_state_changed")
    );
    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Failed);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
}

#[tokio::test]
async fn plan_repair_after_retry_preflight_creates_no_retry_records_or_provider_start() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (_pause, reached, resume) = register_coding_mutation_test_pause(
        store.paths().root(),
        CodingMutationTestPoint::ProviderFailure,
    );
    let provider = TransportFailuresThenSuccessProvider::new(usize::MAX, "unused");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    let context = CodingExecutionContext::default();

    let execute =
        engine.execute_coding_with_commands(&attempt, &provider, &context, &mut command_rx);
    let enter_plan_repair = async {
        tokio::time::timeout(Duration::from_millis(250), reached)
            .await
            .expect("retry preflight did not reach the controlled pause")
            .expect("retry preflight pause sender dropped");
        store
            .update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::AwaitingPlanAmendment,
            )
            .expect("enter plan repair after retry preflight");
        resume.send(()).expect("resume retry preflight");
    };
    let (result, ()) = tokio::join!(execute, enter_plan_repair);
    let error = result.expect_err("Plan Repair rejects retry creation after the first preflight");

    assert!(
        error
            .to_string()
            .contains("provider_retry_attempt_state_changed")
    );
    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("Plan Repair attempt")
            .status,
        CodingAttemptStatus::AwaitingPlanAmendment
    );
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("provider_connection_interrupted")
    );
    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Failed);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
}

#[tokio::test]
async fn owner_change_during_transport_failure_creates_no_automatic_retry_records() {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("shared-worktree");
    fs::create_dir_all(&worktree).expect("shared worktree");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: worktree.clone(),
            base_branch: "HEAD".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            WORK_ITEM_ID,
            "issue_worktree_lease_retry_boundary",
        )
        .expect("initial owner");
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .expect("attempt");
    lifecycle
        .bind_issue_worktree_lock_to_attempt(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID, &attempt.id)
        .expect("bind owner");
    let attempt = store
        .update_attempt_status(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    let provider = RetryBoundaryMutationProvider::new(RetryBoundaryMutation::OwnerChange {
        lifecycle,
        attempt: attempt.clone(),
    });
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect_err("owner change stops the retry cycle");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::Store(ProductStoreError::Conflict {
            kind: "issue_worktree_lock_owner",
            ..
        })
    ));
    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("provider_retry_attempt_state_changed")
    );
    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Failed);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
}

#[tokio::test]
async fn coder_retries_once_then_succeeds_with_two_auditable_runs() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = TransportFailuresThenSuccessProvider::new(1, "coder retry completed");
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let completed = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext {
                work_item_markdown: Some("# Retry work item\n\nKeep the full context.".to_string()),
                verification_commands: vec!["cargo test --locked --lib retry".to_string()],
            },
            &mut command_rx,
        )
        .await
        .expect("second fresh coder invocation succeeds");

    assert_eq!(completed.status, CodingAttemptStatus::Running);
    assert_eq!(completed.rework_count, 0);
    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].working_dir, worktree);
    assert_eq!(inputs[1].working_dir, inputs[0].working_dir);
    assert_eq!(inputs[1].resume_provider_session_id, None);
    assert_eq!(inputs[1].prompt, inputs[0].prompt);
    assert!(inputs[1].prompt.contains("Retry work item"));

    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder role runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_ne!(runs[0].status, CodingRoleRunStatus::Superseded);
    assert_eq!(runs[0].trigger, CodingRoleRunTrigger::Initial);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("provider_connection_interrupted")
    );
    assert_eq!(runs[0].raw_provider_output_refs.len(), 1);
    assert_eq!(runs[1].status, CodingRoleRunStatus::Completed);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::AutomaticRetry);
    let first_retry = runs[0].retry_metadata.as_ref().expect("first metadata");
    let second_retry = runs[1].retry_metadata.as_ref().expect("second metadata");
    assert_eq!(first_retry.attempt_no, 1);
    assert_eq!(second_retry.attempt_no, 2);
    assert_eq!(second_retry.cycle_id, first_retry.cycle_id);
    assert_eq!(
        second_retry.prior_run_id.as_deref(),
        Some(runs[0].id.as_str())
    );

    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder timeline nodes");
    let coder_nodes = nodes
        .iter()
        .filter(|node| node.stage == CodingExecutionStage::Coding)
        .collect::<Vec<_>>();
    assert_eq!(coder_nodes.len(), 2);
    assert_eq!(coder_nodes[0].status, CodingTimelineNodeStatus::Failed);
    assert_eq!(coder_nodes[1].status, CodingTimelineNodeStatus::Completed);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
}

#[tokio::test]
async fn reviewer_three_transport_failures_open_one_human_gate_after_third_run() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = TransportFailuresThenSuccessProvider::new(usize::MAX, "unused")
        .with_first_failure_worktree_update(
            worktree.join("reviewed.rs"),
            "pub fn reviewed_after_retry() {}\n",
        );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("the third transport failure opens the existing reviewer gate");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::ProviderStream(ref message)
            if message == "connection reset by peer"
    ));
    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 3);
    assert!(inputs.iter().all(|input| input.working_dir == worktree));
    assert!(
        inputs
            .iter()
            .all(|input| input.resume_provider_session_id.is_none())
    );
    assert!(
        inputs
            .iter()
            .all(|input| input.prompt.contains("当前阶段：只读代码审查"))
    );
    assert!(
        inputs
            .iter()
            .all(|input| input.prompt.contains("reviewed.rs")
                && input.prompt.contains("pub fn reviewed"))
    );
    assert!(!inputs[0].prompt.contains("reviewed_after_retry"));
    assert!(inputs[1].prompt.contains("reviewed_after_retry"));
    assert!(inputs[2].prompt.contains("reviewed_after_retry"));
    let nonces = inputs
        .iter()
        .map(|input| {
            input
                .structured_output_contract
                .as_ref()
                .expect("review contract")
                .nonce
                .clone()
        })
        .collect::<Vec<_>>();
    assert_ne!(nonces[0], nonces[1]);
    assert_ne!(nonces[1], nonces[2]);

    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reviewer role runs");
    assert_eq!(runs.len(), 3);
    assert_eq!(
        runs.iter()
            .map(|run| run.status.clone())
            .collect::<Vec<_>>(),
        vec![
            CodingRoleRunStatus::Failed,
            CodingRoleRunStatus::Failed,
            CodingRoleRunStatus::Failed,
        ]
    );
    assert_eq!(runs[0].trigger, CodingRoleRunTrigger::Initial);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::AutomaticRetry);
    assert_eq!(runs[2].trigger, CodingRoleRunTrigger::AutomaticRetry);
    assert_eq!(runs[0].retry_metadata.as_ref().unwrap().attempt_no, 1);
    assert_eq!(runs[1].retry_metadata.as_ref().unwrap().attempt_no, 2);
    assert_eq!(runs[2].retry_metadata.as_ref().unwrap().attempt_no, 3);
    assert!(runs.iter().all(|run| {
        run.reason_code.as_deref() == Some("provider_connection_interrupted")
            && run.raw_provider_output_refs.len() == 1
    }));

    let review_nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("review timeline nodes")
        .into_iter()
        .filter(|node| node.stage == CodingExecutionStage::CodeReview)
        .collect::<Vec<_>>();
    assert_eq!(review_nodes.len(), 3);
    assert!(
        review_nodes
            .iter()
            .all(|node| node.status == CodingTimelineNodeStatus::Failed)
    );
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reviewer gate");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("code_review_provider_interrupted")
    );
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("blocked attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
}

#[test]
fn provider_retry_classifier_retries_only_transport_failures() {
    let cases = [
        (
            CodingWorkspaceEngineError::ProviderStream("provider start I/O error".to_string()),
            true,
        ),
        (
            CodingWorkspaceEngineError::ProviderAdapter(ProviderAdapterError::execution_failed(
                None,
                "",
                "Text file busy",
                0,
            )),
            true,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream(
                "provider stream ended before completion".to_string(),
            ),
            true,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("connection reset by peer".to_string()),
            true,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("provider execution timeout".to_string()),
            true,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("upstream returned HTTP 503".to_string()),
            true,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("upstream returned HTTP 504".to_string()),
            true,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("gateway returned status 504".to_string()),
            true,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("HTTP/1.1 503".to_string()),
            true,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream(
                "HTTP status server error (503 Service Unavailable)".to_string(),
            ),
            true,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("HTTP error code 503".to_string()),
            true,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream(
                "request id abc: upstream returned HTTP 503".to_string(),
            ),
            true,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("request id 1503 failed".to_string()),
            false,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("request id 503 failed".to_string()),
            false,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("port 504 unavailable".to_string()),
            false,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("retry count 503".to_string()),
            false,
        ),
        (CodingWorkspaceEngineError::Aborted, false),
        (
            CodingWorkspaceEngineError::ProviderProtocol("invalid event".to_string()),
            false,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream(
                "structured output parser error".to_string(),
            ),
            false,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("permission timeout".to_string()),
            false,
        ),
        (
            CodingWorkspaceEngineError::ProviderStream("choice timeout".to_string()),
            false,
        ),
    ];

    for (error, retryable) in cases {
        assert_eq!(
            classify_provider_failure(&error).is_retryable(),
            retryable,
            "{error}"
        );
    }
}

#[test]
fn provider_retry_classifier_fails_closed_for_unrecognized_adapter_execution_failures() {
    for stderr in [
        "request id 503 failed",
        "port 504 unavailable",
        "retry count 503",
    ] {
        let error = CodingWorkspaceEngineError::ProviderAdapter(
            ProviderAdapterError::execution_failed(None, "", stderr, 1),
        );
        assert!(
            !classify_provider_failure(&error).is_retryable(),
            "{stderr}"
        );
    }
}

#[test]
fn provider_invocation_outcome_keeps_typed_failure_and_interaction_wait() {
    let retry = ProviderInvocationOutcome::from_result(
        Err(CodingWorkspaceEngineError::ProviderAdapter(
            ProviderAdapterError::execution_failed(None, "", "Text file busy", 0),
        )),
        "partial".to_string(),
    );
    let ProviderInvocationOutcome::RetryableTransport {
        failure,
        reason_code,
        message,
        partial_output,
    } = retry
    else {
        panic!("real provider start I/O must remain a typed retryable outcome");
    };
    assert_eq!(failure, RetryableProviderFailure::StartIo);
    assert_eq!(reason_code, "provider_start_io");
    assert!(message.contains("ProviderExecutionFailed"));
    assert!(message.contains("Text file busy"));
    assert_eq!(partial_output, "partial");

    for timeout in ["permission timeout", "choice timeout"] {
        let waiting = ProviderInvocationOutcome::from_result(
            Err(CodingWorkspaceEngineError::ProviderStream(
                timeout.to_string(),
            )),
            String::new(),
        );
        assert!(matches!(
            waiting,
            ProviderInvocationOutcome::NonRetryable {
                interaction_wait: true,
                ..
            }
        ));
    }
}

#[tokio::test]
async fn code_review_provider_failure_blocks_attempt_without_cleaning_shared_worktree() {
    let root = tempdir().expect("tempdir");
    let shared_worktree = root.path().join("shared-worktree");
    fs::create_dir_all(&shared_worktree).expect("shared worktree dir");
    init_test_git_repo(&shared_worktree);
    fs::write(shared_worktree.join("dirty.txt"), "uncommitted\n").expect("dirty worktree");

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: shared_worktree.clone(),
            base_branch: "HEAD".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            WORK_ITEM_ID,
            "issue_worktree_lease_provider_failure",
        )
        .expect("shared worktree lock");

    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(shared_worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    lifecycle
        .bind_issue_worktree_lock_to_attempt(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID, &attempt.id)
        .expect("bind shared worktree lock");
    let active_unit = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: WORK_ITEM_ID.to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("running active unit");
    let attempt = store
        .update_attempt_status(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    let attempt = store
        .update_attempt_stage(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    store
        .save_timeline_node(
            &attempt,
            CodingTimelineNode {
                id: NODE_ID.to_string(),
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                title: "代码审查".to_string(),
                status: CodingTimelineNodeStatus::Running,
                agent_role: Some(CodingAgentRole::Reviewer),
                summary: None,
                started_at: "2026-07-12T00:00:00Z".to_string(),
                completed_at: None,
                artifact_refs: Vec::new(),
            },
        )
        .expect("code review timeline node");
    store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some(NODE_ID.to_string()),
        )
        .expect("reviewer role run");

    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let message = "Permission request permission_1 timed out".to_string();

    let error = engine
        .fail_provider_stream::<()>(&attempt, NODE_ID, message.clone())
        .await
        .expect_err("provider timeout remains surfaced");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::ProviderStream(ref persisted_message)
            if persisted_message == &message
    ));
    let persisted = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    assert_eq!(persisted.stage, CodingExecutionStage::CodeReview);
    assert_eq!(persisted.completed_at, None);

    let timeline_node = store
        .get_timeline_nodes(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("timeline nodes")
        .into_iter()
        .find(|node| node.id == NODE_ID)
        .expect("review timeline node");
    assert_eq!(timeline_node.status, CodingTimelineNodeStatus::Failed);
    assert_eq!(timeline_node.summary.as_deref(), Some(message.as_str()));
    assert!(timeline_node.completed_at.is_some());

    let role_run = store
        .latest_role_run(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
        )
        .expect("latest reviewer role run")
        .expect("reviewer role run");
    assert_eq!(role_run.status, CodingRoleRunStatus::Failed);
    assert_eq!(
        role_run.reason_code.as_deref(),
        Some("code_review_provider_interrupted")
    );

    let open_gates = store
        .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("open blocked gates");
    let recovery_gate = open_gates
        .iter()
        .find(|gate| gate.reason_code.as_deref() == Some("code_review_provider_interrupted"))
        .expect("code review recovery gate");
    assert_eq!(recovery_gate.title, "代码审查中断");
    assert_eq!(
        recovery_gate
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_review"]
    );
    assert_eq!(recovery_gate.available_actions[0].label, "重试代码审查");
    assert!(
        open_gates.iter().all(|gate| {
            gate.reason_code.as_deref() != Some("shared_worktree_dirty_manual_gate")
        })
    );

    let persisted_unit = store
        .get_active_coding_unit(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("active coding unit")
        .expect("active coding unit remains");
    assert_eq!(persisted_unit.id, active_unit.id);
    assert_eq!(persisted_unit.status, CodingExecutionUnitStatus::Running);

    let shared = lifecycle
        .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
        .expect("load shared worktree")
        .expect("shared worktree exists");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some(WORK_ITEM_ID)
    );

    let attempt_before = persisted.clone();
    let units_before = store
        .list_coding_units(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("coding units before rejected send to coder");
    let gates_before = open_gates.clone();
    let role_runs_before = store
        .list_role_runs(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("role runs before rejected send to coder");
    let journal_before = store
        .get_failed_code_review_recovery_journal(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("recovery journal before rejected send to coder");
    let notes_before = store
        .list_context_notes(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("context notes before rejected send to coder");
    let instructions_before = store
        .list_rework_instructions(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("rework instructions before rejected send to coder");
    let recovery_gate_id = recovery_gate.gate_id.clone();

    let error = engine
        .handle_blocked_gate_response(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            &recovery_gate_id,
            "send_to_coder",
            Some("请 Coder 检查权限请求超时前未完成的改动并继续修复".to_string()),
        )
        .await
        .expect_err("provider interruption cannot bypass review recovery");
    assert!(matches!(
        error,
        CodingWorkspaceEngineError::ProviderStream(ref message)
            if message == "coding_failed_review_recovery_action_not_allowed"
    ));
    assert_eq!(
        store
            .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("attempt after rejected send to coder"),
        attempt_before
    );
    assert_eq!(
        store
            .list_coding_units(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("coding units after rejected send to coder"),
        units_before
    );
    assert_eq!(
        store
            .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("gates after rejected send to coder"),
        gates_before
    );
    assert_eq!(
        store
            .list_role_runs(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("role runs after rejected send to coder"),
        role_runs_before
    );
    assert_eq!(
        store
            .get_failed_code_review_recovery_journal(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("recovery journal after rejected send to coder"),
        journal_before
    );
    assert_eq!(
        store
            .list_context_notes(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("context notes after rejected send to coder"),
        notes_before
    );
    assert_eq!(
        store
            .list_rework_instructions(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("rework instructions after rejected send to coder"),
        instructions_before
    );
}

#[tokio::test]
async fn provider_failure_owner_conflict_is_zero_write_at_production_entry() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some(WORK_ITEM_ID.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            title: "provider failure owner preflight".to_string(),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("work item");
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: root.path().join("shared-worktree"),
            base_branch: "HEAD".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID, "coding_attempt_other")
        .expect("conflicting owner lock");

    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .expect("attempt");
    let attempt = store
        .update_attempt_status(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    store
        .save_timeline_node(
            &attempt,
            CodingTimelineNode {
                id: NODE_ID.to_string(),
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::PrepareContext,
                title: "准备上下文".to_string(),
                status: CodingTimelineNodeStatus::Running,
                agent_role: None,
                summary: None,
                started_at: "2026-07-21T00:00:00Z".to_string(),
                completed_at: None,
                artifact_refs: Vec::new(),
            },
        )
        .expect("timeline node");

    let attempt_before = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("attempt before");
    let timeline_before = store
        .get_timeline_nodes(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("timeline before");
    let work_items_before = lifecycle
        .list_work_items(PROJECT_ID, ISSUE_ID)
        .expect("work items before");
    let lease_before = lifecycle
        .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
        .expect("lease before")
        .expect("lease before");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .fail_provider_stream::<()>(&attempt, NODE_ID, "provider failed".to_string())
        .await
        .expect_err("owner conflict must reject provider failure before writes");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::Store(ProductStoreError::Conflict {
            kind: "issue_worktree_lock_owner",
            ..
        })
    ));
    assert_eq!(
        store
            .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("attempt after"),
        attempt_before
    );
    assert_eq!(
        store
            .get_timeline_nodes(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("timeline after"),
        timeline_before
    );
    assert_eq!(
        lifecycle
            .list_work_items(PROJECT_ID, ISSUE_ID)
            .expect("work items after"),
        work_items_before
    );
    assert_eq!(
        lifecycle
            .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
            .expect("lease after")
            .expect("lease after"),
        lease_before
    );
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn abort_during_provider_failure_prewrite_pause_is_stable() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .expect("attempt");
    let attempt = store
        .update_attempt_status(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    let attempt = store
        .update_attempt_stage(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    store
        .save_timeline_node(
            &attempt,
            CodingTimelineNode {
                id: NODE_ID.to_string(),
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                title: "代码审查".to_string(),
                status: CodingTimelineNodeStatus::Running,
                agent_role: Some(CodingAgentRole::Reviewer),
                summary: None,
                started_at: "2026-07-21T00:00:00Z".to_string(),
                completed_at: None,
                artifact_refs: Vec::new(),
            },
        )
        .expect("timeline node");
    store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some(NODE_ID.to_string()),
        )
        .expect("role run");
    let (_pause, reached, resume) = register_coding_mutation_test_pause(
        store.paths().root(),
        CodingMutationTestPoint::ProviderFailure,
    );
    let failure_store = store.clone();
    let failure_attempt = attempt.clone();
    let failure = tokio::spawn(async move {
        let (tx, _rx) = mpsc::channel(8);
        CodingWorkspaceEngine::new(failure_store, GitWorkspaceService::new(), tx)
            .fail_provider_stream::<()>(
                &failure_attempt,
                NODE_ID,
                "provider interrupted".to_string(),
            )
            .await
    });
    reached.await.expect("provider failure prewrite pause");

    let (tx, _rx) = mpsc::channel(8);
    CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx)
        .handle_abort(PROJECT_ID, ISSUE_ID, &attempt.id)
        .await
        .expect("abort paused provider failure");
    let attempt_after_abort = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("attempt after abort");
    let timeline_after_abort = store
        .get_timeline_nodes(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("timeline after abort");
    let runs_after_abort = store
        .list_role_runs(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("runs after abort");
    let gates_after_abort = store
        .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("gates after abort");
    resume.send(()).expect("resume provider failure");
    let error = failure
        .await
        .expect("provider failure task")
        .expect_err("aborted provider failure must fail closed");

    assert!(
        error
            .to_string()
            .contains("provider_failure_attempt_state_changed")
    );
    assert_eq!(
        store
            .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("attempt after stale failure"),
        attempt_after_abort
    );
    assert_eq!(
        store
            .get_timeline_nodes(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("timeline after stale failure"),
        timeline_after_abort
    );
    assert_eq!(
        store
            .list_role_runs(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("runs after stale failure"),
        runs_after_abort
    );
    assert_eq!(
        store
            .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("gates after stale failure"),
        gates_after_abort
    );
}

#[tokio::test]
async fn internal_review_blocked_gates_use_triage_actions_without_coder_rework() {
    for reason_code in [
        "group_final_review_blocked",
        "internal_review_blocked",
        "internal_review_change_requested",
        "internal_review_verification_incomplete",
        "internal_review_human_triage",
        "internal_review_operational_blocker",
    ] {
        let root = tempdir().expect("tempdir");
        let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: WORK_ITEM_ID.to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/issues/issue_0001".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Codex,
                    reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                    permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(
                    ),
                },
                max_auto_rework: 2,
            })
            .expect("attempt");
        let attempt = store
            .update_attempt_status(
                PROJECT_ID,
                ISSUE_ID,
                &attempt.id,
                CodingAttemptStatus::Running,
            )
            .expect("running attempt");
        let attempt = store
            .update_attempt_stage(
                PROJECT_ID,
                ISSUE_ID,
                &attempt.id,
                CodingExecutionStage::InternalPrReview,
            )
            .expect("internal review stage");
        let (tx, _rx) = mpsc::channel(8);
        let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

        engine
            .create_review_blocked_gate(ReviewBlockedGateInput {
                attempt: &attempt,
                node_id: "coding_node_0010",
                stage: CodingExecutionStage::InternalPrReview,
                role: CodingProviderRole::InternalReviewer,
                title: "Internal PR review blocked".to_string(),
                description: "review interrupted".to_string(),
                reason_code,
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
            })
            .await
            .expect("internal review blocked gate");

        let persisted = store
            .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("blocked attempt");
        assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
        let gate = store
            .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("open gates")
            .into_iter()
            .next()
            .expect("internal review gate");
        assert_eq!(gate.reason_code.as_deref(), Some(reason_code));
        assert_eq!(
            gate.available_actions
                .iter()
                .map(|action| action.action_id.as_str())
                .collect::<Vec<_>>(),
            vec!["retry_internal_review", "manual_continue", "abort"]
        );
        assert_eq!(gate.available_actions[0].label, "重试 Internal Review");
    }
}

#[tokio::test]
async fn retry_coding_gate_clears_latest_coder_provider_conversation() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let mut role_provider_config = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role provider config");
    role_provider_config.coder = ProviderName::ClaudeCode;
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            role_provider_config,
        )
        .expect("update role provider config");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt,
            vec![
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::Codex,
                    provider_session_id: "old-codex-coder-thread".to_string(),
                    updated_at: "2026-07-13T00:00:00Z".to_string(),
                    last_node_id: Some("coding_node_0001".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "latest-claude-coder-thread".to_string(),
                    updated_at: "2026-07-13T00:00:01Z".to_string(),
                    last_node_id: Some("coding_node_0002".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::CodeReviewer,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "review-thread".to_string(),
                    updated_at: "2026-07-13T00:00:02Z".to_string(),
                    last_node_id: Some("coding_node_0003".to_string()),
                },
            ],
        )
        .expect("seed provider conversations");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
        )
        .expect("coding stage");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked attempt");
    let gate = store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Coding,
                node_id: Some("coding_node_0002".to_string()),
                role: Some(CodingProviderRole::Coder),
                title: "Coder 执行中断".to_string(),
                description: "provider interrupted".to_string(),
                reason_code: Some("coder_provider_interrupted".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: vec![
                    coding_gate_action_for_id("retry_coding").expect("retry coding action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            },
        )
        .expect("coder recovery gate");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let resumed = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            "retry_coding",
            None,
        )
        .await
        .expect("retry coding gate response");

    assert_eq!(resumed.status, CodingAttemptStatus::Running);
    assert_eq!(resumed.stage, CodingExecutionStage::Coding);
    assert!(resumed.provider_conversations.iter().any(|conversation| {
        conversation.role == ProviderConversationRole::Coder
            && conversation.provider == ProviderName::Codex
            && conversation.provider_session_id == "old-codex-coder-thread"
    }));
    assert!(resumed.provider_conversations.iter().all(|conversation| {
        !(conversation.role == ProviderConversationRole::Coder
            && conversation.provider == ProviderName::ClaudeCode)
    }));
    assert!(resumed.provider_conversations.iter().any(|conversation| {
        conversation.role == ProviderConversationRole::CodeReviewer
            && conversation.provider_session_id == "review-thread"
    }));
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("resolved gate")
            .is_empty()
    );
}
