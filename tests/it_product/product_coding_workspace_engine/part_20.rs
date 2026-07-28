// 组完成写入范围门禁：数据源必须是 git 事实，不依赖交接摘要字段。
//
// 对应 change `remove-work-item-handoff` 工作包 1.8、1.9。
// 这两个测试替代 part_13.rs 的 group_final_confirm_rejects_unit_handoff_outside_exclusive_scope
// —— 原测试靠覆写 WorkItemHandoff.files_changed 触发违规，模型移除后无法构造。

/// 在共享 worktree 中提交指定文件，返回 commit sha。
fn commit_unit_change(worktree: &Path, relative_path: &str, contents: &str) -> String {
    let absolute = worktree.join(relative_path);
    if let Some(parent) = absolute.parent() {
        fs::create_dir_all(parent).expect("create parent dir for unit change");
    }
    fs::write(&absolute, contents).expect("write unit change");
    run_git(worktree, &["add", "--all"]);
    run_git(worktree, &["commit", "-m", &format!("change {relative_path}")]);
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(worktree)
        .output()
        .expect("git rev-parse HEAD");
    assert!(output.status.success(), "git rev-parse HEAD failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// 构造一个等待最终确认的 group attempt：两个已完成 unit，各自有真实 completion commit。
///
/// `unit2_relative_path` 决定第二个 unit 实际改了哪个文件，用于分别覆盖越界与合规两种情形。
/// work_item_0002 的 exclusive_write_scopes 固定为 `src/frontend.rs`。
fn group_attempt_with_committed_unit_changes(
    unit2_relative_path: &str,
) -> (
    tempfile::TempDir,
    ProductAppPaths,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
) {
    let (root, paths, store, engine, attempt) = group_engine_with_last_running_unit();
    let lifecycle = LifecycleStore::new(paths.clone());
    let worktree = attempt
        .worktree_path
        .clone()
        .expect("group attempt worktree path");

    for (work_item_id, scope) in [
        ("work_item_0001", "src/backend.rs"),
        ("work_item_0002", "src/frontend.rs"),
    ] {
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                title: format!("title for {work_item_id}"),
                exclusive_write_scopes: vec![scope.to_string()],
                forbidden_write_scopes: Vec::new(),
                ..Default::default()
            })
            .expect("create scoped work item");
        lifecycle
            .update_work_item_execution_status(
                "project_0001",
                "issue_0001",
                work_item_id,
                WorkItemStatus::Coding,
            )
            .expect("set coding status");
    }

    // 每个 unit 的实际变更各自成为一个 commit，completion_commit 指向它。
    let unit1_commit = commit_unit_change(&worktree, "src/backend.rs", "// backend changed\n");
    store
        .update_coding_unit_completion_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0001",
            Some(unit1_commit),
        )
        .expect("set unit1 completion commit");

    let unit2_commit = commit_unit_change(&worktree, unit2_relative_path, "// unit2 changed\n");
    let unit2_commit_for_head = unit2_commit.clone();
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
            CodingExecutionUnitStatus::Completed,
            Some("unit2 done".to_string()),
        )
        .expect("complete last unit");
    store
        .update_coding_unit_completion_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
            Some(unit2_commit),
        )
        .expect("set unit2 completion commit");

    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("set running");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::FinalConfirm,
        )
        .expect("final confirm stage");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::WaitingForHuman,
        )
        .expect("waiting for human");
    let attempt = store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some(unit2_commit_for_head),
        )
        .expect("set head commit");

    (root, paths, store, engine, attempt)
}

#[tokio::test]
async fn group_completion_gate_rejects_changed_files_outside_exclusive_scope_from_git() {
    // unit2 的 exclusive scope 是 src/frontend.rs，但实际提交改了 web/src/app.tsx。
    let (_root, _paths, _store, engine, attempt) =
        group_attempt_with_committed_unit_changes("web/src/app.tsx");

    let error = engine
        .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("group final confirm should reject out-of-scope write");

    match error {
        cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngineError::WorkItemDiffScopeViolation(path) => {
            assert_eq!(
                path, "web/src/app.tsx",
                "越界文件必须来自 git 事实，而非交接摘要字段"
            );
        }
        other => panic!("expected diff scope violation, got {other:?}"),
    }
}

#[tokio::test]
async fn group_completion_gate_allows_changed_files_within_exclusive_scope_from_git() {
    // unit2 实际提交只改了自己的 exclusive scope，门禁必须放行，不得因改数据源而误拒。
    let (_root, _paths, _store, engine, attempt) =
        group_attempt_with_committed_unit_changes("src/frontend.rs");

    let report = engine
        .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await;

    assert!(
        report.is_ok(),
        "合规写入不得被写入范围门禁拒绝，实际: {:?}",
        report.err()
    );
}
