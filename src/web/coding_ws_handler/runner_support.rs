use std::sync::Arc;

use tokio::sync::mpsc;

use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionAttempt};
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::json_store::ProductStoreError;
use crate::product::models::ProviderName;
use crate::web::provider_availability::host_real_workflow_ready;
use crate::web::state::WebAppState;
use crate::web::workspace_ws_handler::refresh_coding_runtime_revision_history;

use super::{
    CodingWsOutMessage, build_coding_session_state, emit_current_session_state,
    update_provider_selection,
};

pub(super) async fn recover_plan_amendment_if_needed(
    engine: &CodingWorkspaceEngine,
    attempt: &CodingExecutionAttempt,
) -> Result<(CodingExecutionAttempt, Option<String>), CodingWorkspaceEngineError> {
    if matches!(
        attempt.status,
        CodingAttemptStatus::AwaitingPlanAmendment
            | CodingAttemptStatus::ApplyingPlanAmendment
            | CodingAttemptStatus::AmendmentApplyFailed
    ) {
        let (recovered, child_session_id) = engine
            .recover_plan_amendment_with_history_session(attempt)
            .await?;
        return Ok((recovered, Some(child_session_id)));
    }
    Ok((attempt.clone(), None))
}

pub(super) fn refresh_runtime_revision_history(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    current_child_session_id: Option<&str>,
) -> Result<(), CodingWorkspaceEngineError> {
    if attempt.scope != crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
        return Ok(());
    }
    refresh_coding_runtime_revision_history(app_paths, attempt, current_child_session_id).map_err(
        |error| {
            CodingWorkspaceEngineError::ProviderStream(format!(
                "runtime_revision_history_refresh_failed: {error}"
            ))
        },
    )?;
    Ok(())
}

pub(super) async fn handle_pending_runner_commands(
    command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    coding_store: &CodingAttemptStore,
    engine: &CodingWorkspaceEngine,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    attempt: &CodingExecutionAttempt,
) -> Result<bool, CodingWorkspaceEngineError> {
    while let Ok(command) = command_rx.try_recv() {
        match command {
            CodingRunnerCommand::AbortAttempt => {
                let updated = engine
                    .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
                    .await?;
                emit_current_session_state(
                    event_tx,
                    coding_store,
                    &updated,
                    engine.cancellation_token(),
                )
                .await?;
                return Ok(true);
            }
            CodingRunnerCommand::ProviderSelect { role, provider } => {
                let (updated, changed_role, changed_provider) =
                    update_provider_selection(coding_store, attempt, &role, provider)?;
                let _ = engine
                    .event_tx
                    .send(CodingWsOutMessage::CodingProviderConfigUpdated {
                        role: changed_role,
                        provider: changed_provider,
                    })
                    .await;
                let _ = engine
                    .event_tx
                    .send(build_coding_session_state(coding_store, updated)?)
                    .await;
            }
            CodingRunnerCommand::StageGateConfirm { .. } => {}
            CodingRunnerCommand::RetryPush => {
                let _review_request = engine
                    .execute_review_request(attempt, "origin", "feat: implement work item")
                    .await?;
                let updated = coding_store.get_attempt(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                )?;
                emit_current_session_state(
                    event_tx,
                    coding_store,
                    &updated,
                    engine.cancellation_token(),
                )
                .await?;
            }
            CodingRunnerCommand::PermissionResponse { .. }
            | CodingRunnerCommand::ChoiceResponse { .. } => {}
        }
    }
    Ok(false)
}

pub(super) fn provider_for(
    state: &WebAppState,
    provider_name: &ProviderName,
    kind: &'static str,
) -> Result<Arc<dyn StreamingProviderAdapter>, CodingWorkspaceEngineError> {
    if !state.test_provider_enabled {
        state
            .provider_gate
            .ensure_available(provider_name)
            .map_err(|error| {
                CodingWorkspaceEngineError::Store(ProductStoreError::Io(format!(
                    "{}: {}",
                    error.code(),
                    error
                )))
            })?;
        host_real_workflow_ready().map_err(|error| {
            CodingWorkspaceEngineError::Store(ProductStoreError::Io(format!(
                "{}: {}",
                error.code, error.message
            )))
        })?;
    }
    state.provider_registry.get(provider_name).ok_or_else(|| {
        CodingWorkspaceEngineError::Store(ProductStoreError::NotFound {
            kind,
            id: format!("{provider_name:?}"),
        })
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::coding_attempt_store::{CodingGitOperationPhase, CreateCodingAttemptInput};
    use crate::product::coding_models::PushStatus;
    use crate::product::git_workspace_service::GitWorkspaceService;
    use crate::web::events::EventHub;
    use crate::web::runtime::WebRuntime;
    use crate::web::test_controls::{TestControlledFakeStreamingProvider, TestControls};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    fn init_test_git_repo(repo: &std::path::Path) {
        run_test_git(repo, &["init"]);
        run_test_git(repo, &["config", "user.email", "aria@example.com"]);
        run_test_git(repo, &["config", "user.name", "Aria Test"]);
        std::fs::write(repo.join("README.md"), "initial\n").expect("seed file");
        run_test_git(repo, &["add", "."]);
        run_test_git(repo, &["commit", "-m", "initial"]);
    }

    fn git_stdout(cwd: &std::path::Path, args: &[&str]) -> String {
        let output = run_test_git(cwd, args);
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn run_test_git(cwd: &std::path::Path, args: &[&str]) -> std::process::Output {
        use std::process::Command as StdCommand;
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|error| panic!("git {} failed to start: {error}", args.join(" ")));
        if !output.status.success() {
            panic!(
                "git {} failed\nstdout:\n{}\nstderr:\n{}",
                args.join(" "),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        output
    }

    #[test]
    fn coding_provider_for_rejects_real_provider_when_health_is_unavailable() {
        let root = tempdir().expect("workspace");
        let state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_real(root.path().to_path_buf()).expect("real runtime"),
        );

        let error = provider_for(&state, &ProviderName::Codex, "coding provider")
            .err()
            .expect("degraded initial health must reject real provider");

        assert!(error.to_string().contains("provider_unavailable"));
    }

    #[test]
    fn coding_provider_for_allows_fake_provider_from_test_registry() {
        let root = tempdir().expect("workspace");
        let mut registry = ProviderRegistry::new();
        registry.register(
            ProviderName::Fake,
            Arc::new(TestControlledFakeStreamingProvider::new(
                TestControls::default(),
            )),
        );
        let state = WebAppState::with_events_and_provider_registry(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            EventHub::new(),
            Arc::new(registry),
        );

        assert!(provider_for(&state, &ProviderName::Fake, "coding provider").is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn handle_pending_runner_commands_retry_push_reopens_failed_push() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempdir().expect("root");
        let repo = root.path().join("repo");
        let remote = root.path().join("remote.git");
        std::fs::create_dir_all(&repo).expect("repo dir");
        std::fs::create_dir_all(&remote).expect("remote dir");
        init_test_git_repo(&repo);
        run_test_git(&remote, &["init", "--bare"]);
        let hook = remote.join("hooks/pre-receive");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("rejecting hook");
        let mut permissions = std::fs::metadata(&hook)
            .expect("hook metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&hook, permissions).expect("hook executable");
        run_test_git(
            &repo,
            &["remote", "add", "origin", &remote.to_string_lossy()],
        );
        let base_branch = git_stdout(&repo, &["branch", "--show-current"])
            .trim()
            .to_string();
        let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch,
                branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Codex,
                    reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                    permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(
                    ),
                },
                target_snapshot: None,
                max_auto_rework: 2,
            })
            .expect("attempt");
        let (prepare_tx, _prepare_rx) = mpsc::channel(8);
        let prepared =
            CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), prepare_tx)
                .execute_worktree_prepare(&attempt, &repo)
                .await
                .expect("prepare worktree");
        let worktree = prepared.worktree_path.clone().expect("worktree path");
        std::fs::write(worktree.join("feature.txt"), "retry me\n").expect("feature change");

        let (event_tx, _event_rx) = mpsc::channel(16);
        let engine =
            CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx.clone());

        let failed = engine
            .execute_review_request(&prepared, "origin", "feat: implement work item")
            .await
            .expect("first review request");
        assert_eq!(failed.push_status, PushStatus::Failed);
        let journal = store
            .get_coding_git_operation(&prepared)
            .expect("journal read")
            .expect("journal persisted");
        assert_eq!(journal.phase, CodingGitOperationPhase::Completed);
        assert_eq!(journal.push_status, Some(PushStatus::Failed));

        std::fs::remove_file(&hook).expect("remove rejecting hook");
        let current = store
            .get_attempt(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .expect("current attempt");

        let (command_tx, mut command_rx) = mpsc::channel(4);
        command_tx
            .send(CodingRunnerCommand::RetryPush)
            .await
            .expect("send retry push");
        let stopped =
            handle_pending_runner_commands(&mut command_rx, &store, &engine, &event_tx, &current)
                .await
                .expect("handle retry push");
        assert!(!stopped, "RetryPush must not stop the runner loop");

        let requests = store
            .list_review_requests(&current.project_id, &current.issue_id, &current.id)
            .expect("review requests");
        let request = requests.last().expect("review request after retry");
        assert_eq!(request.push_status, PushStatus::Pushed);
    }
}
