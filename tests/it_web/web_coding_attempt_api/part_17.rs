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
    let persisted = CodingAttemptStore::new(app_paths.clone())
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
async fn multi_repo_work_item_attempt_uses_target_repo_worktree_not_primary() {
    // REQ-COD-01 分层 c：多仓 WorkItem 启动 coding 时，shared worktree 必须落在
    // 目标仓 checkout 下（不是 issue 的 primary 仓），且单仓 legacy 记录不被写入。
    let root = tempdir().expect("root");
    let primary_repo = git_repo();
    let target_repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    // 项目 + 两个物理仓 + issue 绑定 primary 仓。
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name": "Coding", "description": null}),
    )
    .await;
    let primary = register_repository_and_wait(
        app.clone(),
        json!({"name": "Primary", "path": primary_repo.path(), "default_provider_mode": "fake"}),
    )
    .await;
    assert_eq!(primary["repository_id"], "repository_0001");
    let target = register_repository_and_wait(
        app.clone(),
        json!({"name": "Target", "path": target_repo.path(), "default_provider_mode": "fake"}),
    )
    .await;
    assert_eq!(target["repository_id"], "repository_0002");
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title": "多仓工作项",
            "description": "target 在 repository_0002",
            "repository_id": "repository_0001"
        }),
    )
    .await;

    // WorkItem 绑定 target 仓；确认 plan 后经迁移获得 target_repository_id。
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0002".to_string(),
            title: "实现目标仓功能".to_string(),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create work item");

    IdentityMigrationExecutor::new(app_paths.clone())
        .ensure_identity_schema("project_0001")
        .expect("migrate fixture to logical codebase");
    let target_logical_id = LogicalCodebaseStore::new(app_paths.clone())
        .list_members("project_0001")
        .expect("logical members")
        .into_iter()
        .find(|member| member.physical_repository_id == "repository_0002")
        .expect("target logical member")
        .logical_repository_id;

    let primary_head_before = git_head_of(primary_repo.path());

    let (status, created) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "response: {created}");

    // 仓维 shared worktree 记录必须指向目标仓 checkout（不是 primary 仓）。
    let shared = lifecycle
        .get_repo_shared_worktree("project_0001", "issue_0001", target_logical_id)
        .expect("repo shared worktree read")
        .expect("repo shared worktree must exist");
    assert_eq!(
        shared.worktree_path,
        target_repo
            .path()
            .join(".worktrees")
            .join("aria-issues")
            .join("issue_0001"),
        "多仓 worktree 必须落在目标仓 checkout 下"
    );
    // 单仓 legacy 记录不得被写入（红线：Legacy 路径只在单仓 attempt 时使用）。
    assert!(
        lifecycle
            .get_issue_shared_worktree("project_0001", "issue_0001")
            .expect("legacy shared worktree read")
            .is_none(),
        "多仓 attempt 不得写 issue-shared-worktree.json"
    );
    // primary 仓 HEAD 与工作区保持不变。
    assert_eq!(
        git_head_of(primary_repo.path()),
        primary_head_before,
        "primary checkout HEAD 不得变化"
    );
    assert!(
        !primary_repo.path().join(".worktrees").exists(),
        "primary checkout 下不得创建 .worktrees"
    );
}

fn git_head_of(repo_path: &std::path::Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()
        .expect("git rev-parse HEAD");
    assert!(
        output.status.success(),
        "git rev-parse HEAD failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 git output")
        .trim()
        .to_string()
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
    let persisted = CodingAttemptStore::new(app_paths.clone())
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

#[tokio::test]
async fn logical_group_initialization_replay_reuses_journal_target_snapshot() {
    use cadence_aria::product::coding_attempt_store::CodingGroupInitializationPhase;
    use cadence_aria::web::test_controls::GroupAttemptInitializationCheckpoint;

    for (checkpoint, expected_phase) in [
        (
            GroupAttemptInitializationCheckpoint::PreparedBeforeAttemptPersisted,
            CodingGroupInitializationPhase::Prepared,
        ),
        (
            GroupAttemptInitializationCheckpoint::PersistedBeforeBind,
            CodingGroupInitializationPhase::AttemptPersisted,
        ),
    ] {
        let root = tempdir().expect("root");
        let repo = git_repo();
        let initial_state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        );
        initial_state
            .test_controls
            .fail_next_group_attempt_initialization_at(checkpoint);
        let initial_app = build_web_router(initial_state);
        bootstrap_confirmed_work_item_plan_group(initial_app.clone(), repo.path()).await;
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let logical_repository_id = seed_group_logical_target_fixture(&app_paths, repo.path());
        let create_path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";

        let (interrupted_status, interrupted) =
            request_json(initial_app, Method::POST, create_path, json!({})).await;
        assert_eq!(
            interrupted_status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{checkpoint:?}: {interrupted}"
        );
        assert_eq!(
            interrupted["code"],
            "coding_group_initialization_interrupted",
            "{checkpoint:?}"
        );

        let store = CodingAttemptStore::new(app_paths.clone());
        let interrupted_journal = store
            .get_group_initialization("project_0001", "issue_0001", "work_item_plan_0001")
            .expect("interrupted logical group initialization journal");
        assert_eq!(
            interrupted_journal.phase, expected_phase,
            "{checkpoint:?}"
        );
        let frozen_snapshot = interrupted_journal
            .attempt
            .target_snapshot
            .clone()
            .expect("logical group initialization must freeze a target snapshot");
        assert_eq!(frozen_snapshot.logical_repository_id, logical_repository_id);

        let restarted_app = build_web_router(WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        ));
        let (retry_status, retry) =
            request_json(restarted_app.clone(), Method::POST, create_path, json!({})).await;
        assert_eq!(retry_status, StatusCode::OK, "{checkpoint:?}: {retry}");
        assert_eq!(
            retry["attempt_id"], interrupted_journal.attempt.id,
            "{checkpoint:?}"
        );

        let completed_journal = store
            .get_group_initialization("project_0001", "issue_0001", "work_item_plan_0001")
            .expect("completed logical group initialization journal");
        assert_eq!(
            completed_journal.phase,
            CodingGroupInitializationPhase::Completed,
            "{checkpoint:?}"
        );

        let (replay_status, replay) =
            request_json(restarted_app, Method::POST, create_path, json!({})).await;
        assert_eq!(
            replay_status,
            StatusCode::OK,
            "{checkpoint:?}: completed journal replay: {replay}"
        );
        assert_eq!(
            replay["attempt_id"], interrupted_journal.attempt.id,
            "{checkpoint:?}: completed journal replay must return the original attempt"
        );

        let persisted = store
            .get_attempt(
                "project_0001",
                "issue_0001",
                &interrupted_journal.attempt.id,
            )
            .expect("replayed logical group attempt");
        assert_eq!(
            persisted.target_snapshot.as_ref(),
            Some(&frozen_snapshot),
            "{checkpoint:?}: replay must preserve the journal-frozen target snapshot, including captured_at"
        );
        assert_eq!(
            completed_journal.attempt.target_snapshot.as_ref(),
            Some(&frozen_snapshot),
            "{checkpoint:?}: completed journal must preserve the frozen target snapshot"
        );
    }
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

/// R9：非 legacy 新 LC issue 的 coding attempt 目标快照必须按 issue 所属 lc_id
/// 从 `logical-codebases/{lc_id}/` 子树权威记录采集（repos.json 无投影）。
#[tokio::test]
async fn new_lc_attempt_persists_frozen_target_snapshot_from_lc_subtree() {
    use cadence_aria::product::models::{IssueRecord, LifecycleWorkItemRecord};
    use cadence_aria::product::json_store::{read_json, write_json};
    use cadence_aria::product::logical_codebase::{
        AggregatePolicyArtifactStore, CheckoutAvailability, CheckoutKind,
        CodebaseMemberRecord as LcMemberRecord, IdentityRegistryEntry, IdentityRegistryStore,
        LogicalCodebaseCreateInput, LogicalCodebaseManifest, MemberStatus,
        RepositoryCheckoutId, RepositoryCheckoutRecord, RepositoryType,
    };
    use uuid::Uuid;

    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));

    // 新 LC：权威记录只落在 logical-codebases/{lc_id}/ 子树。
    let record = LogicalCodebaseStore::new(app_paths.clone())
        .create(
            "project_0001",
            LogicalCodebaseCreateInput {
                name: "new-lc".to_string(),
                aggregate_root: root.path().join("aggregate-root"),
            },
        )
        .expect("create logical codebase");
    let lc_id = record.id;
    let authority = LogicalCodebaseStore::for_lc(app_paths.clone(), lc_id.clone());
    let logical_id = LogicalRepositoryId(Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
    let physical_id = format!("repository_{}", Uuid::new_v4().simple());
    let canonical_path = repo.path().to_path_buf();
    let manifest = LogicalCodebaseManifest::new(
        "project_0001",
        root.path().join("aggregate-root"),
        vec![logical_id],
    );
    authority
        .save_manifest("project_0001", &manifest)
        .expect("save lc manifest");
    let now = "2026-08-18T00:00:00Z".to_string();
    let source_identity =
        RepositorySourceIdentity::from_git_parts(&canonical_path, canonical_path.join(".git"), None);
    authority
        .save_member(
            "project_0001",
            &LcMemberRecord {
                logical_repository_id: logical_id,
                physical_repository_id: physical_id.clone(),
                alias: "repo".to_string(),
                role: "repository".to_string(),
                ordinal: 0,
                source_identity: source_identity.clone(),
                repo_type: RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_id],
                status: MemberStatus::Active,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .expect("save lc member");
    authority
        .save_checkout(
            "project_0001",
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: logical_id,
                physical_repository_id: physical_id.clone(),
                kind: CheckoutKind::Main,
                canonical_path: canonical_path.clone(),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: source_identity.git_dir_identity().to_string(),
                revision: None,
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            },
        )
        .expect("save lc checkout");
    IdentityRegistryStore::new(app_paths.clone())
        .upsert_active(
            "project_0001",
            IdentityRegistryEntry::active(
                source_identity,
                logical_id,
                physical_id.clone(),
                checkout_id,
                "new-lc-attempt-fixture".to_string(),
            ),
        )
        .expect("register identity");
    let policy = AggregatePolicyArtifactStore::for_lc(app_paths.clone(), lc_id.clone())
        .ensure_bootstrap(&manifest)
        .expect("ensure lc policy");

    // issue 归属 + selection + work item 目标仓（测试内直接落盘权威记录）。
    let issue_path = app_paths.issue_root("project_0001", "issue_0001").join("issue.json");
    let mut issue: IssueRecord = read_json(&issue_path).expect("read issue");
    issue.logical_codebase_id = Some(lc_id.clone());
    write_json(&issue_path, &issue).expect("write issue attribution");
    IssueCodebaseSelectionStore::for_lc(app_paths.clone(), lc_id.clone())
        .save(&IssueCodebaseSelection::all_members(
            "project_0001",
            "issue_0001",
            None,
        )
        .for_logical_codebase(lc_id.clone()))
        .expect("save lc selection");
    let work_item_path = app_paths
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("work-items")
        .join("work_item_0001.json");
    let mut work_item: LifecycleWorkItemRecord =
        read_json(&work_item_path).expect("read work item");
    work_item.target_repository_id = Some(logical_id);
    write_json(&work_item_path, &work_item).expect("write work item target");

    let (status, created) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "response: {created}");
    let attempt_id = assert_global_attempt_id(&created);
    let persisted = CodingAttemptStore::new(app_paths.clone())
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("persisted attempt");
    let snapshot = persisted
        .target_snapshot
        .expect("new-lc attempt must persist a frozen target snapshot");
    assert_eq!(snapshot.logical_repository_id, logical_id);
    assert_eq!(snapshot.checkout_id, checkout_id);
    assert_eq!(snapshot.physical_repository_id, physical_id);
    assert_eq!(snapshot.canonical_path, canonical_path);
    assert_eq!(snapshot.policy_digest, policy.digest);
    assert_eq!(snapshot.membership_revision, manifest.membership_revision);
    assert!(
        snapshot.revision.as_deref().is_some_and(|r| !r.is_empty()),
        "snapshot revision must be non-empty"
    );
    // 新 LC 成员不写 repos.json 投影（仅 bootstrap 的单仓记录在）。
    let repositories: Vec<cadence_aria::product::models::RepositoryRecord> =
        read_json(&app_paths.project_root("project_0001").join("repos.json"))
            .expect("read repos.json");
    assert!(repositories
        .iter()
        .all(|record| record.id != physical_id));
}
