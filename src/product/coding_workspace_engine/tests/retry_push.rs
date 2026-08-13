use super::*;
use crate::product::coding_attempt_store::CodingGitOperationPhase;
use crate::product::coding_models::PushStatus;

#[cfg(unix)]
#[tokio::test]
async fn execute_review_request_retries_push_after_failed_review_request() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    let remote = root.path().join("remote.git");
    fs::create_dir_all(&repo).expect("repo dir");
    fs::create_dir_all(&remote).expect("remote dir");
    init_test_git_repo(&repo);
    run_test_git(&remote, &["init", "--bare"]);
    // 注入拒绝 push 的 pre-receive hook，使第一次 push 必然失败。
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
    fs::write(worktree.join("feature.txt"), "retry me\n").expect("feature change");

    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    let failed = engine
        .execute_review_request(&prepared, "origin", "feat: retry push")
        .await
        .expect("first review request");
    assert_eq!(failed.push_status, PushStatus::Failed);
    let journal = store
        .get_coding_git_operation(&prepared)
        .expect("journal read")
        .expect("journal persisted");
    assert_eq!(journal.phase, CodingGitOperationPhase::Completed);
    assert_eq!(journal.push_status, Some(PushStatus::Failed));

    // 移除拒绝 hook 后重推应成功（等价于 RetryPush 触发 execute_review_request 重入）。
    fs::remove_file(&hook).expect("remove rejecting hook");
    let pushed = engine
        .execute_review_request(&prepared, "origin", "feat: retry push")
        .await
        .expect("retried review request");
    assert_eq!(pushed.push_status, PushStatus::Pushed);

    let journal = store
        .get_coding_git_operation(&prepared)
        .expect("journal read")
        .expect("journal after retry");
    assert_eq!(journal.phase, CodingGitOperationPhase::Completed);
    assert_eq!(journal.push_status, Some(PushStatus::Pushed));

    let attempt_after = store
        .get_attempt(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .expect("attempt after retry");
    assert_eq!(attempt_after.stage, CodingExecutionStage::ReviewRequest);
}
