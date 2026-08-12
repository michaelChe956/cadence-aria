#[tokio::test]
async fn multi_repo_attempt_persists_frozen_target_snapshot() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    IdentityMigrationExecutor::new(app_paths.clone())
        .ensure_identity_schema("project_0001")
        .expect("migrate fixture to logical codebase");
    let logical_repository_id = LogicalCodebaseStore::new(app_paths.clone())
        .load_manifest("project_0001")
        .expect("load logical manifest")
        .expect("logical manifest")
        .member_ids[0];

    let (status, created) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "response: {created}");
    let attempt_id = assert_global_attempt_id(&created);
    let persisted = CodingAttemptStore::new(app_paths)
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("persisted attempt");
    let snapshot = persisted
        .target_snapshot
        .expect("logical attempt must persist a frozen target snapshot");
    assert_eq!(snapshot.logical_repository_id, logical_repository_id);
    assert!(
        snapshot.revision.as_deref().is_some_and(|revision| !revision.is_empty()),
        "snapshot revision must be non-empty"
    );
}

#[tokio::test]
async fn legacy_attempt_target_snapshot_remains_none() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;

    let (status, created) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "response: {created}");
    let attempt_id = assert_global_attempt_id(&created);
    let persisted = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("persisted attempt");
    assert_eq!(persisted.target_snapshot, None);
}

#[tokio::test]
async fn multi_repo_group_attempt_persists_frozen_target_snapshot() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let logical_repository_id = seed_group_logical_target_fixture(&app_paths, repo.path());

    let (status, created) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "response: {created}");
    let attempt_id = assert_global_attempt_id(&created);
    let persisted = CodingAttemptStore::new(app_paths)
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("persisted group attempt");
    let snapshot = persisted
        .target_snapshot
        .expect("logical group attempt must persist a frozen target snapshot");
    assert_eq!(snapshot.logical_repository_id, logical_repository_id);
    assert!(
        snapshot.revision.as_deref().is_some_and(|revision| !revision.is_empty()),
        "snapshot revision must be non-empty"
    );
}

fn seed_group_logical_target_fixture(
    app_paths: &ProductAppPaths,
    _repository_path: &std::path::Path,
) -> LogicalRepositoryId {
    IdentityMigrationExecutor::new(app_paths.clone())
        .ensure_identity_schema("project_0001")
        .expect("migrate fixture to logical codebase");
    let manifest = LogicalCodebaseStore::new(app_paths.clone())
        .load_manifest("project_0001")
        .expect("load logical manifest")
        .expect("logical manifest");
    let logical_repository_id = manifest.member_ids[0];
    IssueCodebaseSelectionStore::new(app_paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![logical_repository_id],
            Vec::new(),
            vec![logical_repository_id],
            None,
        ))
        .expect("issue selection");
    rewrite_group_draft_targets(app_paths, logical_repository_id);
    logical_repository_id
}

fn rewrite_group_draft_targets(app_paths: &ProductAppPaths, logical_repository_id: LogicalRepositoryId) {
    let draft_store = WorkItemPlanStore::new(app_paths.clone());
    for (draft_id, outline_id, logical_work_item_id, title) in [
        (
            "draft_work_item_revision_0001",
            "outline_0001",
            "work_item_0001",
            "实现爬楼梯",
        ),
        (
            "draft_work_item_revision_0002",
            "outline_0002",
            "work_item_0002",
            "实现爬楼梯 part 2",
        ),
    ] {
        draft_store
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
                    target_repository_id: Some(logical_repository_id),
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
            .expect("create authoritative draft");
    }
}

async fn seed_logical_fixture_multi_target(app: &axum::Router) {
    let app_paths = ProductAppPaths::new(workspace_root_from_app(app.clone()).await.join(".aria"));
    let target_one = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let target_two = LogicalRepositoryId(uuid::Uuid::from_u128(2));
    let logical_store = LogicalCodebaseStore::new(app_paths.clone());
    let aggregate_root = app_paths.root().join("aggregate-root");
    logical_store
        .save_manifest(
            "project_0001",
            &LogicalCodebaseManifest::new(
                "project_0001",
                aggregate_root.clone(),
                vec![target_one, target_two],
            ),
        )
        .expect("logical manifest");
    for (logical_repository_id, physical_repository_id, alias) in [
        (target_one, "repository_0001", "backend"),
        (target_two, "repository_0002", "frontend"),
    ] {
        let checkout_path = aggregate_root.join(alias);
        logical_store
            .save_member(
                "project_0001",
                &CodebaseMemberRecord {
                    logical_repository_id,
                    physical_repository_id: physical_repository_id.to_string(),
                    alias: alias.to_string(),
                    role: "service".to_string(),
                    ordinal: 1,
                    source_identity: RepositorySourceIdentity::from_git_parts(
                        &checkout_path,
                        checkout_path.join(".git"),
                        None,
                    ),
                    repo_type: RepositoryType::Backend,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: Vec::new(),
                    status: MemberStatus::Active,
                    created_at: "2026-08-11T00:00:00Z".to_string(),
                    updated_at: "2026-08-11T00:00:00Z".to_string(),
                },
            )
            .expect("logical member");
    }
    IssueCodebaseSelectionStore::new(app_paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![target_one, target_two],
            Vec::new(),
            vec![target_one],
            None,
        ))
        .expect("issue selection");

    let draft_store = WorkItemPlanStore::new(app_paths);
    for (draft_id, outline_id, logical_work_item_id, target_repository_id, title) in [
        (
            "draft_work_item_revision_0001",
            "outline_0001",
            "work_item_0001",
            target_one,
            "实现爬楼梯",
        ),
        (
            "draft_work_item_revision_0002",
            "outline_0002",
            "work_item_0002",
            target_two,
            "实现爬楼梯 part 2",
        ),
    ] {
        draft_store
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
                    target_repository_id: Some(target_repository_id),
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
            .expect("authority draft");
    }
}

#[tokio::test]
async fn delete_coding_attempt_then_rebuild_does_not_conflict_on_handoff() {
    // 删除后重建：第一个 attempt 的 handoff 被清理后，对同一 plan 重新创建 attempt
    // 不应报 group_completion_handoff_revision_conflict；重建成功。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));

    let (app, first_attempt_id, handoff_paths, _lineage, _store) =
        seed_group_attempt_with_published_handoffs(
            app,
            &app_paths,
            &[SeededHandoff {
                logical_work_item_id: "work_item_0001",
                unit_id: "coding_unit_0001",
                handoff_revision_id: "handoff_revision_coding_unit_run_0001",
            }],
        )
        .await;
    assert_eq!(handoff_paths.len(), 1);

    let (status, _body) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&first_attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        !handoff_paths[0].exists(),
        "precondition for rebuild: handoff cleaned after DELETE"
    );

    // 重建：对同一 plan 创建新 group attempt，断言不返回冲突且成功。
    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rebuild response: {body}");
    assert_ne!(body["code"], "group_completion_handoff_revision_conflict");
    let second_attempt_id = assert_global_attempt_id(&body);
    assert_ne!(second_attempt_id, first_attempt_id);
}
