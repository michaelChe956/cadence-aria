use super::*;
use crate::cross_cutting::streaming_provider::{ProviderCompletion, ProviderSession};
use crate::product::coding_workspace_engine::group_review_errors::GroupReviewExecutionError;
use crate::product::coding_workspace_engine::group_review_orchestrator::{
    GroupReviewExecutor, RealGroupReviewExecutor,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

struct GroupReviewOutputProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for GroupReviewOutputProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let output = format!("executed: {}", input.prompt);
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output,
                    Some("group-review-session".to_string()),
                )))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[derive(Default)]
struct GroupReviewCancellationProbe {
    entered: Notify,
    cancelled: AtomicBool,
}

struct GroupReviewCancellationProvider {
    probe: Arc<GroupReviewCancellationProbe>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for GroupReviewCancellationProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        let probe = self.probe.clone();
        tokio::spawn(async move {
            probe.entered.notify_one();
            cancel.cancelled().await;
            probe.cancelled.store(true, Ordering::SeqCst);
            drop(event_tx);
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

fn executor_fixture(
    cancellation: Option<CancellationToken>,
) -> (
    tempfile::TempDir,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
    CodingRoleRun,
) {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    init_test_git_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .expect("attempt");
    attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::InternalPrReview,
        )
        .expect("review stage");
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
            CodingRoleRunTrigger::Initial,
            Some("group_review_node".to_string()),
        )
        .expect("role run");
    let (event_tx, _event_rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx);
    let engine = match cancellation {
        Some(cancellation) => engine.with_cancellation(cancellation),
        None => engine,
    };
    (root, engine, attempt, role_run)
}

#[tokio::test]
async fn real_group_review_executor_returns_provider_full_output() {
    let (_root, engine, attempt, role_run) = executor_fixture(None);
    let provider = GroupReviewOutputProvider;
    let executor = RealGroupReviewExecutor::new(
        &engine,
        attempt,
        &provider,
        "group_review_node".to_string(),
        role_run,
        ProviderName::Codex,
    );

    let result = executor.execute("shard prompt").await.expect("execute");

    assert_eq!(result.full_output, "executed: shard prompt");
    assert_eq!(result.role_run_id.as_deref(), Some("coding_role_run_0001"));
}

#[tokio::test]
async fn real_group_review_executor_maps_engine_cancellation_to_user_cancelled() {
    let cancellation = CancellationToken::new();
    let (_root, engine, attempt, role_run) = executor_fixture(Some(cancellation.clone()));
    let probe = Arc::new(GroupReviewCancellationProbe::default());
    let provider = GroupReviewCancellationProvider {
        probe: probe.clone(),
    };
    let executor = RealGroupReviewExecutor::new(
        &engine,
        attempt,
        &provider,
        "group_review_node".to_string(),
        role_run,
        ProviderName::Codex,
    );

    let execute = executor.execute("cancel prompt");
    let cancel = async {
        tokio::time::timeout(
            std::time::Duration::from_millis(250),
            probe.entered.notified(),
        )
        .await
        .expect("provider start");
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(execute, cancel);

    assert!(matches!(
        result,
        Err(GroupReviewExecutionError::UserCancelled)
    ));
    tokio::time::timeout(std::time::Duration::from_millis(250), async {
        while !probe.cancelled.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider child token cancellation");
}
