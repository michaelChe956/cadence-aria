// 阶段 D WP6：多仓安全矩阵补缺。
// 已由 part_20 覆盖同仓串行/异仓隔离；本文件只覆盖 it_web 缺失的
// target missing blocker 与 checkout identity fail-closed HTTP 路径。

#[tokio::test]
async fn logical_work_item_without_target_repository_is_blocked_over_http() {
    // REQ-TGT-02：多仓 routing 下 WorkItem 缺少 target_repository_id 不得回落
    // 到 Issue 的 primary/legacy repository_id，必须返回稳定 blocker 码。
    let root = tempdir().expect("root");
    let primary_repo = git_repo();
    let target_repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state);
    bootstrap_multi_repo_two_work_items(
        app.clone(),
        root.path(),
        primary_repo.path(),
        target_repo.path(),
        "repository_0002",
        "repository_0002",
    )
    .await;

    // Migration normally backfills the target; explicitly remove it to model a
    // provider-produced WorkItem that reached the execution boundary without one.
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let work_item_path = paths
        .issue_root("project_0001", "issue_0001")
        .join("work-items")
        .join("work_item_0001.json");
    let mut work_item: cadence_aria::product::models::LifecycleWorkItemRecord =
        cadence_aria::product::json_store::read_json(&work_item_path).expect("work item");
    work_item.target_repository_id = None;
    assert_eq!(work_item.repository_id, "repository_0002");
    cadence_aria::product::json_store::write_json(&work_item_path, &work_item)
        .expect("target-less work item");
    let persisted: cadence_aria::product::models::LifecycleWorkItemRecord =
        cadence_aria::product::json_store::read_json(&work_item_path).expect("work item");
    assert_eq!(persisted.target_repository_id, None);

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "repository_routing_target_missing");
    assert!(
        body["details"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("has no target repository")),
        "blocker must explain missing target: {body}"
    );
}

#[tokio::test]
async fn logical_attempt_checkout_identity_forgery_is_rejected_fail_closed() {
    // REQ-ENV-05/REQ-COD-02：attempt 冻结的 canonical_path 与 git-dir identity
    // 必须同时匹配 authority checkout；仅能解析到一个可用 Git 仓不代表身份有效。
    let root = tempdir().expect("root");
    let primary_repo = git_repo();
    let target_repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_multi_repo_two_work_items(
        app.clone(),
        root.path(),
        primary_repo.path(),
        target_repo.path(),
        "repository_0002",
        "repository_0002",
    )
    .await;

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    // The helper's target is intentionally absent for the first item, so set it
    // to the real logical member before creating a legitimate attempt.
    let members = LogicalCodebaseStore::new(paths.clone())
        .list_members("project_0001")
        .expect("logical members");
    let target = members
        .iter()
        .find(|member| member.physical_repository_id == "repository_0002")
        .expect("target member")
        .logical_repository_id;
    let work_item_path = paths
        .issue_root("project_0001", "issue_0001")
        .join("work-items")
        .join("work_item_0001.json");
    let mut work_item: cadence_aria::product::models::LifecycleWorkItemRecord =
        cadence_aria::product::json_store::read_json(&work_item_path).expect("work item");
    work_item.target_repository_id = Some(target);
    cadence_aria::product::json_store::write_json(&work_item_path, &work_item)
        .expect("target work item");

    let (status, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let attempt_id = assert_global_attempt_id(&created);

    // Forge only the persisted authority checkout path while retaining a real
    // Git checkout and a valid logical/member identity. The scoped GET must
    // fail closed rather than silently resolving the physical repository.
    let authority = LogicalCodebaseStore::new(paths.clone());
    let checkout_id = authority
        .load_member("project_0001", target)
        .expect("member")
        .expect("target member")
        .checkout_ids[0];
    let checkout_path = authority
        .load_checkout("project_0001", checkout_id)
        .expect("checkout")
        .expect("target checkout")
        .canonical_path;
    let mut forged: cadence_aria::product::logical_codebase::RepositoryCheckoutRecord = authority
        .load_checkout("project_0001", checkout_id)
        .expect("checkout")
        .expect("target checkout");
    forged.canonical_path = primary_repo.path().to_path_buf();
    forged.git_dir_identity = "sha256:forged-git-dir-identity".to_string();
    assert_ne!(forged.canonical_path, checkout_path);
    authority
        .save_checkout("project_0001", &forged)
        .expect("forged checkout fixture");

    // DELETE resolves the attempt's frozen target through the strict authority
    // path; it must fail closed rather than silently resolving the physical repo.
    let (status, body) = request_json(
        app,
        Method::DELETE,
        &format!(
            "/api/projects/project_0001/issues/issue_0001/coding-attempts/{attempt_id}"
        ),
        json!({}),
    )
    .await;
    assert!(status.is_client_error(), "forged checkout must fail closed: {body}");
    assert_eq!(body["code"], "repository_routing_inconsistent");
}

#[tokio::test]
async fn logical_attempt_missing_checkout_is_rejected_fail_closed() {
    // clone/checkout unavailable simulation：availability=missing + deleted checkout
    // 必须阻止 coding attempt，而不是继续使用 primary 仓或旧路径。
    let root = tempdir().expect("root");
    let primary_repo = git_repo();
    let target_repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_multi_repo_two_work_items(
        app.clone(),
        root.path(),
        primary_repo.path(),
        target_repo.path(),
        "repository_0002",
        "repository_0002",
    )
    .await;

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let members = LogicalCodebaseStore::new(paths.clone())
        .list_members("project_0001")
        .expect("logical members");
    let target = members
        .iter()
        .find(|member| member.physical_repository_id == "repository_0002")
        .expect("target member")
        .logical_repository_id;
    let work_item_path = paths
        .issue_root("project_0001", "issue_0001")
        .join("work-items")
        .join("work_item_0001.json");
    let mut work_item: cadence_aria::product::models::LifecycleWorkItemRecord =
        cadence_aria::product::json_store::read_json(&work_item_path).expect("work item");
    work_item.target_repository_id = Some(target);
    cadence_aria::product::json_store::write_json(&work_item_path, &work_item)
        .expect("target work item");

    let authority = LogicalCodebaseStore::new(paths.clone());
    let member = authority
        .load_member("project_0001", target)
        .expect("member")
        .expect("target member");
    let checkout_id = member.checkout_ids[0];
    let mut checkout = authority
        .load_checkout("project_0001", checkout_id)
        .expect("checkout")
        .expect("target checkout");
    checkout.availability = cadence_aria::product::logical_codebase::CheckoutAvailability::Missing;
    let missing_path = checkout.canonical_path.clone();
    std::fs::remove_dir_all(&missing_path).expect("remove target checkout");
    authority
        .save_checkout("project_0001", &checkout)
        .expect("mark unavailable checkout");

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "repository_path_not_git_repo");
    assert_eq!(body["message"], "repository path must point to a git work tree");
}
