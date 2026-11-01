// ============================================================================
// per-issue-base-branch REQ-PIB-03 场景 1（T3.1 红绿用例，k3 终审 F1 修复配套）：
// coding fork 入口必须读 issue.base_branch（三面同源解析链
// resolve_effective_base_branch），不得回退当前检出或 HEAD。覆盖：
//   - 单件入口（create_coding_attempt）与组入口（create_group_coding_attempt）：
//     issue.base_branch=feature/x、仓库当前检出 main 时，attempt.base_branch
//     锁定 feature/x，共享 worktree 登记同基线，且生产 fork 原语
//     （create_branch(branch, base)）从 feature/x tip 分叉而非 main tip；
//   - 基线分支被删（fail-closed）：启动 422 issue_base_branch_not_found，
//     不落 attempt、不写 group journal、不登记共享 worktree。
//
// 夹具：issue 经 REST 创建后由 set_issue_base_branch 把 base_branch 锁定为
// feature/x（等价于按 REQ-PIB-01 场景 1 显式选择基准分支创建的 issue 记录；
// 创建校验面已在 web_product_api 族钉死，此处聚焦 fork 消费面）。
// ============================================================================

fn git_repo_with_feature_branch() -> tempfile::TempDir {
    let dir = tempdir().expect("repo");
    run_git(dir.path(), &["init", "-b", "main"]);
    run_git(dir.path(), &["config", "user.email", "aria@example.com"]);
    run_git(dir.path(), &["config", "user.name", "Aria Test"]);
    fs::write(dir.path().join("README.md"), "# repo\n").expect("seed readme");
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "initial"]);
    run_git(dir.path(), &["checkout", "-b", "feature/x"]);
    fs::write(dir.path().join("feature-marker.txt"), "feature only\n").expect("feature marker");
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "feature work"]);
    // 当前检出回到 main：旧实现（current_git_branch→HEAD）会从 main 分叉，
    // 与 issue 锁定的 feature/x 基线错位——本组用例的红绿分界点。
    run_git(dir.path(), &["checkout", "main"]);
    dir
}

fn set_issue_base_branch(app_paths: &ProductAppPaths, base_branch: &str) {
    let issue_path = app_paths
        .issue_root("project_0001", "issue_0001")
        .join("issue.json");
    let mut issue: Value = serde_json::from_slice(&fs::read(&issue_path).expect("read issue json"))
        .expect("parse issue json");
    issue["base_branch"] = json!(base_branch);
    fs::write(
        &issue_path,
        serde_json::to_vec_pretty(&issue).expect("serialize issue json"),
    )
    .expect("write issue json");
}

fn rev_of(repo_path: &std::path::Path, reference: &str) -> String {
    let output = Command::new("git")
        .args(["rev-parse", reference])
        .current_dir(repo_path)
        .output()
        .expect("git rev-parse");
    assert!(
        output.status.success(),
        "git rev-parse {reference}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[tokio::test]
async fn coding_attempt_forks_from_issue_base_branch_not_current_checkout() {
    let root = tempdir().expect("root");
    let repo = git_repo_with_feature_branch();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    set_issue_base_branch(&app_paths, "feature/x");

    let (status, attempt) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "response: {attempt}");
    assert_eq!(attempt["branch_name"], "aria/issues/issue_0001");
    assert_eq!(attempt["base_branch"], "feature/x");

    // 共享 worktree 登记与 attempt 同基线（fork 契约两面同源）。
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("read shared worktree")
        .expect("shared worktree registered");
    assert_eq!(shared.base_branch, "feature/x");

    // 生产 fork 原语（engine execute_worktree_prepare 同款 create_branch）按
    // attempt.base_branch 分叉：当前检出是 main，分叉点必须等于 feature/x tip。
    let coding_store = CodingAttemptStore::new(app_paths);
    let stored = coding_store
        .get_attempt(
            "project_0001",
            "issue_0001",
            &assert_global_attempt_id(&attempt),
        )
        .expect("stored attempt");
    assert_eq!(stored.base_branch, "feature/x");
    GitWorkspaceService::new()
        .create_branch(repo.path(), &stored.branch_name, &stored.base_branch)
        .await
        .expect("fork branch from attempt base branch");
    let forked = rev_of(repo.path(), &format!("refs/heads/{}", stored.branch_name));
    let feature_tip = rev_of(repo.path(), "refs/heads/feature/x");
    let main_tip = rev_of(repo.path(), "refs/heads/main");
    assert_eq!(forked, feature_tip, "fork point must be the feature/x tip");
    assert_ne!(
        forked, main_tip,
        "fork point must not follow the current checkout (main)"
    );
}

#[tokio::test]
async fn group_coding_attempt_forks_from_issue_base_branch_not_current_checkout() {
    let root = tempdir().expect("root");
    let repo = git_repo_with_feature_branch();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    set_issue_base_branch(&app_paths, "feature/x");
    fs::remove_dir_all(
        app_paths
            .issue_root("project_0001", "issue_0001")
            .join("work-items"),
    )
    .expect("remove legacy work items");

    let (status, attempt) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "response: {attempt}");
    assert_eq!(attempt["branch_name"], "aria/issues/issue_0001");
    assert_eq!(attempt["base_branch"], "feature/x");

    // 组入口共享 worktree 登记同基线。
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("read shared worktree")
        .expect("shared worktree registered");
    assert_eq!(shared.base_branch, "feature/x");
}

#[tokio::test]
async fn coding_attempt_fails_closed_with_422_when_issue_base_branch_deleted() {
    let root = tempdir().expect("root");
    let repo = git_repo_with_feature_branch();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    set_issue_base_branch(&app_paths, "feature/x");
    run_git(repo.path(), &["branch", "-D", "feature/x"]);

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "response: {body}"
    );
    assert_eq!(body["code"], "issue_base_branch_not_found");
    assert!(
        body["message"]
            .as_str()
            .expect("diagnosis message")
            .contains("feature/x"),
        "diagnosis must name the missing branch: {body}"
    );

    // fail-closed：基线解析先于任何持久化——不落 attempt、不登记共享 worktree。
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    assert!(coding_store
        .list_attempts_for_work_item("project_0001", "issue_0001", "work_item_0001")
        .expect("list attempts after failed create")
        .is_empty());
    let lifecycle = LifecycleStore::new(app_paths);
    assert_eq!(
        lifecycle
            .get_issue_shared_worktree("project_0001", "issue_0001")
            .expect("read shared worktree"),
        None
    );
}

#[tokio::test]
async fn group_coding_attempt_fails_closed_with_422_when_issue_base_branch_deleted() {
    let root = tempdir().expect("root");
    let repo = git_repo_with_feature_branch();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    set_issue_base_branch(&app_paths, "feature/x");
    fs::remove_dir_all(
        app_paths
            .issue_root("project_0001", "issue_0001")
            .join("work-items"),
    )
    .expect("remove legacy work items");
    run_git(repo.path(), &["branch", "-D", "feature/x"]);

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "response: {body}"
    );
    assert_eq!(body["code"], "issue_base_branch_not_found");

    // fail-closed：不写 group journal、不登记共享 worktree。
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    assert!(matches!(
        coding_store.get_group_initialization("project_0001", "issue_0001", "work_item_plan_0001"),
        Err(cadence_aria::product::json_store::ProductStoreError::NotFound { .. })
    ));
    let lifecycle = LifecycleStore::new(app_paths);
    assert_eq!(
        lifecycle
            .get_issue_shared_worktree("project_0001", "issue_0001")
            .expect("read shared worktree"),
        None
    );
}
