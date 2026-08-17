use super::*;
use crate::product::coding_attempt_store::CodingGitOperationPhase;
use crate::web::test_controls::pause_next_git_command_after_exit;

#[tokio::test]
async fn worktree_prepare_persists_completed_git_operation_journal() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_test_git_repo(&repo);
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
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("attempt");
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    let updated = engine
        .execute_worktree_prepare(&attempt, &repo)
        .await
        .expect("worktree prepare");

    let journal = store
        .get_coding_git_operation(&updated)
        .expect("journal read")
        .expect("journal persisted");
    assert_eq!(journal.phase, CodingGitOperationPhase::Completed);
    assert_eq!(updated.worktree_path.as_ref(), Some(&journal.worktree_path));
    assert!(journal.worktree_path.exists());
    assert_eq!(
        git_stdout(&repo, &["rev-parse", &journal.branch_name]).trim(),
        journal.before_head
    );
}

#[tokio::test]
async fn worktree_prepare_issues_evidence_token_and_rewrites_per_attempt() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_test_git_repo(&repo);
    let base_branch = git_stdout(&repo, &["branch", "--show-current"])
        .trim()
        .to_string();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    let create_attempt = |branch_name: &str| {
        store
            .create_attempt(CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: base_branch.clone(),
                branch_name: branch_name.to_string(),
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
            .expect("create attempt")
    };

    let attempt = create_attempt("aria/work-items/work_item_0001/attempt-1");
    let updated = engine
        .execute_worktree_prepare(&attempt, &repo)
        .await
        .expect("first worktree prepare");
    let first_worktree = updated.worktree_path.clone().expect("first worktree path");
    let first_token = fs::read_to_string(first_worktree.join(".aria/evidence-token"))
        .expect("first evidence token");
    assert!(!first_token.trim().is_empty(), "token must be non-empty");

    // 关闭首个 attempt（Created → Aborted），腾出同 work item 的 active 槽位，
    // 以便创建 attempt_no=2 的新 attempt，验证令牌按 attempt 重写。
    store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Aborted,
        )
        .expect("abort first attempt");

    let second = create_attempt("aria/work-items/work_item_0001/attempt-2");
    let second_updated = engine
        .execute_worktree_prepare(&second, &repo)
        .await
        .expect("second worktree prepare");
    let second_token = fs::read_to_string(
        second_updated
            .worktree_path
            .as_ref()
            .expect("second worktree path")
            .join(".aria/evidence-token"),
    )
    .expect("second evidence token");
    assert!(
        !second_token.trim().is_empty(),
        "second token must be non-empty"
    );
    assert_ne!(
        first_token, second_token,
        "token must be rewritten per attempt"
    );
}

#[tokio::test]
async fn cancellation_after_branch_exit_compensates_before_returning_aborted() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_test_git_repo(&repo);
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
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("attempt");
    let cancellation = CancellationToken::new();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .with_cancellation(cancellation.clone());
    let pause = pause_next_git_command_after_exit(&repo, "branch aria/work-items/");
    let execute = tokio::spawn({
        let attempt = attempt.clone();
        let repo = repo.clone();
        async move { engine.execute_worktree_prepare(&attempt, &repo).await }
    });
    pause.wait_until_reached().await;
    cancellation.cancel();
    pause.release();

    let result = execute.await.expect("execute task");
    assert!(matches!(result, Err(CodingWorkspaceEngineError::Aborted)));
    let branch_ref = format!("refs/heads/{}", attempt.branch_name);
    assert!(
        !StdCommand::new("git")
            .args(["show-ref", "--verify", "--quiet", &branch_ref])
            .current_dir(&repo)
            .status()
            .expect("probe branch")
            .success()
    );
    let journal = store
        .get_coding_git_operation(&attempt)
        .expect("journal")
        .expect("journal persisted");
    assert_eq!(journal.phase, CodingGitOperationPhase::Compensated);
    assert!(attempt.worktree_path.is_none());
}

#[tokio::test]
async fn cancellation_after_worktree_add_exit_removes_worktree_and_branch_before_return() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_test_git_repo(&repo);
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
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("attempt");
    let cancellation = CancellationToken::new();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .with_cancellation(cancellation.clone());
    let pause = pause_next_git_command_after_exit(&repo, "worktree add ");
    let execute = tokio::spawn({
        let attempt = attempt.clone();
        let repo = repo.clone();
        async move { engine.execute_worktree_prepare(&attempt, &repo).await }
    });
    pause.wait_until_reached().await;
    cancellation.cancel();
    pause.release();

    let result = execute.await.expect("execute task");
    assert!(matches!(result, Err(CodingWorkspaceEngineError::Aborted)));
    let journal = store
        .get_coding_git_operation(&attempt)
        .expect("journal")
        .expect("journal persisted");
    assert_eq!(journal.phase, CodingGitOperationPhase::Compensated);
    assert!(!journal.worktree_path.exists());
    assert_eq!(
        GitWorkspaceService::new()
            .git_local_branch_head(&repo, &attempt.branch_name)
            .await
            .expect("branch probe"),
        None
    );
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("attempt after compensation")
            .worktree_path,
        None
    );
}

#[tokio::test]
async fn cancellation_after_commit_exit_mixed_resets_head_and_preserves_changes() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_test_git_repo(&repo);
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
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("attempt");
    let (prepare_tx, _prepare_rx) = tokio::sync::mpsc::channel(8);
    let prepared =
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), prepare_tx)
            .execute_worktree_prepare(&attempt, &repo)
            .await
            .expect("prepare worktree");
    let worktree = prepared.worktree_path.clone().expect("worktree path");
    let before_head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    fs::write(worktree.join("feature.txt"), "preserve me\n").expect("feature change");

    let cancellation = CancellationToken::new();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .with_cancellation(cancellation.clone());
    let pause = pause_next_git_command_after_exit(&worktree, "commit -m feat: cancellation test");
    let execute = tokio::spawn({
        let prepared = prepared.clone();
        async move {
            engine
                .execute_review_request(&prepared, "origin", "feat: cancellation test")
                .await
        }
    });
    pause.wait_until_reached().await;
    cancellation.cancel();
    pause.release();

    let result = execute.await.expect("review request task");
    assert!(matches!(result, Err(CodingWorkspaceEngineError::Aborted)));
    assert_eq!(
        git_stdout(&worktree, &["rev-parse", "HEAD"]).trim(),
        before_head
    );
    assert_eq!(
        fs::read_to_string(worktree.join("feature.txt")).expect("preserved feature"),
        "preserve me\n"
    );
    assert!(git_stdout(&worktree, &["status", "--porcelain"]).contains("feature.txt"));
    let authoritative = store
        .get_attempt(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .expect("attempt after commit compensation");
    let journal = store
        .get_coding_git_operation(&authoritative)
        .expect("journal")
        .expect("review journal");
    assert_eq!(journal.phase, CodingGitOperationPhase::Compensated);
    assert!(
        store
            .list_review_requests(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .expect("review requests")
            .is_empty()
    );
}

#[tokio::test]
async fn cancellation_after_successful_push_exit_records_authoritative_completion() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    let remote = root.path().join("remote.git");
    fs::create_dir_all(&repo).expect("repo dir");
    fs::create_dir_all(&remote).expect("remote dir");
    init_test_git_repo(&repo);
    run_test_git(&remote, &["init", "--bare"]);
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
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("attempt");
    let (prepare_tx, _prepare_rx) = tokio::sync::mpsc::channel(8);
    let prepared =
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), prepare_tx)
            .execute_worktree_prepare(&attempt, &repo)
            .await
            .expect("prepare worktree");
    let worktree = prepared.worktree_path.clone().expect("worktree path");
    fs::write(worktree.join("feature.txt"), "push me\n").expect("feature change");

    let cancellation = CancellationToken::new();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .with_cancellation(cancellation.clone());
    let pause = pause_next_git_command_after_exit(&worktree, "push origin ");
    let execute = tokio::spawn({
        let prepared = prepared.clone();
        async move {
            engine
                .execute_review_request(&prepared, "origin", "feat: pushed cancellation")
                .await
        }
    });
    pause.wait_until_reached().await;
    cancellation.cancel();
    pause.release();

    let request = execute
        .await
        .expect("review request task")
        .expect("remote completion is authoritative");
    assert_eq!(request.push_status, PushStatus::Pushed);
    let remote_ref = format!("refs/heads/{}", prepared.branch_name);
    assert_eq!(
        git_stdout(&remote, &["rev-parse", &remote_ref]).trim(),
        request.commit_sha
    );
    let authoritative = store
        .get_attempt(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .expect("attempt after push reconciliation");
    assert_eq!(
        authoritative.review_request_id.as_deref(),
        Some(request.id.as_str())
    );
    assert_eq!(
        authoritative.head_commit.as_deref(),
        Some(request.commit_sha.as_str())
    );
    let journal = store
        .get_coding_git_operation(&authoritative)
        .expect("journal")
        .expect("review journal");
    assert_eq!(journal.phase, CodingGitOperationPhase::Completed);
    assert_eq!(journal.push_status, Some(PushStatus::Pushed));
}

#[tokio::test]
async fn cancellation_after_push_exit_keeps_ambiguous_push_started_and_local_commit() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    let remote = root.path().join("remote.git");
    fs::create_dir_all(&repo).expect("repo dir");
    fs::create_dir_all(&remote).expect("remote dir");
    init_test_git_repo(&repo);
    run_test_git(&remote, &["init", "--bare"]);
    let hook = remote.join("hooks/pre-receive");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("rejecting hook");
    let mut permissions = fs::metadata(&hook).expect("hook metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&hook, permissions).expect("hook executable");
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
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("attempt");
    let (prepare_tx, _prepare_rx) = tokio::sync::mpsc::channel(8);
    let prepared =
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), prepare_tx)
            .execute_worktree_prepare(&attempt, &repo)
            .await
            .expect("prepare worktree");
    let worktree = prepared.worktree_path.clone().expect("worktree path");
    let before_head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    fs::write(worktree.join("feature.txt"), "keep after rejected push\n").expect("feature change");

    let cancellation = CancellationToken::new();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .with_cancellation(cancellation.clone());
    let pause = pause_next_git_command_after_exit(&worktree, "push origin ");
    let execute = tokio::spawn({
        let prepared = prepared.clone();
        async move {
            engine
                .execute_review_request(&prepared, "origin", "feat: rejected cancellation")
                .await
        }
    });
    pause.wait_until_reached().await;
    cancellation.cancel();
    pause.release();

    let result = execute.await.expect("review request task");
    assert!(matches!(result, Err(CodingWorkspaceEngineError::Aborted)));
    let committed_head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    assert_ne!(committed_head, before_head);
    assert_eq!(
        fs::read_to_string(worktree.join("feature.txt")).expect("preserved feature"),
        "keep after rejected push\n"
    );
    assert_eq!(
        GitWorkspaceService::new()
            .git_remote_branch_head(&worktree, "origin", &prepared.branch_name)
            .await
            .expect("remote probe"),
        None
    );
    let authoritative = store
        .get_attempt(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .expect("attempt after ambiguous push cancellation");
    let journal = store
        .get_coding_git_operation(&authoritative)
        .expect("journal")
        .expect("review journal");
    assert_eq!(journal.phase, CodingGitOperationPhase::PushStarted);
    assert_eq!(journal.commit_sha.as_deref(), Some(committed_head.as_str()));
    assert_eq!(journal.push_status, None);
    assert!(
        store
            .list_review_requests(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .expect("review requests")
            .is_empty()
    );

    let (abort_tx, _abort_rx) = tokio::sync::mpsc::channel(8);
    let abort_error =
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), abort_tx)
            .handle_abort(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .await
            .expect_err("ambiguous push must block terminal abort");
    assert!(matches!(
        abort_error,
        CodingWorkspaceEngineError::Git(GitWorkspaceError::PushIndeterminate { .. })
    ));
    let still_running = store
        .get_attempt(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .expect("attempt remains retryable");
    assert_eq!(still_running.status, authoritative.status);
    assert_eq!(
        store
            .get_coding_git_operation(&still_running)
            .expect("journal after blocked abort")
            .expect("push journal")
            .phase,
        CodingGitOperationPhase::PushStarted
    );
}

#[tokio::test]
async fn abort_reconciles_unjournaled_worktree_side_effect_before_aborted_status() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_test_git_repo(&repo);
    let base_branch = git_stdout(&repo, &["branch", "--show-current"])
        .trim()
        .to_string();
    let before_head = git_stdout(&repo, &["rev-parse", &base_branch])
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
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("attempt");
    let worktree = worktree_path_for_attempt(&repo, &attempt);
    let journal = store
        .prepare_coding_git_operation(
            &attempt,
            PrepareCodingGitOperationInput {
                kind: CodingGitOperationKind::WorktreePrepare,
                repo_path: repo.clone(),
                worktree_path: worktree.clone(),
                branch_name: attempt.branch_name.clone(),
                base_branch: attempt.base_branch.clone(),
                before_head,
                remote: None,
                commit_message: None,
            },
        )
        .expect("prepare journal");
    let git = GitWorkspaceService::new();
    git.create_branch(&repo, &attempt.branch_name, &attempt.base_branch)
        .await
        .expect("create branch");
    let journal = store
        .advance_coding_git_operation(
            &attempt,
            &journal,
            CodingGitOperationPhase::BranchCreated,
            None,
        )
        .expect("branch phase");
    git.create_worktree(&repo, &attempt.branch_name, &worktree)
        .await
        .expect("create worktree side effect");

    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let aborted = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("abort reconciles");

    assert_eq!(aborted.status, CodingAttemptStatus::Aborted);
    assert!(!worktree.exists());
    assert_eq!(
        git.git_local_branch_head(&repo, &attempt.branch_name)
            .await
            .expect("branch probe"),
        None
    );
    let reconciled = store
        .get_coding_git_operation(&aborted)
        .expect("journal")
        .expect("journal persisted");
    assert_eq!(journal.phase, CodingGitOperationPhase::BranchCreated);
    assert_eq!(reconciled.phase, CodingGitOperationPhase::Compensated);
}
