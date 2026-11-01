use super::*;
use crate::product::coding_attempt_store::CodingGitOperationPhase;
use crate::product::coding_models::PushStatus;
use std::path::PathBuf;

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

/// 无远端仓库交付夹具：repo 可选配置 origin；返回已 prepare 的 attempt 与其 worktree。
async fn prepare_local_delivery_attempt(
    root: &Path,
    origin: Option<&Path>,
) -> (CodingAttemptStore, CodingExecutionAttempt, PathBuf) {
    let repo = root.join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_test_git_repo(&repo);
    if let Some(origin) = origin {
        run_test_git(
            &repo,
            &["remote", "add", "origin", &origin.to_string_lossy()],
        );
    }
    let base_branch = git_stdout(&repo, &["branch", "--show-current"])
        .trim()
        .to_string();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.join(".aria")));
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
    (store, prepared, worktree)
}

/// 把 journal 推进到 PushStarted（worktree 内已有一次真实提交），复现现场形态：
/// 推送开始后进程死亡，恢复后 execute_review_request 重入。
fn seed_push_started_journal(
    store: &CodingAttemptStore,
    prepared: &CodingExecutionAttempt,
    worktree: &Path,
    commit_message: &str,
) -> String {
    let before_head = git_stdout(worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    fs::write(worktree.join("feature.txt"), "local delivery\n").expect("feature change");
    run_test_git(worktree, &["add", "-A"]);
    run_test_git(worktree, &["commit", "-m", commit_message]);
    let commit_sha = git_stdout(worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let mut journal = store
        .prepare_coding_git_operation(
            prepared,
            PrepareCodingGitOperationInput {
                kind: CodingGitOperationKind::ReviewRequest,
                repo_path: worktree.to_path_buf(),
                worktree_path: worktree.to_path_buf(),
                branch_name: prepared.branch_name.clone(),
                base_branch: prepared.base_branch.clone(),
                before_head,
                remote: Some("origin".to_string()),
                commit_message: Some(commit_message.to_string()),
            },
        )
        .expect("prepare journal");
    for (phase, sha) in [
        (CodingGitOperationPhase::CommitStarted, None),
        (
            CodingGitOperationPhase::CommitCreated,
            Some(commit_sha.clone()),
        ),
        (CodingGitOperationPhase::PushStarted, None),
    ] {
        journal = store
            .advance_coding_git_operation(prepared, &journal, phase, sha)
            .expect("advance journal");
    }
    commit_sha
}

#[tokio::test]
async fn execute_review_request_without_origin_delivers_locally() {
    let root = tempdir().expect("root");
    let (store, prepared, worktree) = prepare_local_delivery_attempt(root.path(), None).await;
    fs::write(worktree.join("feature.txt"), "local only\n").expect("feature change");
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    let request = engine
        .execute_review_request(&prepared, "origin", "feat: local delivery")
        .await
        .expect("no-origin review request must converge locally");

    assert_eq!(request.push_status, PushStatus::SkippedNotConfigured);
    assert_eq!(request.push_error, None);
    assert!(
        request
            .manual_instructions
            .iter()
            .all(|line| line.contains("本地交付") && !line.contains("基于远端")),
        "manual_instructions must describe local delivery: {:?}",
        request.manual_instructions
    );
    let journal = store
        .get_coding_git_operation(&prepared)
        .expect("journal read")
        .expect("journal persisted");
    assert_eq!(journal.phase, CodingGitOperationPhase::Completed);
    assert_eq!(journal.push_status, Some(PushStatus::SkippedNotConfigured));
    let attempt_after = store
        .get_attempt(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .expect("attempt after delivery");
    assert_eq!(
        attempt_after.review_request_id.as_deref(),
        Some(request.id.as_str())
    );
    let nodes = store
        .get_timeline_nodes(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .expect("timeline nodes");
    let node = nodes
        .iter()
        .find(|node| node.stage == CodingExecutionStage::ReviewRequest)
        .expect("review request timeline node");
    assert_eq!(node.status, CodingTimelineNodeStatus::Completed);
    assert!(
        node.summary
            .as_deref()
            .is_some_and(|s| s.contains("本地交付")),
        "timeline summary: {:?}",
        node.summary
    );
}

#[tokio::test]
async fn execute_review_request_reentered_at_push_started_without_origin_converges() {
    let root = tempdir().expect("root");
    let (store, prepared, worktree) = prepare_local_delivery_attempt(root.path(), None).await;
    let commit_sha =
        seed_push_started_journal(&store, &prepared, &worktree, "feat: local delivery");
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    let request = engine
        .execute_review_request(&prepared, "origin", "feat: local delivery")
        .await
        .expect("PushStarted re-entry without origin must converge");

    assert_eq!(request.push_status, PushStatus::SkippedNotConfigured);
    assert_eq!(request.commit_sha, commit_sha);
    let journal = store
        .get_coding_git_operation(&prepared)
        .expect("journal read")
        .expect("journal persisted");
    assert_eq!(journal.phase, CodingGitOperationPhase::Completed);
    assert_eq!(journal.push_status, Some(PushStatus::SkippedNotConfigured));
}

#[tokio::test]
async fn execute_review_request_with_unreachable_configured_origin_stays_indeterminate() {
    let root = tempdir().expect("root");
    let missing_origin = root.path().join("missing-origin.git");
    let (store, prepared, worktree) =
        prepare_local_delivery_attempt(root.path(), Some(&missing_origin)).await;
    fs::write(worktree.join("feature.txt"), "unreachable\n").expect("feature change");
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    // 首次进入：origin 已配置但不可达，不得伪装成本地交付。
    let first = engine
        .execute_review_request(&prepared, "origin", "feat: unreachable")
        .await;
    assert!(
        first.is_err(),
        "configured-but-unreachable origin must not be skipped"
    );
    let journal = store
        .get_coding_git_operation(&prepared)
        .expect("journal read")
        .expect("journal persisted");
    assert_eq!(journal.phase, CodingGitOperationPhase::PushStarted);
    assert_eq!(journal.push_status, None);

    // PushStarted 重入：仍为 Err，journal 不被推进。
    let second = engine
        .execute_review_request(&prepared, "origin", "feat: unreachable")
        .await;
    assert!(second.is_err(), "re-entry must keep failing closed");
    let journal = store
        .get_coding_git_operation(&prepared)
        .expect("journal read")
        .expect("journal persisted");
    assert_eq!(journal.phase, CodingGitOperationPhase::PushStarted);
}
