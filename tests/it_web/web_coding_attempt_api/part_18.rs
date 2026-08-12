// Task 12（REQ-COD-04）：mixed-target group 在创建/恢复/replay 三处调同一函数
// `validate_group_single_target` 一致拒绝，堵 Some+None 绕过，稳定码
// `mixed_target_group_rejected` → 422 UNPROCESSABLE_ENTITY。

fn put_group_draft_target(
    app_paths: &ProductAppPaths,
    draft_id: &str,
    outline_id: &str,
    logical_work_item_id: &str,
    target_repository_id: Option<LogicalRepositoryId>,
    title: &str,
) {
    WorkItemPlanStore::new(app_paths.clone())
        .put_draft_record(&WorkItemDraftRecord {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            draft_id: draft_id.to_string(),
            outline_id: outline_id.to_string(),
            generation_round_id: "round_0001".to_string(),
            batch_id: None,
            attempt_index: 1,
            outline_version_ref: "outline_version_0001".to_string(),
            generation_mode: WorkItemGenerationMode::Serial,
            generation_diagnostics: None,
            candidate: WorkItemDraftCandidate {
                target_repository_id,
                outline_id: outline_id.to_string(),
                logical_work_item_id: logical_work_item_id.to_string(),
                canonical_contract_candidate: group_canonical_contract(logical_work_item_id, title),
                verification_plan: WorkItemDraftVerificationPlan { checks: Vec::new() },
            },
            status: WorkItemDraftStatus::Accepted,
            active: true,
            superseded_by_draft_id: None,
            supersede_reason: None,
            copied_from_draft_id: None,
            review_node_id: None,
            review_verdict_ref: None,
            generated_from_node_id: "timeline_node_0001".to_string(),
            accepted_at: Some("2026-08-11T00:00:00Z".to_string()),
            superseded_at: None,
            created_at: "2026-08-11T00:00:00Z".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
        })
        .expect("authoritative draft");
}

/// item2 的 source draft 改成 None target，与 item1 的 Some(target) 形成 Some+None。
fn rewrite_item2_draft_target_none(app_paths: &ProductAppPaths) {
    put_group_draft_target(
        app_paths,
        "draft_work_item_revision_0002",
        "outline_0002",
        "work_item_0002",
        None,
        "实现爬楼梯 part 2",
    );
}

#[tokio::test]
async fn mixed_target_group_creation_some_none_is_rejected_422() {
    // 创建处：Some(A)+None 必须被 validate_group_single_target 拒绝（web handler 的
    // filter_map 会放过它），稳定码 mixed_target_group_rejected → 422。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    seed_group_logical_target_fixture(&app_paths, repo.path());
    rewrite_item2_draft_target_none(&app_paths);

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "response: {body}");
    assert_eq!(body["code"], "mixed_target_group_rejected");
}

#[tokio::test]
async fn mixed_target_group_recovery_rejects_drifted_some_none() {
    // 恢复处：已创建的 Logical 单目标 group，权威 unit 漂移成 Some+None 后，
    // validate_group_attempt_integrity 必须拒绝（recovery 调用同一函数）。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    seed_group_logical_target_fixture(&app_paths, repo.path());

    let (create_status, created) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(create_status, StatusCode::OK, "create: {created}");
    let attempt_id = assert_global_attempt_id(&created);

    rewrite_item2_draft_target_none(&app_paths);

    let store = CodingAttemptStore::new(app_paths);
    let attempt = store
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("persisted attempt");
    let error = store
        .validate_group_attempt_integrity(&attempt)
        .expect_err("drifted Some+None group must be rejected on recovery");
    assert!(
        error.to_string().contains("mixed_target_group_rejected"),
        "recovery must surface mixed_target_group_rejected, got: {error}"
    );
}

#[tokio::test]
async fn mixed_target_group_replay_rejects_drifted_some_none() {
    // replay 处：初始化中断后权威 unit 漂移成 Some+None，重放必须拒绝
    // （replay 走 prepare_group_initialization → validate_group_initialization_input）。
    use cadence_aria::product::coding_attempt_store::CodingGroupInitializationPhase;
    use cadence_aria::web::test_controls::GroupAttemptInitializationCheckpoint;

    let root = tempdir().expect("root");
    let repo = git_repo();
    let initial_state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    initial_state
        .test_controls
        .fail_next_group_attempt_initialization_at(
            GroupAttemptInitializationCheckpoint::PreparedBeforeAttemptPersisted,
        );
    let initial_app = build_web_router(initial_state);
    bootstrap_confirmed_work_item_plan_group(initial_app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    seed_group_logical_target_fixture(&app_paths, repo.path());
    let create_path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";

    let (interrupted_status, interrupted) =
        request_json(initial_app, Method::POST, create_path, json!({})).await;
    assert_eq!(
        interrupted_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{interrupted}"
    );
    assert_eq!(interrupted["code"], "coding_group_initialization_interrupted");

    let store = CodingAttemptStore::new(app_paths.clone());
    let interrupted_journal = store
        .get_group_initialization("project_0001", "issue_0001", "work_item_plan_0001")
        .expect("interrupted journal");
    assert_eq!(
        interrupted_journal.phase,
        CodingGroupInitializationPhase::Prepared
    );

    rewrite_item2_draft_target_none(&app_paths);

    let restarted_app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let (retry_status, retry) =
        request_json(restarted_app, Method::POST, create_path, json!({})).await;
    assert_eq!(
        retry_status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "replay of drifted Some+None group must be rejected: {retry}"
    );
    assert_eq!(retry["code"], "mixed_target_group_rejected");
}
