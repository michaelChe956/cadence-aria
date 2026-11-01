// Task 18（REQ-COD-03 并发验收）：在 web handler 层证明多仓仓维锁的
// 「异仓并行、同仓串行、锁不泄漏」三项设计承诺。
//
// 同步方式：沿用 part_09 的 channel（test_controls Notify）同步两个启动请求——
// 先让第一个请求在「已 acquire 仓维 lease、尚未创建 attempt」处暂停，再放第二个
// 请求竞争，最后 resume 第一个请求。这比裸 barrier 更确定，可精确断言
// 「异仓不阻塞」与「同仓串行」两个方向。
//
// 仓维锁键为 (project, issue, logical_repository_id)：
//   - 异仓（不同 logical_repository_id）：两个 WorkItem 各自 acquire，互不阻塞。
//   - 同仓（相同 logical_repository_id）：第二个 acquire 返回稳定码 repo_worktree_active → 409。
//   - 失败方在成功方 release（abort 获胜 attempt → 仓维锁按 owner 释放）后可重新 acquire。

const WORK_ITEM_ONE_CODING_URI: &str =
    "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts";
const WORK_ITEM_TWO_CODING_URI: &str =
    "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts";

struct MultiRepoWorkItemFixture {
    logical_primary: LogicalRepositoryId,
    logical_target: LogicalRepositoryId,
}

/// 构造「一个 issue 绑定 primary 仓 + 两个已确认 WorkItem + 逻辑代码库迁移」的多仓夹具。
///
/// `item_one_physical_repo` / `item_two_physical_repo` 指定两个 WorkItem 分别落在哪个
/// 物理仓（"repository_0001" 或 "repository_0002"），迁移后经 backfill 得到各自的
/// `target_repository_id`。issue 的 codebase selection 显式包含两个逻辑成员，
/// 使多仓 attempt 的删除/路由均通过 fail-closed 校验（语义等价于多仓 issue）。
async fn bootstrap_multi_repo_two_work_items(
    app: axum::Router,
    root: &std::path::Path,
    primary_repo: &std::path::Path,
    target_repo: &std::path::Path,
    item_one_physical_repo: &str,
    item_two_physical_repo: &str,
) -> MultiRepoWorkItemFixture {
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name": "Coding", "description": null}),
    )
    .await;
    let primary = register_repository_and_wait(
        app.clone(),
        json!({"name": "Primary", "path": primary_repo, "default_provider_mode": "fake"}),
    )
    .await;
    assert_eq!(primary["repository_id"], "repository_0001");
    let target = register_repository_and_wait(
        app.clone(),
        json!({"name": "Target", "path": target_repo, "default_provider_mode": "fake"}),
    )
    .await;
    assert_eq!(target["repository_id"], "repository_0002");
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title": "多仓并发验收",
            "description": "REQ-COD-03 并发验收",
            "repository_id": "repository_0001"
        }),
    )
    .await;

    let app_paths = ProductAppPaths::new(root.join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    for (work_item_id, repository_id, title) in [
        ("work_item_0001", item_one_physical_repo, "实现爬楼梯"),
        ("work_item_0002", item_two_physical_repo, "实现爬楼梯 part 2"),
    ] {
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repository_id.to_string(),
                title: title.to_string(),
                plan_status: WorkItemPlanStatus::Confirmed,
                ..Default::default()
            })
            .expect("create multi-repo work item");
    }

    IdentityMigrationExecutor::new(app_paths.clone())
        .ensure_identity_schema("project_0001")
        .expect("migrate fixture to logical codebase");

    let members = LogicalCodebaseStore::new(app_paths.clone())
        .list_members("project_0001")
        .expect("logical members");
    let logical_primary = members
        .iter()
        .find(|member| member.physical_repository_id == "repository_0001")
        .expect("primary logical member")
        .logical_repository_id;
    let logical_target = members
        .iter()
        .find(|member| member.physical_repository_id == "repository_0002")
        .expect("target logical member")
        .logical_repository_id;

    // 多仓 issue 的 codebase selection 显式包含两个逻辑成员，保证多仓 attempt 的
    // 删除/路由均通过 fail-closed 校验（与 seed_logical_fixture_multi_target 同构）。
    IssueCodebaseSelectionStore::new(app_paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![logical_primary, logical_target],
            Vec::new(),
            vec![logical_primary],
            None,
        ))
        .expect("issue codebase selection includes both members");

    MultiRepoWorkItemFixture {
        logical_primary,
        logical_target,
    }
}

/// ① 异仓并行：work_item_0001 已持有 primary 仓 lease（暂停中）时，work_item_0002
/// 仍能在 target 仓独立 acquire 并完成启动——不同 logical_repository_id 互不阻塞。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn distinct_target_work_items_start_coding_without_blocking_each_other() {
    let root = tempdir().expect("root");
    let primary_repo = git_repo();
    let target_repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let acquire_pause = state
        .test_controls
        .pause_next_coding_attempt_after_worktree_acquire();
    let app = build_web_router(state);
    let fixture = bootstrap_multi_repo_two_work_items(
        app.clone(),
        root.path(),
        primary_repo.path(),
        target_repo.path(),
        "repository_0001",
        "repository_0002",
    )
    .await;

    // 第一个请求：work_item_0001 → primary 仓，acquire 后暂停（持有 primary 仓 lease）。
    let first_app = app.clone();
    let mut first_request = tokio::spawn(async move {
        request_json(
            first_app,
            Method::POST,
            WORK_ITEM_ONE_CODING_URI,
            json!({}),
        )
        .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        acquire_pause.wait_until_paused(),
    )
    .await
    .expect("first request did not pause after repo worktree acquire");

    // 第二个请求：work_item_0002 → target 仓。在第一个仍持有 primary 仓 lease 时必须
    // 独立完成（异仓不阻塞）。
    let second_app = app.clone();
    let mut second_request = tokio::spawn(async move {
        request_json(
            second_app,
            Method::POST,
            WORK_ITEM_TWO_CODING_URI,
            json!({}),
        )
        .await
    });
    let (second_status, second_body) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        &mut second_request,
    )
    .await
    .expect("target repo request blocked while primary repo lease was held")
    .expect("target repo request task");
    assert_eq!(second_status, StatusCode::OK, "异仓启动必须成功: {second_body}");
    assert_global_attempt_id(&second_body);
    assert_eq!(second_body["work_item_id"], "work_item_0002");

    // 释放第一个请求，异仓同样成功。
    acquire_pause.resume();
    let (first_status, first_body) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        &mut first_request,
    )
    .await
    .expect("primary repo request did not finish after resume")
    .expect("primary repo request task");
    assert_eq!(first_status, StatusCode::OK, "异仓启动必须成功: {first_body}");
    assert_global_attempt_id(&first_body);
    assert_eq!(first_body["work_item_id"], "work_item_0001");

    // 各自仓维 lease：两个不同 logical_repository_id 均有自己的 active work item。
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let primary_shared = lifecycle
        .get_repo_shared_worktree("project_0001", "issue_0001", fixture.logical_primary)
        .expect("primary repo worktree read")
        .expect("primary repo worktree exists");
    assert_eq!(
        primary_shared.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );
    let target_shared = lifecycle
        .get_repo_shared_worktree("project_0001", "issue_0001", fixture.logical_target)
        .expect("target repo worktree read")
        .expect("target repo worktree exists");
    assert_eq!(
        target_shared.current_active_work_item_id.as_deref(),
        Some("work_item_0002")
    );
}

/// ② 同仓串行：同 logical_repository_id 的两个 WorkItem 并发启动，
/// 恰好一个成功、一个得到稳定码 repo_worktree_active（409 CONFLICT）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_target_work_items_start_coding_one_lease_one_repo_worktree_active() {
    let root = tempdir().expect("root");
    let primary_repo = git_repo();
    let target_repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let acquire_pause = state
        .test_controls
        .pause_next_coding_attempt_after_worktree_acquire();
    let app = build_web_router(state);
    let fixture = bootstrap_multi_repo_two_work_items(
        app.clone(),
        root.path(),
        primary_repo.path(),
        target_repo.path(),
        "repository_0002",
        "repository_0002",
    )
    .await;

    // 第一个请求 acquire 同仓 lease 后暂停。
    let first_app = app.clone();
    let mut first_request = tokio::spawn(async move {
        request_json(
            first_app,
            Method::POST,
            WORK_ITEM_ONE_CODING_URI,
            json!({}),
        )
        .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        acquire_pause.wait_until_paused(),
    )
    .await
    .expect("first request did not pause after repo worktree acquire");

    // 第二个请求竞争同仓 lease → 稳定码 repo_worktree_active。
    let second_app = app.clone();
    let mut second_request = tokio::spawn(async move {
        request_json(
            second_app,
            Method::POST,
            WORK_ITEM_TWO_CODING_URI,
            json!({}),
        )
        .await
    });
    let (second_status, second_body) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        &mut second_request,
    )
    .await
    .expect("same-target competitor did not finish")
    .expect("same-target competitor task");
    assert_eq!(
        second_status,
        StatusCode::CONFLICT,
        "同仓第二个启动必须 409: {second_body}"
    );
    assert_eq!(second_body["code"], "repo_worktree_active");

    // 释放第一个请求，获胜方成功完成启动。
    acquire_pause.resume();
    let (first_status, first_body) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        &mut first_request,
    )
    .await
    .expect("same-target winner did not finish after resume")
    .expect("same-target winner task");
    assert_eq!(first_status, StatusCode::OK, "同仓获胜方必须成功: {first_body}");
    let winner_attempt_id = assert_global_attempt_id(&first_body);
    let winner_work_item_id = first_body["work_item_id"]
        .as_str()
        .expect("winner work item id");

    // 获胜方持有的仓维 lease 与其 work_item 一致。
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let shared = lifecycle
        .get_repo_shared_worktree("project_0001", "issue_0001", fixture.logical_target)
        .expect("target repo worktree read")
        .expect("target repo worktree exists");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some(winner_work_item_id),
        "仓维 lease 必须由获胜方持有"
    );
    assert_eq!(
        shared.current_lock_owner_id.as_deref(),
        Some(winner_attempt_id.as_str()),
        "仓维 lease 必须绑定到获胜方 attempt"
    );
}

/// ③ 失败后锁释放：同仓竞争失败方（repo_worktree_active）在成功方 release
/// （abort 获胜 attempt → 仓维锁按 owner 释放）后，可重新 acquire 成功。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_target_loser_can_reacquire_after_winner_releases_lock() {
    let root = tempdir().expect("root");
    let primary_repo = git_repo();
    let target_repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let acquire_pause = state
        .test_controls
        .pause_next_coding_attempt_after_worktree_acquire();
    let app = build_web_router(state);
    let fixture = bootstrap_multi_repo_two_work_items(
        app.clone(),
        root.path(),
        primary_repo.path(),
        target_repo.path(),
        "repository_0002",
        "repository_0002",
    )
    .await;

    let first_app = app.clone();
    let mut first_request = tokio::spawn(async move {
        request_json(
            first_app,
            Method::POST,
            WORK_ITEM_ONE_CODING_URI,
            json!({}),
        )
        .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        acquire_pause.wait_until_paused(),
    )
    .await
    .expect("first request did not pause after repo worktree acquire");

    let second_app = app.clone();
    let mut second_request = tokio::spawn(async move {
        request_json(
            second_app,
            Method::POST,
            WORK_ITEM_TWO_CODING_URI,
            json!({}),
        )
        .await
    });
    let (second_status, second_body) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        &mut second_request,
    )
    .await
    .expect("same-target competitor did not finish")
    .expect("same-target competitor task");
    assert_eq!(second_status, StatusCode::CONFLICT, "{second_body}");
    assert_eq!(second_body["code"], "repo_worktree_active");

    acquire_pause.resume();
    let (first_status, first_body) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        &mut first_request,
    )
    .await
    .expect("same-target winner did not finish after resume")
    .expect("same-target winner task");
    assert_eq!(first_status, StatusCode::OK, "{first_body}");
    let winner_attempt_id = assert_global_attempt_id(&first_body);
    let winner_work_item_id = first_body["work_item_id"].as_str().expect("winner work item id");
    let loser_work_item_id = if winner_work_item_id == "work_item_0001" {
        "work_item_0002"
    } else {
        "work_item_0001"
    };

    // 成功方 release：abort 获胜 attempt，仓维锁由 engine 按 owner 释放。
    // （DELETE 路径会经 resolve_coding_attempt_repository 做 target snapshot 校验，
    //   而本夹具的迁移路径 checkout.revision=None 会触发无关的 snapshot 漂移；
    //   锁释放语义由 abort 路径覆盖，保持本测试聚焦仓维锁本身。）
    let (abort_status, abort_body) = request_json(
        app.clone(),
        Method::POST,
        &scoped_attempt_uri(&winner_attempt_id, "/abort"),
        json!({}),
    )
    .await;
    assert_eq!(abort_status, StatusCode::OK, "{abort_body}");

    // 显式断言：获胜方 abort 后仓维锁已清空（锁不泄漏），失败方可重新 acquire。
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let shared_after_abort = lifecycle
        .get_repo_shared_worktree("project_0001", "issue_0001", fixture.logical_target)
        .expect("target repo worktree read")
        .expect("target repo worktree exists");
    assert_eq!(shared_after_abort.current_active_work_item_id, None);
    assert_eq!(shared_after_abort.current_lock_owner_id, None);

    // 之前失败方重新启动 → 成功 acquire（锁不泄漏）。
    let loser_uri = format!(
        "/api/projects/project_0001/issues/issue_0001/work-items/{loser_work_item_id}/coding-attempts"
    );
    let (retry_status, retry_body) =
        request_json(app, Method::POST, &loser_uri, json!({})).await;
    assert_eq!(
        retry_status,
        StatusCode::OK,
        "失败方在成功方 release 后必须能重新 acquire: {retry_body}"
    );
    assert_global_attempt_id(&retry_body);
    assert_eq!(retry_body["work_item_id"], loser_work_item_id);
}
