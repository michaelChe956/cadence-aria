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

/// 现场形态（2026-10-14 组删 500，coding_attempt_8d1183a5…）：仓库无任何
/// 远端（journal.remote 是 push 启动时的默认 origin），runner 死在
/// review_request/push_started。远端未配置 = push 确定未发生——终止
/// reconcile 不得跑 ls-remote（无远端仓必 fatal → CommandFailed 500），应按
/// S2a 本地交付同模式以 SkippedNotConfigured 确定性收完 journal。
#[tokio::test]
async fn termination_reconcile_completes_push_started_journal_when_remote_not_configured() {
    use crate::product::coding_models::PushStatus;

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
    let head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    // 模拟 runner 死在 push_started：journal 停在 push_started（现场
    // commit_sha == before_head 同形），无 push 终态。
    let journal = store
        .prepare_coding_git_operation(
            &prepared,
            PrepareCodingGitOperationInput {
                kind: CodingGitOperationKind::ReviewRequest,
                repo_path: repo.canonicalize().expect("canonical repo"),
                worktree_path: worktree.canonicalize().expect("canonical worktree"),
                branch_name: prepared.branch_name.clone(),
                base_branch: prepared.base_branch.clone(),
                before_head: head.clone(),
                remote: Some("origin".to_string()),
                commit_message: Some("feat: implement work item".to_string()),
            },
        )
        .expect("seed review journal");
    let journal = store
        .advance_coding_git_operation(
            &prepared,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .expect("advance commit started");
    let journal = store
        .advance_coding_git_operation(
            &prepared,
            &journal,
            CodingGitOperationPhase::CommitCreated,
            Some(head.clone()),
        )
        .expect("advance commit created");
    let journal = store
        .advance_coding_git_operation(
            &prepared,
            &journal,
            CodingGitOperationPhase::PushStarted,
            None,
        )
        .expect("advance push started");
    assert_eq!(journal.phase, CodingGitOperationPhase::PushStarted);

    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let reconciled =
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
            .reconcile_coding_git_operation_for_termination(&prepared)
            .await
            .expect("termination reconcile converges unconfigured push journal");
    assert_eq!(reconciled.id, prepared.id);

    let journal = store
        .get_coding_git_operation(&reconciled)
        .expect("journal read")
        .expect("journal persisted");
    assert_eq!(journal.phase, CodingGitOperationPhase::Completed);
    assert_eq!(journal.push_status, Some(PushStatus::SkippedNotConfigured));
    assert_eq!(journal.push_error, None);
    assert!(journal.review_request_id.is_some());
    // 收敛只补 journal 终态，不动本地：head 保持原位。
    assert_eq!(git_stdout(&worktree, &["rev-parse", "HEAD"]).trim(), head);
}

// ---------------------------------------------------------------------------
// T19:平台写通道守卫(窗口冻结期间 stage/commit/push/补偿拒绝;窗外不变)
// ---------------------------------------------------------------------------

use crate::cross_cutting::provider_resources::{
    ProviderRunResourceScope, ResourceScopeKey, new_boot_identity,
};
use crate::product::reviewer_snapshot::{ReviewerSnapshotStore, SnapshotBudget as T19Budget};
use crate::product::reviewer_window::{ReviewerWindowBeginRequest, begin_review_window};
use crate::product::reviewer_window_store::ReviewerWindowStore as T19WindowStore;

/// T17 API 直连开窗(单主 scope;空登记 Quiescent 即合法静止证明)。
async fn t19_open_window(
    root: &Path,
    attempt: &CodingExecutionAttempt,
    worktree: &Path,
    window_id: &str,
) -> crate::product::reviewer_window::ReviewerWindowHandle {
    let window_store = T19WindowStore::new(root.join(".aria").join("reviewer-windows"));
    let snapshot_store = ReviewerSnapshotStore::new(root.join(".aria").join("reviewer-snapshots"));
    let private = root.join("t19-scope-private").join(window_id);
    fs::create_dir_all(&private).expect("scope private root");
    let scope_store_root = root.join("t19-scope-store");
    let key = ResourceScopeKey {
        scope_id: format!("t19-{window_id}"),
        run_id: format!("run-{window_id}"),
        boot_identity: new_boot_identity(),
    };
    let scope = ProviderRunResourceScope::open_for_recovery(
        &key,
        Some(window_id),
        &private,
        &scope_store_root,
        None,
    )
    .expect("open scope");
    let targets: [&ProviderRunResourceScope; 1] = [&scope];
    begin_review_window(
        &window_store,
        &snapshot_store,
        &ReviewerWindowBeginRequest {
            window_id,
            attempt_id: &attempt.id,
            run_id: &format!("run-{window_id}"),
            provider: "claude_code",
            worktree,
            quiesce_targets: &targets,
            platform_write_paths: Vec::new(),
            budget: T19Budget::generous(),
            drain_timeout: std::time::Duration::from_secs(5),
        },
    )
    .await
    .expect("open reviewer window")
}

#[tokio::test]
async fn review_request_platform_write_rejected_while_window_frozen_resumes_after_release() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("deliver.rs"), "pub fn deliver() {}\n").expect("coder delivery");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let handle = t19_open_window(root.path(), &attempt, &worktree, "t19-guard-window").await;
    let head_before = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let staged_before = git_stdout(&worktree, &["diff", "--cached", "--name-only"]);

    // 窗口冻结期间平台交付(stage/commit/push)拒绝,无 Git mutation。
    let error = engine
        .execute_review_request(&attempt, "origin", "feat: t19 delivery")
        .await
        .expect_err("frozen window must reject platform git write");
    assert!(
        error.to_string().contains("reviewer_window_channel"),
        "unexpected error: {error}"
    );
    assert_eq!(
        git_stdout(&worktree, &["rev-parse", "HEAD"]).trim(),
        head_before,
        "no commit may land while frozen"
    );
    assert_eq!(
        git_stdout(&worktree, &["diff", "--cached", "--name-only"]),
        staged_before,
        "no stage may land while frozen"
    );

    // 干净回收释放锁(T19 生命周期面)。
    engine
        .recycle_reviewer_window(&handle, "t19_teardown")
        .await
        .expect("clean recycle releases the tree");

    // 正常 Coder 窗外交付不变:交付照常完成。
    let request = engine
        .execute_review_request(&attempt, "origin", "feat: t19 delivery")
        .await
        .expect("delivery outside window succeeds");
    assert_eq!(request.push_status, PushStatus::SkippedNotConfigured);
    assert_ne!(
        git_stdout(&worktree, &["rev-parse", "HEAD"]).trim(),
        head_before,
        "delivery commit must land after window release"
    );
    assert!(
        store
            .get_coding_git_operation(&attempt)
            .expect("journal")
            .expect("journal persisted")
            .is_terminal()
    );
}

#[tokio::test]
async fn cancelled_review_commit_compensation_rejected_while_window_frozen() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    // journal 推进到 CommitCreated(真实 commit 已存在;通道依次进出,
    // CommitCreated 相位无在途登记)。
    let before_head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    fs::write(worktree.join("committed.rs"), "pub fn committed() {}\n").expect("staged change");
    git_stdout(&worktree, &["add", "."]);
    git_stdout(
        &worktree,
        &[
            "-c",
            "user.email=t19@aria.invalid",
            "-c",
            "user.name=t19",
            "commit",
            "-qm",
            "t19 commit",
        ],
    );
    let commit_sha = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let journal = store
        .prepare_coding_git_operation(
            &attempt,
            crate::product::coding_attempt_store::PrepareCodingGitOperationInput {
                kind: crate::product::coding_attempt_store::CodingGitOperationKind::ReviewRequest,
                repo_path: worktree.clone(),
                worktree_path: worktree.clone(),
                branch_name: attempt.branch_name.clone(),
                base_branch: attempt.base_branch.clone(),
                before_head: before_head.clone(),
                remote: Some("origin".to_string()),
                commit_message: Some("t19 commit".to_string()),
            },
        )
        .expect("prepare journal");
    let journal = store
        .advance_coding_git_operation(
            &attempt,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .expect("commit started");
    let journal = store
        .advance_coding_git_operation(
            &attempt,
            &journal,
            CodingGitOperationPhase::CommitCreated,
            Some(commit_sha),
        )
        .expect("commit created");

    // 冻结窗口(无在途 writer,直接 Frozen)。
    let handle = t19_open_window(root.path(), &attempt, &worktree, "t19-comp-window").await;

    // 补偿(reset)被拒:窗口冻结期间平台补偿同样拒绝。
    let error = engine
        .compensate_cancelled_review_commit(&attempt, &journal)
        .await
        .expect_err("frozen window must reject compensation");
    assert!(
        error.to_string().contains("reviewer_window_channel"),
        "unexpected error: {error}"
    );
    assert_eq!(
        git_stdout(&worktree, &["rev-parse", "HEAD"]).trim(),
        journal.commit_sha.clone().expect("commit sha"),
        "compensation reset must not run while frozen"
    );
    assert_eq!(
        store
            .get_coding_git_operation(&attempt)
            .expect("journal")
            .expect("journal persisted")
            .phase,
        CodingGitOperationPhase::CommitCreated,
        "journal stays at CommitCreated for later compensation retry"
    );

    // 释放后补偿可续接(终止收敛面)。
    engine
        .recycle_reviewer_window(&handle, "t19_teardown")
        .await
        .expect("recycle");
    engine
        .compensate_cancelled_review_commit(&attempt, &journal)
        .await
        .expect("compensation resumes after window release");
    assert_eq!(
        git_stdout(&worktree, &["rev-parse", "HEAD"]).trim(),
        before_head,
        "reset restores before_head after release"
    );
}

// ---------------------------------------------------------------------------
// T19 修复轮 1:F1 补偿 mid-flow 冻结钉测 + resume/terminate 参数化用例
// ---------------------------------------------------------------------------

#[tokio::test]
async fn compensation_reguards_after_commit_created_advance_when_frozen_mid_flow() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    // journal 停在 CommitStarted,真实 commit 已存在(head != before_head):
    // 补偿 inner 会先 advance(CommitCreated)——其自愈式 exit-all 清掉
    // 外层 Compensation 登记——随后才 reset。
    let before_head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    fs::write(worktree.join("committed.rs"), "pub fn committed() {}\n").expect("staged change");
    git_stdout(&worktree, &["add", "."]);
    git_stdout(
        &worktree,
        &[
            "-c",
            "user.email=t19@aria.invalid",
            "-c",
            "user.name=t19",
            "commit",
            "-qm",
            "t19 midflow commit",
        ],
    );
    let commit_sha = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let journal = store
        .prepare_coding_git_operation(
            &attempt,
            crate::product::coding_attempt_store::PrepareCodingGitOperationInput {
                kind: crate::product::coding_attempt_store::CodingGitOperationKind::ReviewRequest,
                repo_path: worktree.clone(),
                worktree_path: worktree.clone(),
                branch_name: attempt.branch_name.clone(),
                base_branch: attempt.base_branch.clone(),
                before_head: before_head.clone(),
                remote: Some("origin".to_string()),
                commit_message: Some("t19 midflow commit".to_string()),
            },
        )
        .expect("prepare journal");
    let journal = store
        .advance_coding_git_operation(
            &attempt,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .expect("commit started");

    // 复刻外层 enter(Compensation) 成功(冻结前的窗口期)。
    engine
        .guard_platform_write_channel(
            &journal.worktree_path,
            crate::product::reviewer_window_store::WriteChannelKind::Compensation,
            &attempt.id,
        )
        .expect("outer compensation enter while gate open");
    // 复刻 inner 第一段:advance(CommitCreated) 的 exit-all 清登记。
    let journal = store
        .advance_coding_git_operation(
            &attempt,
            &journal,
            CodingGitOperationPhase::CommitCreated,
            Some(commit_sha),
        )
        .expect("commit created advance clears channel registration");
    // 冻结恰好发生在 enter 之后(间隙窗口):
    let handle = t19_open_window(root.path(), &attempt, &worktree, "t19-midflow-window").await;

    // inner 直驱:re-guard 必须拒绝,reset 不在冻结窗口内裸跑。
    let error = engine
        .compensate_cancelled_review_commit_inner(&attempt, &journal)
        .await
        .expect_err("mid-flow freeze must reject the compensation reset");
    assert!(
        error.to_string().contains("reviewer_window_channel"),
        "unexpected error: {error}"
    );
    assert_eq!(
        git_stdout(&worktree, &["rev-parse", "HEAD"]).trim(),
        journal.commit_sha.clone().expect("commit sha"),
        "reset must not run inside the frozen window"
    );

    engine
        .recycle_reviewer_window(&handle, "t19 teardown")
        .await
        .expect("recycle");
    engine
        .compensate_cancelled_review_commit_inner(&attempt, &journal)
        .await
        .expect("compensation resumes after release");
    assert_eq!(
        git_stdout(&worktree, &["rev-parse", "HEAD"]).trim(),
        before_head,
        "reset restores before_head after release"
    );
}

#[tokio::test]
async fn terminated_attempt_reconcile_rejected_while_window_frozen() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    // journal 推到 CommitCreated(终止路径的补偿输入形态)。
    let before_head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    fs::write(worktree.join("committed.rs"), "pub fn terminated() {}\n").expect("staged change");
    git_stdout(&worktree, &["add", "."]);
    git_stdout(
        &worktree,
        &[
            "-c",
            "user.email=t19@aria.invalid",
            "-c",
            "user.name=t19",
            "commit",
            "-qm",
            "t19 terminate commit",
        ],
    );
    let commit_sha = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let journal = store
        .prepare_coding_git_operation(
            &attempt,
            crate::product::coding_attempt_store::PrepareCodingGitOperationInput {
                kind: crate::product::coding_attempt_store::CodingGitOperationKind::ReviewRequest,
                repo_path: worktree.clone(),
                worktree_path: worktree.clone(),
                branch_name: attempt.branch_name.clone(),
                base_branch: attempt.base_branch.clone(),
                before_head,
                remote: Some("origin".to_string()),
                commit_message: Some("t19 terminate commit".to_string()),
            },
        )
        .expect("prepare");
    let journal = store
        .advance_coding_git_operation(
            &attempt,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .expect("commit started");
    let journal = store
        .advance_coding_git_operation(
            &attempt,
            &journal,
            CodingGitOperationPhase::CommitCreated,
            Some(commit_sha),
        )
        .expect("commit created");

    let handle = t19_open_window(root.path(), &attempt, &worktree, "t19-terminate-window").await;

    // terminate(AbortAttempt 收口面):reconcile_for_termination 驱动补偿,
    // 冻结窗口内被拒——不 partial 收口、HEAD 不动。
    let error = engine
        .reconcile_coding_git_operation_for_termination(&attempt)
        .await
        .expect_err("termination reconcile must fail closed while frozen");
    assert!(
        error.to_string().contains("reviewer_window_channel"),
        "unexpected error: {error}"
    );
    assert_eq!(
        git_stdout(&worktree, &["rev-parse", "HEAD"]).trim(),
        journal.commit_sha.clone().expect("commit sha"),
        "termination compensation must not run while frozen"
    );

    engine
        .recycle_reviewer_window(&handle, "t19 teardown")
        .await
        .expect("recycle");
    let reconciled = engine
        .reconcile_coding_git_operation_for_termination(&attempt)
        .await
        .expect("termination reconcile resumes after release");
    assert_eq!(
        reconciled.status,
        crate::product::coding_models::CodingAttemptStatus::Running
    );
}
