// runner 兜底 commit 修复（provider 无关化）的守卫测试：
// - sc_advance 建组预置 head_commit=base（lifecycle.rs）不得屏蔽 internal_pr_review.rs
//   commit 决策里的 runner 兜底 commit：coder 留下 staged/未 commit 改动时
//   （issue_0141/0171 现场形态），由 runner 提交，commit_sha=新 head 而非 base。
// - 消费面守卫：CompletedRetry/recover_completed_group_unit_commit 的比对、
//   group final readiness 的 empty_observation 记录。
// - 回归锚：legacy 通道（head_commit=None）行为零变化；预置 base 且无 staged
//   改动时的空观察语义不变（沿用当前 head，不 Blocked、不造空 commit）。

use crate::product::coding_attempt_store::CodingGitOperationPhase;

fn runner_fallback_commit_fixture() -> (GroupCompletionFixture, std::path::PathBuf) {
    let fixture = group_completion_fixture(true, false);
    let remote = fixture._root.path().join("remote.git");
    fs::create_dir_all(&remote).expect("remote dir");
    run_test_git(&remote, &["init", "--bare"]);
    run_test_git(
        &fixture.worktree,
        &["remote", "add", "origin", &remote.to_string_lossy()],
    );
    run_test_git(
        &fixture.worktree,
        &["checkout", "-b", "aria/issues/issue_0001"],
    );
    (fixture, remote)
}

/// 复现 sc_advance 建组短路（lifecycle.rs）与半启动恢复（prepare_resumed_）
/// 的同一动作：把 attempt.head_commit 预置为 worktree 当前 head（base）。
fn preset_head_commit_like_sc_advance(fixture: &GroupCompletionFixture) -> CodingExecutionAttempt {
    fixture
        .store
        .update_attempt_head_commit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            Some(fixture.original_head.clone()),
        )
        .expect("preset head_commit=base like sc_advance")
}

fn worktree_head(worktree: &Path) -> String {
    git_stdout(worktree, &["rev-parse", "HEAD"]).trim().to_string()
}

#[tokio::test]
async fn sc_advance_preset_head_commit_no_longer_masks_runner_fallback_commit() {
    let (fixture, remote) = runner_fallback_commit_fixture();
    let preset = preset_head_commit_like_sc_advance(&fixture);
    // coder 只留下 staged/未 commit 的工作树改动（issue_0141/0171 现场形态）。
    fs::write(fixture.worktree.join("unit1.txt"), "unit 1 change\n")
        .expect("coder change without commit");

    let request = fixture
        .engine
        .execute_review_request(&preset, "origin", "feat: implement work item")
        .await
        .expect("runner fallback commit review request");

    let committed_head = worktree_head(&fixture.worktree);
    assert_ne!(
        committed_head, fixture.original_head,
        "兜底 commit 必须产生新 commit，而不是把预置 base 记为 commit_sha"
    );
    assert_eq!(request.commit_sha, committed_head);
    assert_eq!(
        request.push_status, PushStatus::Pushed,
        "兜底 commit 推送的是新 head"
    );
    let remote_ref = format!("refs/heads/{}", preset.branch_name);
    assert_eq!(
        git_stdout(&remote, &["rev-parse", &remote_ref]).trim(),
        committed_head
    );
    assert_eq!(
        git_stdout(&fixture.worktree, &["status", "--porcelain"]).trim(),
        "",
        "兜底 commit 后共享工作树必须干净（dirty 门自然通过）"
    );
    let journal = fixture
        .store
        .get_coding_git_operation(&preset)
        .expect("journal lookup")
        .expect("review journal");
    assert_eq!(journal.phase, CodingGitOperationPhase::Completed);
    assert_eq!(
        journal.commit_sha.as_deref(),
        Some(committed_head.as_str()),
        "journal 不得再记录 base 为 commit_sha"
    );
    let attempt_after = fixture
        .store
        .get_attempt(&preset.project_id, &preset.issue_id, &preset.id)
        .expect("attempt after review request");
    assert_eq!(
        attempt_after.head_commit.as_deref(),
        Some(committed_head.as_str())
    );
}

#[tokio::test]
async fn runner_fallback_commit_completed_retry_recovery_matches_new_head() {
    let (fixture, _remote) = runner_fallback_commit_fixture();
    let run = create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    let preset = preset_head_commit_like_sc_advance(&fixture);
    fs::write(fixture.worktree.join("unit1.txt"), "unit 1 change\n")
        .expect("coder change without commit");
    let request = fixture
        .engine
        .execute_review_request(&preset, "origin", "feat: implement work item")
        .await
        .expect("runner fallback commit review request");
    let committed_head = worktree_head(&fixture.worktree);
    assert_ne!(committed_head, fixture.original_head);

    // 模拟崩溃：run 已完成落盘（completion_commit=兜底 commit），unit 状态与
    // 推进未写——重入 complete_group_unit_after_code_review 走 CompletedRetry。
    fixture
        .store
        .complete_coding_unit_run(&preset, &run.id, &request.commit_sha)
        .expect("seed completed run");

    let attempt = fixture
        .store
        .get_attempt(&preset.project_id, &preset.issue_id, &preset.id)
        .expect("attempt before completed retry");
    let updated = fixture
        .engine
        .complete_group_unit_after_code_review(&attempt)
        .await
        .expect("CompletedRetry 恢复不得报 group_completion_commit_mismatch");

    let units = fixture
        .store
        .list_coding_units(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("units");
    let unit1 = units
        .iter()
        .find(|unit| unit.id == "coding_unit_0001")
        .expect("unit 1");
    assert_eq!(unit1.status, CodingExecutionUnitStatus::Completed);
    assert_eq!(
        unit1.completion_commit.as_deref(),
        Some(request.commit_sha.as_str())
    );
    let completed_run = fixture
        .store
        .list_coding_unit_runs(&updated, "coding_unit_0001")
        .expect("unit runs")
        .into_iter()
        .find(|candidate| candidate.id == run.id)
        .expect("completed run");
    assert_eq!(
        completed_run.completion_commit.as_deref(),
        Some(request.commit_sha.as_str())
    );
    assert_eq!(
        updated.head_commit.as_deref(),
        Some(request.commit_sha.as_str())
    );
}

#[tokio::test]
async fn runner_fallback_commit_readiness_records_non_empty_observation() {
    let (fixture, _remote) = runner_fallback_commit_fixture();
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    let preset = preset_head_commit_like_sc_advance(&fixture);
    fs::write(fixture.worktree.join("unit1.txt"), "unit 1 change\n")
        .expect("coder change without commit");
    let request = fixture
        .engine
        .execute_review_request(&preset, "origin", "feat: implement work item")
        .await
        .expect("runner fallback commit review request");
    let committed_head = worktree_head(&fixture.worktree);
    assert_ne!(committed_head, fixture.original_head);

    let updated = fixture
        .engine
        .complete_group_unit_after_code_review(&preset)
        .await
        .expect("running mode group completion");

    let snapshot = fixture
        .engine
        .build_group_final_readiness_snapshot(&updated)
        .await
        .expect("readiness snapshot");
    let unit1 = snapshot
        .units
        .iter()
        .find(|unit| unit.unit_id == "coding_unit_0001")
        .expect("unit 1 readiness");
    assert!(
        !unit1.empty_observation,
        "兜底 commit 交付了真实 commit，不得记为空观察"
    );
    assert_eq!(unit1.commit_shas, vec![request.commit_sha.clone()]);
    assert_eq!(
        unit1.diff_ref,
        format!("{}..{}", fixture.original_head, request.commit_sha)
    );
}

#[tokio::test]
async fn runner_fallback_commit_legacy_head_commit_none_channel_is_unchanged() {
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
    let base_head = worktree_head(&repo);
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
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let started = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");
    let _stage = rx.recv().await.expect("stage event");
    let _node = rx.recv().await.expect("node event");
    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("prepare worktree");
    let _worktree_complete = rx.recv().await.expect("worktree complete");
    let worktree = prepared.worktree_path.as_ref().expect("worktree path");
    fs::write(worktree.join("feature.txt"), "legacy change\n").expect("feature change");

    let request = engine
        .execute_review_request(&prepared, "origin", "feat: implement work item")
        .await
        .expect("legacy fallback commit");

    assert_ne!(request.commit_sha, base_head);
    assert_eq!(request.commit_sha, worktree_head(worktree));
    assert_eq!(
        git_stdout(worktree, &["status", "--porcelain"]).trim(),
        "",
        "legacy 通道兜底 commit 后工作树干净"
    );
}

#[tokio::test]
async fn runner_fallback_commit_preset_base_without_staged_changes_keeps_empty_observation() {
    let (fixture, _remote) = runner_fallback_commit_fixture();
    let preset = preset_head_commit_like_sc_advance(&fixture);
    // coder 未留下任何可提交改动（真实空观察）。

    let request = fixture
        .engine
        .execute_review_request(&preset, "origin", "feat: implement work item")
        .await
        .expect("empty observation review request");

    assert_eq!(
        request.commit_sha, fixture.original_head,
        "无 staged 且无人 commit：沿用当前 head（空观察），不 Blocked 也不造空 commit"
    );
    assert_eq!(worktree_head(&fixture.worktree), fixture.original_head);
    let attempt_after = fixture
        .store
        .get_attempt(&preset.project_id, &preset.issue_id, &preset.id)
        .expect("attempt after empty observation");
    assert_eq!(attempt_after.status, CodingAttemptStatus::Running);
}
