// Task 1-3 端到端测试：harden-coding-attempt-deletion（DELETE coding-attempts 清理健壮化）。
//
// 覆盖 spec 四组 requirement：
//   - worktree 缺失不阻断（Task 1）
//   - 清理该 attempt 残留 lock + 不误删其他 attempt work_item lock（Task 2）
//   - shared-worktree 条件清理（无其他 attempt 删 / 有保留）（Task 3）
//   - 不得误伤同 issue 其他数据（由 Task 2/3 的精确性 + 条件覆盖）

const HARDEN_PROJECT_ID: &str = "project_0001";
const HARDEN_ISSUE_ID: &str = "issue_0001";

fn harden_issue_root(app_paths: &ProductAppPaths) -> PathBuf {
    app_paths.issue_root(HARDEN_PROJECT_ID, HARDEN_ISSUE_ID)
}

/// 断言该 attempt 的残留 lock 全部被清理。
fn assert_attempt_lock_residue_purged(
    app_paths: &ProductAppPaths,
    attempt_id: &str,
    expect_group_arbitration: bool,
    work_item_ids: &[&str],
) {
    let coding_root = harden_issue_root(app_paths).join("coding-attempts");
    assert!(
        !coding_root.join(format!(".{attempt_id}.json.lock")).exists(),
        ".coding_attempt_<id>.json.lock must be purged for {attempt_id}"
    );
    if expect_group_arbitration {
        assert!(
            !coding_root
                .join(".group-initialization-arbitration.lock")
                .exists(),
            ".group-initialization-arbitration.lock must be purged for group attempt"
        );
    }
    let locks_dir = coding_root.join("work-item-attempt-locks");
    for work_item_id in work_item_ids {
        assert!(
            !locks_dir.join(work_item_id).exists(),
            "work item attempt lock for {work_item_id} must be purged"
        );
        assert!(
            !locks_dir
                .join(format!(".{work_item_id}.lock"))
                .exists(),
            "work item attempt lock sidecar for {work_item_id} must be purged"
        );
    }
}

#[tokio::test]
async fn delete_coding_attempt_succeeds_when_worktree_dir_removed() {
    // Spec: worktree 缺失不得阻断 coding attempt 删除。
    // 真实场景：用户/运维已手动删 worktree 目录，DELETE 仍必须返回 204。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let (status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&attempt);

    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = prepare_attempt_with_worktree(
        &store,
        repo.path(),
        "project_0001",
        "issue_0001",
        &attempt_id,
    );
    let worktree_path = attempt.worktree_path.clone().expect("worktree path");
    // 模拟运维/外部进程提前删了 worktree 目录。
    fs::remove_dir_all(&worktree_path).expect("pre-remove worktree dir");

    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "DELETE must succeed when worktree dir already removed: {body}"
    );
    assert!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt_id)
            .is_err(),
        "attempt record must be deleted"
    );
}

#[tokio::test]
async fn delete_coding_attempt_purges_single_attempt_lock_residue() {
    // Spec: 删除 attempt 必须清理该 attempt 的残留 lock（single scope）。
    // single scope 不持有 .group-initialization-arbitration.lock，但持自身 .json.lock
    // 与 work-item-attempt-locks/<work_item_id>（按 attempt.work_item_id）。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let (status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&attempt);

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_root = harden_issue_root(&app_paths).join("coding-attempts");
    // 模拟 attempt 创建后残留的 lock（运行时 lock + work item attempt lock）。
    fs::write(
        coding_root.join(format!(".{attempt_id}.json.lock")),
        "",
    )
    .expect("seed single attempt json lock");
    let locks_dir = coding_root.join("work-item-attempt-locks");
    fs::create_dir_all(&locks_dir).expect("attempt locks dir");
    fs::write(locks_dir.join("work_item_0001"), "{}").expect("seed single attempt work item lock");
    fs::write(locks_dir.join(".work_item_0001.lock"), "").expect("seed single attempt sidecar");

    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete body: {body}");

    assert_attempt_lock_residue_purged(&app_paths, &attempt_id, false, &["work_item_0001"]);
}

#[tokio::test]
async fn delete_coding_attempt_purges_group_attempt_lock_residue() {
    // Spec: 删除 group attempt 必须清理 .coding_attempt_<id>.json.lock、
    // .group-initialization-arbitration.lock、各 unit 的 work-item-attempt-locks/<wi>.lock。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let (status, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "group attempt create: {created}");
    let attempt_id = assert_global_attempt_id(&created);

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_root = harden_issue_root(&app_paths).join("coding-attempts");
    // 播种 group 残留 lock：自身 json.lock、group 仲裁 lock、各 unit work_item lock。
    fs::write(
        coding_root.join(format!(".{attempt_id}.json.lock")),
        "",
    )
    .expect("seed group attempt json lock");
    fs::write(
        coding_root.join(".group-initialization-arbitration.lock"),
        "",
    )
    .expect("seed group arbitration lock");
    let locks_dir = coding_root.join("work-item-attempt-locks");
    fs::create_dir_all(&locks_dir).expect("attempt locks dir");
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        fs::write(locks_dir.join(work_item_id), "{}").expect("seed group unit lock");
        fs::write(locks_dir.join(format!(".{work_item_id}.lock")), "")
            .expect("seed group unit sidecar");
    }

    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete body: {body}");

    // group attempt 的 work_item 来自 plan 各 unit 的 logical_work_item_id。
    assert_attempt_lock_residue_purged(
        &app_paths,
        &attempt_id,
        true,
        &["work_item_0001", "work_item_0002"],
    );
}

#[tokio::test]
async fn delete_coding_attempt_preserves_other_attempt_work_item_lock() {
    // Spec: 删除本 attempt 不得误删其他 attempt work_item 的 lock（按 work_item 精确删）。
    // 共享的 issue 级 work-item-attempt-locks 目录里混入「另一 attempt 的 work_item」锁。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let (status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&attempt);

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_root = harden_issue_root(&app_paths).join("coding-attempts");
    let locks_dir = coding_root.join("work-item-attempt-locks");
    fs::create_dir_all(&locks_dir).expect("attempt locks dir");
    // 本 attempt 的 work_item lock（删除时精确清理）。
    fs::write(locks_dir.join("work_item_0001"), "{}").expect("seed own lock");
    fs::write(locks_dir.join(".work_item_0001.lock"), "").expect("seed own sidecar");
    // 另一 attempt 的 work_item lock（必须保留——spec 不误删）。
    fs::write(locks_dir.join("work_item_other"), "{}").expect("seed other attempt lock");
    fs::write(locks_dir.join(".work_item_other.lock"), "").expect("seed other attempt sidecar");

    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete body: {body}");

    assert!(
        !locks_dir.join("work_item_0001").exists(),
        "own work item lock must be purged"
    );
    assert!(
        !locks_dir.join(".work_item_0001.lock").exists(),
        "own work item lock sidecar must be purged"
    );
    assert!(
        locks_dir.join("work_item_other").exists(),
        "other attempt work item lock must be preserved (spec: 不误伤)"
    );
    assert!(
        locks_dir.join(".work_item_other.lock").exists(),
        "other attempt work item lock sidecar must be preserved"
    );
}

#[tokio::test]
async fn delete_coding_attempt_cleans_shared_worktree_when_no_other_attempts() {
    // Spec: 删除 attempt 后该 issue 无其他 attempt → shared-worktree.json + .lock 必须删除。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let (status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&attempt);

    // attempt 创建时会 upsert 仓维 shared-worktrees/{repository}.json（REQ-COD-03
    // 三元键；迁移后单件 attempt 冻结快照走仓维路由）。经文件路径断言便于
    // 同步钉 json 与 lock 两个文件的清理。
    let attempt_record = CodingAttemptStore::new(app_paths.clone())
        .get_attempt(HARDEN_PROJECT_ID, HARDEN_ISSUE_ID, &attempt_id)
        .expect("load attempt for repo route");
    let logical_id = attempt_record
        .target_snapshot
        .as_ref()
        .expect("repo-scoped attempt snapshot")
        .logical_repository_id;
    let repo_dir = harden_issue_root(&app_paths).join("shared-worktrees");
    let shared_json = repo_dir.join(format!("{}.json", logical_id.0));
    let shared_lock = repo_dir.join(format!(".{}.json.lock", logical_id.0));
    assert!(shared_json.exists(), "precondition: repo shared-worktree seeded");

    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete body: {body}");

    assert!(
        !shared_json.exists(),
        "repo shared-worktrees/{}.json must be deleted when no other active attempts remain",
        logical_id.0
    );
    assert!(
        !shared_lock.exists(),
        "repo shared-worktrees/.{}.json.lock must be deleted when no other active attempts remain",
        logical_id.0
    );
}

#[tokio::test]
async fn delete_coding_attempt_preserves_shared_worktree_when_other_attempts_exist() {
    // Spec: 删除 attempt 后该 issue 仍有其他 attempt 记录 → shared-worktree.json 必须保留。
    // 构造：删除当前 attempt 前，预先在该 issue 的 coding-attempts 目录里播种另一条 attempt json
    // （非 active，但 list_attempts_for_issue 仍返回）。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let (status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&attempt);

    // 仓维路由（REQ-COD-03）：删除后「同 logical repository 无其他【活动】
    // attempt 引用时才删 shared-worktrees/{id}.json」。经文件路径断言 json 与
    // lock 两个文件的保留。
    let attempt_record = CodingAttemptStore::new(app_paths.clone())
        .get_attempt(HARDEN_PROJECT_ID, HARDEN_ISSUE_ID, &attempt_id)
        .expect("load attempt for repo route");
    let logical_id = attempt_record
        .target_snapshot
        .as_ref()
        .expect("repo-scoped attempt snapshot")
        .logical_repository_id;
    let repo_dir = harden_issue_root(&app_paths).join("shared-worktrees");
    let shared_json = repo_dir.join(format!("{}.json", logical_id.0));
    let shared_lock = repo_dir.join(format!(".{}.json.lock", logical_id.0));
    assert!(shared_json.exists(), "precondition: repo shared-worktree seeded");

    // 在 coding-attempts 目录里伪造另一条同仓【活动】attempt json（基于当前
    // attempt 改写 id/work_item_id，保持 created 活动态与同一冻结快照）。
    // 删除当前 attempt 后，同仓仍有活动 attempt 引用 → 仓维 shared-worktree 保留
    // （REQ-COD-03 §4.2.4：同仓其他 item 可能复用，不能一个 attempt 删就清）。
    let coding_root = harden_issue_root(&app_paths).join("coding-attempts");
    let current_json_path = coding_root.join(format!("{attempt_id}.json"));
    let mut other_record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&current_json_path).expect("read attempt json"))
            .expect("parse attempt json");
    other_record["id"] = json!("coding_attempt_otherremainingattempt00000");
    other_record["work_item_id"] = json!("work_item_other");
    other_record["status"] = json!("created");
    let other_json_path = coding_root.join("coding_attempt_otherremainingattempt00000.json");
    fs::write(&other_json_path, other_record.to_string())
        .expect("seed other remaining attempt json");

    let (status, body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete body: {body}");

    assert!(
        shared_json.exists(),
        "repo shared-worktrees/{}.json must be preserved while another active same-repo attempt remains",
        logical_id.0
    );
    assert!(
        shared_lock.exists(),
        "repo shared-worktrees/.{}.json.lock must be preserved while another active same-repo attempt remains",
        logical_id.0
    );
    assert!(
        other_json_path.exists(),
        "other attempt json must be untouched (spec: 不误伤)"
    );
}
