// B 裁决修复(2026-09-03)配套测试:create group coding attempt 端点必须能收养 advance
// 已创建的 sc_advance 初始化 journal(amendment 轮 coding 段首测确定性 400 的根因)。
//
// 根因:advance 写 journal 时 admission_kind=ScAdvance、worktree_path=Some(...);而端点
// 重放固定传 admission_kind=LegacyGroup、worktree_path=None,journal_matches_request 的
// 全等校验必败 → 400 coding_group_attempt_incomplete。以下测试先在 store 层复刻 advance
// 初始化链(见 workspace_engine::advance::initialize_advance_inner),再以 HTTP 请求验证收养。

use cadence_aria::product::coding_attempt_store::CodingGroupInitializationJournal;
use cadence_aria::product::coding_attempt_store::CodingGroupInitializationPhase;
use cadence_aria::product::coding_models::CodingAdmissionKind;
use cadence_aria::product::repository_store::RepositoryStore;
use cadence_aria::web::provider_availability::resolve_default_coding_provider;

fn current_branch_of(repo_path: &std::path::Path) -> String {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(repo_path)
        .output()
        .expect("git branch --show-current");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// 复刻 advance 初始化:bootstrap 无 Confirmed WorkItem session,端点重放必然走
/// default provider 回退分支(coding_provider_config_snapshot_for_runtime_binding 尾部),
/// 与 fake runtime 的恒真 availability 组合,可确定性推导出端点将重算出的快照。
fn endpoint_replay_provider_snapshot(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
) -> ProviderConfigSnapshot {
    let sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list workspace sessions");
    assert!(
        !sessions.iter().any(|session| session.workspace_type == WorkspaceType::WorkItem
            && session.status == WorkspaceSessionStatus::Confirmed),
        "derivation assumes no confirmed WorkItem session exists"
    );
    let repository = RepositoryStore::new(app_paths.to_owned())
        .list("project_0001")
        .expect("list repositories")
        .into_iter()
        .next()
        .expect("fixture repository");
    let author = resolve_default_coding_provider(&repository.default_provider_mode, |_| true)
        .expect("default coding provider")
        .provider;
    ProviderConfigSnapshot {
        author: author.clone(),
        reviewer: Some(author),
        review_rounds: 1,
        permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
    }
}

/// 按 advance 初始化链(workspace_engine::advance::initialize_advance_inner)播种
/// sc_advance group 初始化 journal,推进到 stop_after 阶段:
/// - AttemptPersisted:attempt 已落盘、共享 worktree 尚未绑定(advance 中断形态)
/// - Completed:advance 全链完成(campaign 现场:advance record status=ready)
#[allow(clippy::too_many_lines)]
fn seed_advance_group_initialization(
    app_paths: &ProductAppPaths,
    repo_path: &std::path::Path,
    stop_after: CodingGroupInitializationPhase,
) -> CodingGroupInitializationJournal {
    let coding_store = CodingAttemptStore::new(app_paths.to_owned());
    let lifecycle = LifecycleStore::new(app_paths.to_owned());
    let _initialization_guard = coding_store
        .acquire_group_initialization_arbitration("project_0001", "issue_0001")
        .expect("advance initialization arbitration");
    let authoritative = coding_store
        .resolve_authoritative_group_plan_binding("project_0001", "issue_0001", "work_item_plan_0001")
        .expect("authoritative group plan binding");
    let current_unit = &authoritative.units[0];
    let repository = RepositoryStore::new(app_paths.to_owned())
        .list("project_0001")
        .expect("list repositories")
        .into_iter()
        .next()
        .expect("fixture repository");
    let creation_guard = coding_store
        .acquire_work_item_attempt_creation(
            "project_0001",
            "issue_0001",
            &current_unit.logical_work_item_id,
        )
        .expect("attempt creation guard");
    let input = CreateGroupCodingAttemptInput {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: "work_item_plan_0001".to_string(),
        current_work_item_id: current_unit.logical_work_item_id.clone(),
        base_branch: current_branch_of(repo_path),
        branch_name: "aria/issues/issue_0001".to_string(),
        worktree_path: Some(
            repo_path
                .join(".worktrees")
                .join("aria-issues")
                .join("issue_0001"),
        ),
        provider_config_snapshot: endpoint_replay_provider_snapshot(app_paths, &lifecycle),
        target_snapshot: None,
        max_auto_rework: 2,
    };
    let mut journal = coding_store
        .prepare_group_initialization_with_admission(
            &input,
            &authoritative.plan_revision_id,
            &authoritative.units,
            CodingAdmissionKind::ScAdvance,
        )
        .expect("prepare advance group initialization");
    assert_eq!(journal.attempt.admission_kind, CodingAdmissionKind::ScAdvance);
    assert!(journal.attempt.worktree_path.is_some());

    let attempt = coding_store
        .ensure_group_initialization_attempt(&journal, &creation_guard)
        .expect("persist advance group attempt");
    journal = coding_store
        .advance_group_initialization_phase(
            &journal,
            CodingGroupInitializationPhase::AttemptPersisted,
        )
        .expect("checkpoint attempt persistence");
    if stop_after == CodingGroupInitializationPhase::AttemptPersisted {
        return journal;
    }

    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            branch_name: journal.attempt.branch_name.clone(),
            worktree_path: journal.attempt.worktree_path.clone().expect("frozen worktree"),
            base_branch: journal.attempt.base_branch.clone(),
        })
        .expect("persist shared worktree");
    let lease = lifecycle
        .try_acquire_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            &journal.lock_work_item_id,
            &journal.worktree_lease_id,
        )
        .expect("acquire shared worktree lease");
    assert!(lease.acquired, "advance must own the shared worktree lease");
    lifecycle
        .bind_issue_worktree_lock_to_attempt(
            "project_0001",
            "issue_0001",
            &journal.lock_work_item_id,
            &attempt.id,
        )
        .expect("bind shared worktree lease");
    journal = coding_store
        .advance_group_initialization_phase(
            &journal,
            CodingGroupInitializationPhase::WorktreeBound,
        )
        .expect("checkpoint worktree binding");
    coding_store
        .ensure_group_initialization_plan_binding(&journal)
        .expect("persist advance plan binding");
    journal = coding_store
        .advance_group_initialization_phase(
            &journal,
            CodingGroupInitializationPhase::PlanBindingSaved,
        )
        .expect("checkpoint plan binding");
    for index in 0..journal.units.len() {
        coding_store
            .ensure_group_initialization_unit(&journal, index)
            .expect("persist advance group unit");
    }
    journal = coding_store
        .advance_group_initialization_phase(
            &journal,
            CodingGroupInitializationPhase::UnitsMaterialized,
        )
        .expect("checkpoint units materialization");
    let persisted_attempt = coding_store
        .get_attempt("project_0001", "issue_0001", &journal.attempt.id)
        .expect("persisted advance attempt");
    coding_store
        .validate_group_attempt_integrity(&persisted_attempt)
        .expect("advance attempt integrity");
    coding_store
        .advance_group_initialization_phase(&journal, CodingGroupInitializationPhase::Completed)
        .expect("checkpoint initialization completion")
}

#[tokio::test]
async fn create_group_coding_attempt_adopts_completed_advance_journal() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let journal = seed_advance_group_initialization(
        &app_paths,
        repo.path(),
        CodingGroupInitializationPhase::Completed,
    );
    let create_path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";

    let (status, body) = request_json(app.clone(), Method::POST, create_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "create must adopt the advance journal: {body}");
    assert_eq!(body["attempt_id"], journal.attempt.id);

    // 收养不改变既有 attempt 身份与准入标记;重复创建保持幂等。
    let store = CodingAttemptStore::new(app_paths.clone());
    let persisted = store
        .get_attempt("project_0001", "issue_0001", &journal.attempt.id)
        .expect("adopted attempt");
    assert_eq!(persisted.admission_kind, CodingAdmissionKind::ScAdvance);
    assert_eq!(
        persisted.worktree_path,
        journal.attempt.worktree_path,
        "adoption must keep the frozen advance worktree"
    );
    assert_eq!(
        store
            .list_attempts_for_issue("project_0001", "issue_0001")
            .expect("attempts for issue")
            .len(),
        1,
        "adoption must not create a second attempt"
    );
    let (replay_status, replay) = request_json(app, Method::POST, create_path, json!({})).await;
    assert_eq!(replay_status, StatusCode::OK, "{replay}");
    assert_eq!(replay["attempt_id"], journal.attempt.id);
}

#[tokio::test]
async fn create_group_coding_attempt_adopts_interrupted_advance_journal() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let journal = seed_advance_group_initialization(
        &app_paths,
        repo.path(),
        CodingGroupInitializationPhase::AttemptPersisted,
    );
    assert_eq!(
        journal.phase,
        CodingGroupInitializationPhase::AttemptPersisted
    );
    let create_path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";

    let (status, body) = request_json(app, Method::POST, create_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "create must resume the advance journal: {body}");
    assert_eq!(body["attempt_id"], journal.attempt.id);

    let store = CodingAttemptStore::new(app_paths.clone());
    let completed = store
        .get_group_initialization("project_0001", "issue_0001", "work_item_plan_0001")
        .expect("completed journal");
    assert_eq!(completed.phase, CodingGroupInitializationPhase::Completed);
    assert_eq!(completed.attempt.id, journal.attempt.id);
    assert_eq!(completed.attempt.admission_kind, CodingAdmissionKind::ScAdvance);
    assert_eq!(
        completed.attempt.worktree_path,
        journal.attempt.worktree_path,
        "resumed attempt must keep the frozen advance worktree"
    );
    let units = store
        .list_coding_units("project_0001", "issue_0001", &journal.attempt.id)
        .expect("materialized units");
    assert_eq!(units.len(), completed.units.len());
    assert_eq!(units.len(), 2);
    let shared_worktree = LifecycleStore::new(app_paths)
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("shared worktree")
        .expect("shared worktree");
    assert_eq!(
        shared_worktree.current_lock_owner_id.as_deref(),
        Some(journal.attempt.id.as_str()),
        "resumed attempt must own the shared worktree lease"
    );
}
