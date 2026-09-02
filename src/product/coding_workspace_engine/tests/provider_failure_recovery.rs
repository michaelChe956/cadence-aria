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

pub(super) struct TransportFailuresThenSuccessProvider {
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

pub(super) enum RetryBoundaryMutation {
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

pub(super) struct RetryBoundaryMutationProvider {
    starts: AtomicUsize,
    mutation: Mutex<Option<RetryBoundaryMutation>>,
}

impl RetryBoundaryMutationProvider {
    pub(super) fn new(mutation: RetryBoundaryMutation) -> Self {
        Self {
            starts: AtomicUsize::new(0),
            mutation: Mutex::new(Some(mutation)),
        }
    }
}

impl TransportFailuresThenSuccessProvider {
    pub(super) fn new(failures_before_success: usize, output: impl Into<String>) -> Self {
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
    let lifecycle = LifecycleStore::new(store.paths());
    let shared = lifecycle
        .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        .expect("shared worktree")
        .expect("shared worktree exists");
    assert_eq!(shared.current_active_work_item_id, None);
    assert_eq!(shared.current_lock_owner_id, None);
    let replacement = lifecycle
        .try_acquire_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0002",
            "coding_attempt_after_cancel",
        )
        .expect("another work item can acquire the released shared lock");
    assert!(replacement.acquired);
    assert_eq!(
        replacement.worktree.current_active_work_item_id.as_deref(),
        Some("work_item_0002")
    );
    assert_eq!(
        replacement.worktree.current_lock_owner_id.as_deref(),
        Some("coding_attempt_after_cancel")
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
async fn reviewer_transport_failure_then_invalid_structured_output_stops_without_retry_budget() {
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
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let report = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect("invalid structured output blocks review without a third retry");

    assert_eq!(report.verdict, ReviewVerdict::Blocked);
    assert!(report.summary.contains("不是有效 JSON"));
    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 2);
    assert_eq!(
        inputs[0].resume_provider_session_id.as_deref(),
        Some("reviewer-session-before-retry")
    );
    assert_eq!(inputs[1].resume_provider_session_id, None);
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("blocked attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reviewer role runs");
    assert_eq!(runs.len(), 2, "invalid output must not start a third retry");
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(runs[1].status, CodingRoleRunStatus::Blocked);
    assert_eq!(runs[0].trigger, CodingRoleRunTrigger::Initial);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::AutomaticRetry);
    assert_eq!(runs[1].retry_metadata.as_ref().unwrap().attempt_no, 2);
    assert_eq!(runs[1].reason_code.as_deref(), Some("code_review_blocked"));
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reviewer failure gate");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].reason_code.as_deref(), Some("code_review_blocked"));
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
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("attempt");
    lifecycle
        .bind_issue_worktree_lock_to_attempt(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID, &attempt.id)
        .expect("bind owner");
    let attempt = store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &attempt.id)
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

mod logical_legacy_fallback;
mod retry_and_gate;
mod reviewer_recovery;
