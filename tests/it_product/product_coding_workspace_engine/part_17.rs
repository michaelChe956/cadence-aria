// ── 本组夹具：LC 权威 + 冻结 `target_snapshot` + 仓维 shared worktree 锁 ───
//
// 块3 删掉 `IssueSharedWorktreeRoute::Legacy` 臂后，引擎的 shared worktree 事实
// **只在仓维**（三元键 `(project, issue, logical repository)`）：
// `route_issue_shared_worktree` 对无归属 attempt 一律 fail-closed
// （`target_snapshot_identity_drifted`），有归属则读
// `shared-worktrees/{repository_id}.json`；同 issue 下若残留 legacy
// `issue-shared-worktree.json` 则 fail-closed `legacy_shared_worktree_present`。
//
// ⇒ 本组四条「owner 冲突」用例的现场必须建在**仓维**：
// - 旧形态（issue 维 `issue-shared-worktree.json` +
//   `try_acquire_issue_worktree_lock`）在两态化后根本不被引擎读 ⇒ 断言读到的
//   不是真冲突；
// - 缺 LC 权威时引擎走 fail-closed ⇒ 断言「绿在错误的原因上」（确实报错，
//   但原因是归属漂移，不是锁归属冲突）。故这四条都必须补 LC 权威 + 冻结快照，
//   并把断言钉到仓维冲突 kind 上（不是「只要报错就算过」）。
//
// 本组只改【断言的读取维度】（issue 维 → 仓维）与【冲突 kind】
// （`issue_worktree_lock_owner` → `repo_worktree_lock_owner`）；断言语义
// （谁持锁、冲突时不得推进/不得完成/不得中止/不得失败）一律保持并加强。

/// part_17 私有：建**仓维** shared worktree 记录，并把锁交给指定 owner。
///
/// 仓维三元键 `(project, issue, logical repository)`；锁 owner 即
/// `current_lock_owner_id`，持锁 work item 即 `current_active_work_item_id`。
/// owner 不是本 attempt 时，引擎的 `validate_repo_worktree_lock_owner` 报
/// `Conflict{kind:"repo_worktree_lock_owner"}`。
fn part17_seed_repo_worktree_lock(
    paths: &ProductAppPaths,
    authority: &LogicalAuthority,
    attempt: &CodingExecutionAttempt,
    work_item_id: &str,
    owner_id: &str,
) {
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_repo_shared_worktree(
            cadence_aria::product::lifecycle_store::UpsertRepoSharedWorktreeInput {
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                repository_id: authority.member_id,
                branch_name: attempt.branch_name.clone(),
                worktree_path: attempt
                    .worktree_path
                    .clone()
                    .unwrap_or_else(|| paths.root().join("group-worktree")),
                base_branch: attempt.base_branch.clone(),
            },
        )
        .expect("upsert repo shared worktree");
    lifecycle
        .try_acquire_repo_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            authority.member_id,
            work_item_id,
            owner_id,
        )
        .expect("acquire conflicting repo worktree lock");
}

/// part_17 私有：读回仓维锁现场（冲突后锁必须仍由他人持有）。
fn part17_read_repo_worktree_lock(
    paths: &ProductAppPaths,
    authority: &LogicalAuthority,
    attempt: &CodingExecutionAttempt,
) -> cadence_aria::product::models::IssueSharedWorktree {
    LifecycleStore::new(paths.clone())
        .get_repo_shared_worktree(&attempt.project_id, &attempt.issue_id, authority.member_id)
        .expect("read repo shared worktree")
        .expect("repo shared worktree present")
}

/// part_17 私有：group attempt（双 unit）+ LC 权威 + 冻结 `target_snapshot`。
///
/// ⚠️ **LC 权威必须先于 coding attempt 建立**：`ProjectStore::create` 的 project
/// id 按 `projects/` 目录数分配，而 attempt 落盘本身就会建
/// `projects/project_0001/issues/issue_0001/` ⇒ 先建 attempt 再建权威会把
/// project 分配成 `project_0002`，权威与 attempt 的 project 身份错位 ⇒ 路由
/// 恒 fail-closed `target_snapshot_identity_drifted`。
fn part17_group_engine_with_two_units_under_lc_authority() -> (
    tempfile::TempDir,
    ProductAppPaths,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
    LogicalAuthority,
) {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let authority = seed_lc_authority(&paths, root.path());
    // group attempt 的准入要解析归属 work item，故补齐 work item + plan。
    seed_group_work_items_and_plan(&paths);
    let store = CodingAttemptStore::new(paths.clone());
    let worktree = root.path().join("group-worktree");
    init_group_worktree(&worktree);
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: Some(authority.snapshot(&paths)),
            max_auto_rework: 2,
            start_run_policy: cadence_aria::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .expect("create group attempt");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0001".to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("create coding unit 1");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0002".to_string(),
            work_item_revision_id: "work_item_revision_0002".to_string(),
            dependency_logical_work_item_ids: vec!["work_item_0001".to_string()],
            order_index: 1,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("create coding unit 2");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    (root, paths, store, engine, attempt, authority)
}

/// part_17 私有：WorkItem 范围 attempt + LC 权威 + 冻结 `target_snapshot`
/// （FinalConfirm 待确认现场）。
fn part17_final_confirm_attempt_under_lc_authority(
    paths: &ProductAppPaths,
    authority: &LogicalAuthority,
    work_item_id: &str,
) -> (CodingAttemptStore, CodingExecutionAttempt) {
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some(work_item_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "work item".to_string(),
            ..Default::default()
        })
        .expect("create work item");
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: Some(authority.snapshot(paths)),
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = crate::seed_coding_attempt_running(
        &store,
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    );
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::FinalConfirm,
        )
        .expect("set final confirm stage");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::WaitingForHuman,
        )
        .expect("set waiting for human");
    let attempt = store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some("deadbeef".to_string()),
        )
        .expect("set head commit");
    (store, attempt)
}

/// part_17 私有：WorkItem 范围 attempt + LC 权威 + 冻结 `target_snapshot`
/// （Created 现场）。
fn part17_coding_store_with_attempt_under_lc_authority(
    paths: &ProductAppPaths,
    authority: &LogicalAuthority,
    work_item_id: &str,
    branch_name: &str,
) -> (CodingAttemptStore, CodingExecutionAttempt) {
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: branch_name.to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: Some(authority.snapshot(paths)),
            max_auto_rework: 2,
        })
        .expect("create attempt");
    (store, attempt)
}

#[tokio::test]
async fn group_unit_owner_conflict_does_not_advance_unit_or_attempt() {
    let (_root, paths, store, engine, attempt, authority) =
        part17_group_engine_with_two_units_under_lc_authority();
    // 仓维现场：锁被他人（coding_attempt_other）持在 work_item_0001 上。
    part17_seed_repo_worktree_lock(
        &paths,
        &authority,
        &attempt,
        "work_item_0001",
        "coding_attempt_other",
    );

    let error = engine
        .complete_current_group_unit(&attempt, Some("must not persist".to_string()))
        .await
        .expect_err("owner mismatch must fail before state changes");
    // 断言钉到**仓维**锁归属冲突（原断言读 issue 维 kind `issue_worktree_lock_owner`）。
    assert!(matches!(
        error,
        cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngineError::Store(
            cadence_aria::product::json_store::ProductStoreError::Conflict {
            kind: "repo_worktree_lock_owner",
            ..
            }
        )
    ));

    let current = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reload attempt");
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reload units");
    assert_eq!(current.status, CodingAttemptStatus::Created);
    assert_eq!(current.current_work_item_id.as_deref(), Some("work_item_0001"));
    assert_eq!(current.active_unit_id.as_deref(), Some("coding_unit_0001"));
    assert_eq!(units[0].status, CodingExecutionUnitStatus::Running);
    assert_eq!(units[1].status, CodingExecutionUnitStatus::Pending);

    // 冲突后仓维锁仍由他人持有（推进不得夺走他人锁）。
    let record = part17_read_repo_worktree_lock(&paths, &authority, &attempt);
    assert_eq!(
        record.current_lock_owner_id.as_deref(),
        Some("coding_attempt_other")
    );
    assert_eq!(
        record.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );
}

#[tokio::test]
async fn final_confirm_owner_conflict_does_not_complete_attempt() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let authority = seed_lc_authority(&paths, root.path());
    let (store, attempt) =
        part17_final_confirm_attempt_under_lc_authority(&paths, &authority, "work_item_0001");
    part17_seed_repo_worktree_lock(
        &paths,
        &authority,
        &attempt,
        "work_item_0001",
        "coding_attempt_other",
    );
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .handle_final_confirm("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("owner mismatch must fail before terminal status");
    // 断言钉到**仓维**锁归属冲突（原断言读 issue 维 kind `issue_worktree_lock_owner`）。
    assert!(matches!(
        error,
        cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngineError::Store(
            cadence_aria::product::json_store::ProductStoreError::Conflict {
                kind: "repo_worktree_lock_owner",
                ..
            }
        )
    ));
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("reload attempt")
            .status,
        CodingAttemptStatus::WaitingForHuman
    );
    // 冲突后仓维锁仍由他人持有（确认不得夺走他人锁）。
    let record = part17_read_repo_worktree_lock(&paths, &authority, &attempt);
    assert_eq!(
        record.current_lock_owner_id.as_deref(),
        Some("coding_attempt_other")
    );
}

#[tokio::test]
async fn abort_owner_conflict_does_not_abort_attempt() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let authority = seed_lc_authority(&paths, root.path());
    let (store, attempt) = part17_coding_store_with_attempt_under_lc_authority(
        &paths,
        &authority,
        "work_item_0001",
        "branch",
    );
    part17_seed_repo_worktree_lock(
        &paths,
        &authority,
        &attempt,
        "work_item_0001",
        "coding_attempt_other",
    );
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .handle_abort("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("owner mismatch must fail before abort");
    // 断言钉到**仓维**锁归属冲突：本条原断言只要求「报错 + 状态未变」，
    // 缺 LC 权威时恰好被 fail-closed 满足（绿在错误的原因上）⇒ 必须补齐
    // 权威并把断言钉到真正的仓维冲突，否则中止会成功、断言翻红。
    assert!(matches!(
        error,
        cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngineError::Store(
            cadence_aria::product::json_store::ProductStoreError::Conflict {
                kind: "repo_worktree_lock_owner",
                ..
            }
        )
    ));
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("reload attempt")
            .status,
        CodingAttemptStatus::Created
    );
    // 冲突后仓维锁仍由他人持有（中止不得夺走他人锁）。
    let record = part17_read_repo_worktree_lock(&paths, &authority, &attempt);
    assert_eq!(
        record.current_lock_owner_id.as_deref(),
        Some("coding_attempt_other")
    );
    assert_eq!(
        record.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );
}

#[tokio::test]
async fn failure_owner_conflict_does_not_fail_attempt() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let authority = seed_lc_authority(&paths, root.path());
    let (store, attempt) = part17_coding_store_with_attempt_under_lc_authority(
        &paths,
        &authority,
        "work_item_0001",
        "branch",
    );
    part17_seed_repo_worktree_lock(
        &paths,
        &authority,
        &attempt,
        "work_item_0001",
        "coding_attempt_other",
    );
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .handle_attempt_failed("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("owner mismatch must fail before terminal failure");
    // 断言钉到**仓维**锁归属冲突：本条原断言只要求「报错 + 状态未变」，
    // 缺 LC 权威时恰好被 fail-closed 满足（绿在错误的原因上）⇒ 必须补齐
    // 权威并把断言钉到真正的仓维冲突，否则失败收尾会成功、断言翻红。
    assert!(matches!(
        error,
        cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngineError::Store(
            cadence_aria::product::json_store::ProductStoreError::Conflict {
                kind: "repo_worktree_lock_owner",
                ..
            }
        )
    ));
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("reload attempt")
            .status,
        CodingAttemptStatus::Created
    );
    // 冲突后仓维锁仍由他人持有（失败收尾不得夺走他人锁）。
    let record = part17_read_repo_worktree_lock(&paths, &authority, &attempt);
    assert_eq!(
        record.current_lock_owner_id.as_deref(),
        Some("coding_attempt_other")
    );
    assert_eq!(
        record.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );
}
