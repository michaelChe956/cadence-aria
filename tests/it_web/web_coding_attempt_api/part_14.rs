use cadence_aria::product::lifecycle_store::UpsertIssueSharedWorktreeInput;
use cadence_aria::product::models::WorkItemRuntimeBinding;
use cadence_aria::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
use cadence_aria::product::entity_operation_store::{
    EntityOperationError, EntityOperationStore, EntityRunLease,
};
use cadence_aria::product::issue_automation_store::IssueAutomationStore;
use cadence_aria::web::workspace_session::WorkspaceSessionManager;
use cadence_aria::product::workspace_engine::ProviderRunKind;

use tokio_util::sync::CancellationToken;


// Task 3 端到端测试：schema v2 work item group 删除的健壮化。
//
// 覆盖 spec `harden-work-item-group-deletion` 四组 requirement：
//   - 门禁拒绝（存在 coding workspace 时返回 coding_workspace_exists）
//   - 删除无残留（完整 group 与半残 group 均一次性删净）
//   - 不得误伤（issue / story-spec / design-spec / spec 版本 / 仓库初始化保留）
//   - 错误透明（由 Task 2 单测覆盖，这里不重复）

const GROUP_PROJECT_ID: &str = "project_0001";
const GROUP_ISSUE_ID: &str = "issue_0001";
const GROUP_PLAN_ID: &str = "work_item_plan_0001";

fn group_plan_uri() -> &'static str {
    // D2.2 迁移：本文件删除用例均为 Confirmed plan 夹具，REQ-DEL-04 下
    // 须显式确认一次（coding 门禁用例不受影响——门禁先于分类）。
    "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001?confirmed=true"
}

fn group_attempt_uri() -> &'static str {
    "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts"
}

fn group_issue_root(app_paths: &ProductAppPaths) -> PathBuf {
    app_paths.issue_root(GROUP_PROJECT_ID, GROUP_ISSUE_ID)
}

/// 播种一个绑定到本 plan 的 WorkItem 类型 session（含 runtime binding）。
fn seed_bound_work_item_session(
    lifecycle: &LifecycleStore,
    work_item_id: &str,
    work_item_revision_id: &str,
) {
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: GROUP_PROJECT_ID.to_string(),
            issue_id: GROUP_ISSUE_ID.to_string(),
            entity_id: work_item_id.to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Fake),

            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("create work item session");
    let binding = WorkItemRuntimeBinding {
        plan_id: GROUP_PLAN_ID.to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        logical_work_item_id: work_item_id.to_string(),
        work_item_revision_id: work_item_revision_id.to_string(),
        projection_bundle_id: format!("projection_{work_item_revision_id}"),
        verification_plan_revision_id: format!("verification_{work_item_revision_id}"),
        canonical_contract_hash: "sha256:contract".to_string(),
        projection_compiler_version: "projection-compiler-v1".to_string(),
        human_projection_hash: "sha256:human".to_string(),
        coder_projection_hash: "sha256:coder".to_string(),
        reviewer_projection_hash: "sha256:reviewer".to_string(),
    };
    lifecycle
        .ensure_work_item_runtime_binding(&session.id, &binding)
        .expect("bind work item session");
}

/// 播种 issue 级 shared worktree 记录（删除路径必须清理）。
fn seed_issue_shared_worktree_fixture(lifecycle: &LifecycleStore) {
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: GROUP_PROJECT_ID.to_string(),
            issue_id: GROUP_ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: PathBuf::from("/tmp/aria-issue-issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("seed issue shared worktree");
}

/// 播种仓维 shared worktree 记录（Logical 路由下 group attempt 创建与删除
/// 清理的目标布局；issue 维 legacy 文件在该路由被组入口 preflight
/// legacy_shared_worktree_present 422 拒绝）。
fn seed_repo_shared_worktree_fixture(app_paths: &ProductAppPaths) {
    let logical_id = LogicalCodebaseStore::new(app_paths.clone())
        .load_manifest(GROUP_PROJECT_ID)
        .expect("load logical manifest")
        .expect("logical manifest")
        .member_ids[0];
    LifecycleStore::new(app_paths.clone())
        .upsert_repo_shared_worktree(
            cadence_aria::product::lifecycle_store::UpsertRepoSharedWorktreeInput {
                project_id: GROUP_PROJECT_ID.to_string(),
                issue_id: GROUP_ISSUE_ID.to_string(),
                repository_id: logical_id,
                branch_name: "aria/issues/issue_0001".to_string(),
                worktree_path: PathBuf::from("/tmp/aria-issue-issue_0001"),
                base_branch: "main".to_string(),
            },
        )
        .expect("seed repo shared worktree");
}

/// 播种 plan store 的 outline context index（plan 删除需连带清理，避免孤儿）。
fn seed_plan_outline_context_index(app_paths: &ProductAppPaths) {
    use cadence_aria::product::models::{DesignContextCapabilities, OutlineContextIndex};
    use cadence_aria::product::work_item_plan_store::WorkItemPlanStore;
    WorkItemPlanStore::new(app_paths.clone())
        .save_outline_context_index(&OutlineContextIndex {
            project_id: GROUP_PROJECT_ID.to_string(),
            issue_id: GROUP_ISSUE_ID.to_string(),
            plan_id: GROUP_PLAN_ID.to_string(),
            generation_round_id: "outline_stage".to_string(),
            blocker_resolutions: Vec::new(),
            design_context_gaps: vec!["missing_architecture".to_string()],
            design_context_capabilities: DesignContextCapabilities {
                has_architecture: false,
                has_module_breakdown: false,
                has_tech_stack: false,
                has_test_strategy: false,
                has_key_paths: false,
            },
            updated_at: "2026-07-29T00:00:00Z".to_string(),
        })
        .expect("seed outline context index");
}

/// 断言该 plan 的全部产物已被清理（spec「删除无残留」）。
fn assert_plan_artifacts_purged(app_paths: &ProductAppPaths, attempt_id: Option<&str>) {
    let root = group_issue_root(app_paths);
    let lifecycle_root = app_paths.issue_lifecycle_root(GROUP_PROJECT_ID, GROUP_ISSUE_ID);

    // revisions + publications 整目录
    assert!(
        !root
            .join("work-item-revisions")
            .join(GROUP_PLAN_ID)
            .exists(),
        "work-item-revisions/<plan> must be purged"
    );
    assert!(
        !root
            .join("work-item-revision-publications")
            .join(GROUP_PLAN_ID)
            .exists(),
        "work-item-revision-publications/<plan> must be purged"
    );

    // plan store drafts/compiles/outlines
    for orphan in [
        format!("work_item_plan_outlines/{GROUP_PLAN_ID}"),
        format!("work_item_plan_drafts/{GROUP_PLAN_ID}"),
        format!("work_item_plan_compiles/{GROUP_PLAN_ID}"),
    ] {
        assert!(
            !root.join(&orphan).exists(),
            "plan store artifact must be purged: {orphan}"
        );
    }

    // issue shared worktree json + lock
    assert!(
        !lifecycle_root.join("issue-shared-worktree.json").exists(),
        "issue-shared-worktree.json must be deleted"
    );
    assert!(
        !lifecycle_root
            .join(".issue-shared-worktree.json.lock")
            .exists(),
        "issue-shared-worktree.json.lock must be deleted"
    );

    // coding-attempts 残留：attempt json/dir、journal、arbitration、work-item-attempt-locks
    let coding_root = lifecycle_root.join("coding-attempts");
    if let Some(attempt_id) = attempt_id {
        assert!(
            !coding_root.join(format!("{attempt_id}.json")).exists(),
            "attempt json residue must be purged"
        );
        assert!(
            !coding_root.join(attempt_id).exists(),
            "attempt dir residue must be purged"
        );
    }
    assert!(
        !coding_root
            .join("group-initializations")
            .join(format!("{GROUP_PLAN_ID}.json"))
            .exists(),
        "group initialization journal must be purged"
    );
    // journal 的 flock sidecar：journal 已删则 sidecar 也必须清掉（孤儿锁不留）。
    assert!(
        !coding_root
            .join("group-initializations")
            .join(format!(".{GROUP_PLAN_ID}.json.lock"))
            .exists(),
        "group initialization journal lock must be purged"
    );
    // group 初始化仲裁文件与公共锁：半残现场（内容不可解析）同样必须清掉。
    assert!(
        !coding_root
            .join("group-initialization-arbitration")
            .exists(),
        "group initialization arbitration residue must be purged"
    );
    assert!(
        !coding_root
            .join(".group-initialization-arbitration.lock")
            .exists(),
        "group initialization arbitration lock must be purged"
    );
    // work-item-attempt-locks 按 plan 的 work_item 精确清理：本 plan 的锁必须消失，
    // 但共享目录里其他 plan 的 work_item 锁不能被误伤（见不误伤专项测试）。
    let locks_dir = coding_root.join("work-item-attempt-locks");
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        assert!(
            !locks_dir.join(work_item_id).exists(),
            "work item attempt lock for {work_item_id} must be purged"
        );
        assert!(
            !locks_dir.join(format!(".{work_item_id}.lock")).exists(),
            "work item attempt lock sidecar for {work_item_id} must be purged"
        );
    }

    // sessions：WorkItem 与 WorkItemPlan 类型均不再能 list 到本 plan 的记录
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let sessions = lifecycle
        .list_workspace_sessions(GROUP_PROJECT_ID, GROUP_ISSUE_ID)
        .expect("list sessions after delete");
    let work_item_left = sessions
        .iter()
        .filter(|session| {
            session.workspace_type == WorkspaceType::WorkItem
                && session
                    .work_item_runtime_binding
                    .as_ref()
                    .is_some_and(|binding| binding.plan_id == GROUP_PLAN_ID)
        })
        .count();
    assert_eq!(
        work_item_left, 0,
        "WorkItem sessions bound to plan must be deleted"
    );
    let plan_session_left = sessions
        .iter()
        .filter(|session| {
            session.workspace_type == WorkspaceType::WorkItemPlan
                && session.entity_id == GROUP_PLAN_ID
        })
        .count();
    assert_eq!(plan_session_left, 0, "WorkItemPlan session must be deleted");

    // plan 记录本身
    assert!(
        lifecycle
            .get_issue_work_item_plan(GROUP_PROJECT_ID, GROUP_ISSUE_ID, GROUP_PLAN_ID)
            .is_err(),
        "issue work item plan record must be deleted"
    );
}

/// 断言 issue 级与项目级「不应被删除」的资源仍在（spec「不得误伤」）。
fn assert_issue_and_specs_preserved(app_paths: &ProductAppPaths) {
    let root = group_issue_root(app_paths);
    assert!(
        root.join("issue.json").exists(),
        "issue.json must be preserved"
    );
    assert!(
        root.join("story-specs").exists(),
        "story-specs dir must be preserved"
    );
    assert!(
        root.join("design-specs").exists(),
        "design-specs dir must be preserved"
    );
    assert!(
        app_paths
            .repository_initializations_root(GROUP_PROJECT_ID)
            .exists(),
        "repository-initializations dir must be preserved"
    );
    let lifecycle = LifecycleStore::new(app_paths.clone());
    assert!(
        !lifecycle
            .list_story_specs(GROUP_PROJECT_ID, GROUP_ISSUE_ID)
            .expect("list story specs")
            .is_empty(),
        "story specs must be preserved"
    );
    assert!(
        !lifecycle
            .list_design_specs(GROUP_PROJECT_ID, GROUP_ISSUE_ID)
            .expect("list design specs")
            .is_empty(),
        "design specs must be preserved"
    );
}

#[tokio::test]
async fn delete_work_item_plan_rejected_when_coding_workspace_exists() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    // 没有额外 WorkItem session 也能进入删除路径；关键是存在 group coding attempt。
    let (create_status, created) =
        request_json(app.clone(), Method::POST, group_attempt_uri(), json!({})).await;
    assert_eq!(
        create_status,
        StatusCode::OK,
        "group attempt create: {created}"
    );
    let attempt_id = assert_global_attempt_id(&created);
    // bootstrap 不创建 issue-shared-worktree，但 group attempt 创建会写入；先记录删除前状态。
    assert!(
        lifecycle
            .get_issue_work_item_plan(GROUP_PROJECT_ID, GROUP_ISSUE_ID, GROUP_PLAN_ID)
            .is_ok(),
        "plan must exist before delete"
    );

    let (status, body) = request_json(app, Method::DELETE, group_plan_uri(), json!({})).await;
    // 门禁必须拒绝，且使用 409 CONFLICT 表达「资源冲突」。
    assert_eq!(status, StatusCode::CONFLICT, "delete body: {body}");
    assert_eq!(body["code"], "coding_workspace_exists");
    assert_eq!(body["details"]["plan_id"], GROUP_PLAN_ID);
    assert_eq!(body["details"]["attempt_id"], attempt_id);

    // 拒绝时 group 与 attempt 都必须原样保留。
    assert!(
        lifecycle
            .get_issue_work_item_plan(GROUP_PROJECT_ID, GROUP_ISSUE_ID, GROUP_PLAN_ID)
            .is_ok(),
        "plan record must remain after rejection"
    );
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    assert!(
        coding_store
            .get_attempt(GROUP_PROJECT_ID, GROUP_ISSUE_ID, &attempt_id)
            .is_ok(),
        "attempt record must remain after rejection"
    );
}

#[tokio::test]
async fn delete_work_item_plan_removes_all_artifacts_and_preserves_issue() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    seed_bound_work_item_session(&lifecycle, "work_item_0001", "work_item_revision_0001");
    seed_bound_work_item_session(&lifecycle, "work_item_0002", "work_item_revision_0002");
    seed_issue_shared_worktree_fixture(&lifecycle);
    seed_plan_outline_context_index(&app_paths);

    let (status, body) = request_json(app, Method::DELETE, group_plan_uri(), json!({})).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");
    assert_eq!(body["status"], "deleted");

    assert_plan_artifacts_purged(&app_paths, None);
    assert_issue_and_specs_preserved(&app_paths);
}

#[tokio::test]
async fn delete_work_item_plan_succeeds_on_half_deleted_state() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    seed_bound_work_item_session(&lifecycle, "work_item_0001", "work_item_revision_0001");
    seed_bound_work_item_session(&lifecycle, "work_item_0002", "work_item_revision_0002");
    seed_repo_shared_worktree_fixture(&app_paths);
    seed_plan_outline_context_index(&app_paths);

    // 模拟线上「半残」：先创建 group coding attempt，再手动抹掉 attempt 主体但留下锁，
    // 同时破坏部分 WorkItem session 与 worktree 目录，制造「revision bindings 不可靠」。
    let (create_status, created) =
        request_json(app.clone(), Method::POST, group_attempt_uri(), json!({})).await;
    assert_eq!(
        create_status,
        StatusCode::OK,
        "group attempt create: {created}"
    );
    let attempt_id = assert_global_attempt_id(&created);
    let attempt = prepare_attempt_with_worktree(
        &CodingAttemptStore::new(app_paths.clone()),
        repo.path(),
        GROUP_PROJECT_ID,
        GROUP_ISSUE_ID,
        &attempt_id,
    );

    let lifecycle_root = app_paths.issue_lifecycle_root(GROUP_PROJECT_ID, GROUP_ISSUE_ID);
    let coding_root = lifecycle_root.join("coding-attempts");
    let attempt_json = coding_root.join(format!("{attempt_id}.json"));
    let attempt_dir = coding_root.join(&attempt_id);
    // 半残构造 1：attempt 主体 json 被删（门禁应放行）；手动模拟残留的 attempt lock 文件。
    fs::remove_file(&attempt_json).expect("remove attempt json");
    let attempt_lock = coding_root.join(format!(".{attempt_id}.json.lock"));
    fs::write(&attempt_lock, "").expect("seed residue attempt lock");
    // 半残构造 2：attempt 自身产物目录（units/rework 等）被外部清掉。
    if attempt_dir.exists() {
        fs::remove_dir_all(&attempt_dir).expect("remove attempt dir");
    }
    // 半残构造 3：worktree 目录已被运维或外部进程删除。
    if let Some(worktree_path) = attempt.worktree_path.as_ref()
        && worktree_path.exists()
    {
        fs::remove_dir_all(worktree_path).expect("remove worktree dir");
    }
    // 半残构造 4：丢掉一个 WorkItem session 的 json 文件，制造 bindings 数量不匹配。
    let sessions_dir = lifecycle_root.join("workspace-sessions");
    if let Ok(entries) = fs::read_dir(&sessions_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(text) = fs::read_to_string(&path)
                && text.contains("\"entity_id\":\"work_item_0002\"")
            {
                fs::remove_file(&path).expect("remove one work item session");
                break;
            }
        }
    }
    // 半残构造 5：模拟 group attempt 初始化阶段残留的仲裁文件、journal 与 single-attempt 锁。
    // 这些产物在 attempt 主体已删后成为孤儿，删除路径必须一并清理。
    fs::write(coding_root.join("group-initialization-arbitration"), "{}")
        .expect("seed arbitration residue");
    fs::write(
        coding_root.join(".group-initialization-arbitration.lock"),
        "",
    )
    .expect("seed arbitration lock residue");
    let journal_dir = coding_root.join("group-initializations");
    fs::create_dir_all(&journal_dir).expect("journal dir");
    fs::write(journal_dir.join(format!("{GROUP_PLAN_ID}.json")), "{}")
        .expect("seed journal residue");
    fs::write(journal_dir.join(format!(".{GROUP_PLAN_ID}.json.lock")), "")
        .expect("seed journal lock residue");
    let attempt_locks_dir = coding_root.join("work-item-attempt-locks");
    fs::create_dir_all(&attempt_locks_dir).expect("attempt locks dir");
    fs::write(attempt_locks_dir.join("work_item_0001"), "{}")
        .expect("seed work item attempt lock residue");

    let (status, body) = request_json(app, Method::DELETE, group_plan_uri(), json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "half-deleted group must still delete cleanly: {body}"
    );
    assert_eq!(body["status"], "deleted");

    // 仓维 shared worktree 记录随 plan 删除一并清理（REQ-COD-03 §4.2.4）。
    let logical_id = LogicalCodebaseStore::new(app_paths.clone())
        .load_manifest(GROUP_PROJECT_ID)
        .expect("load logical manifest")
        .expect("logical manifest")
        .member_ids[0];
    assert!(
        LifecycleStore::new(app_paths.clone())
            .get_repo_shared_worktree(GROUP_PROJECT_ID, GROUP_ISSUE_ID, logical_id)
            .expect("read repo shared worktree after plan delete")
            .is_none(),
        "repo shared-worktrees record must be purged with the plan"
    );

    assert_plan_artifacts_purged(&app_paths, Some(&attempt_id));
    assert_issue_and_specs_preserved(&app_paths);
}

#[tokio::test]
async fn delete_work_item_plan_preserves_other_work_items_attempt_locks() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));

    // 共享的 issue 级 work-item-attempt-locks 目录里混入「另一个 plan 的 work_item」锁。
    // 真实场景：多 plan 共用同一 issue 的 coding-attempts 目录，per-work-item 锁按 work_item_id 命名。
    let lifecycle_root = app_paths.issue_lifecycle_root(GROUP_PROJECT_ID, GROUP_ISSUE_ID);
    let locks_dir = lifecycle_root
        .join("coding-attempts")
        .join("work-item-attempt-locks");
    fs::create_dir_all(&locks_dir).expect("attempt locks dir");
    // 本 plan 的锁（删除时应被精确清掉）
    fs::write(locks_dir.join("work_item_0001"), "{}").expect("seed own lock");
    fs::write(locks_dir.join(".work_item_0001.lock"), "").expect("seed own sidecar");
    // 不属于本 plan 的锁（删除后必须仍在——spec「删除不影响其他 plan」）
    fs::write(locks_dir.join("other_work_item"), "{}").expect("seed other plan lock");
    fs::write(locks_dir.join(".other_work_item.lock"), "").expect("seed other plan sidecar");

    let (status, body) = request_json(app, Method::DELETE, group_plan_uri(), json!({})).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");
    assert_eq!(body["status"], "deleted");

    // 本 plan 的锁被清掉。
    assert!(
        !locks_dir.join("work_item_0001").exists(),
        "own work item lock must be purged"
    );
    assert!(
        !locks_dir.join(".work_item_0001.lock").exists(),
        "own work item lock sidecar must be purged"
    );
    // 其他 plan 的锁必须原样保留——这是 Task 3 review 发现的 spec 风险的核心断言。
    assert!(
        locks_dir.join("other_work_item").exists(),
        "other plan work item lock must be preserved (spec: 删除不得误伤其他 plan)"
    );
    assert!(
        locks_dir.join(".other_work_item.lock").exists(),
        "other plan work item lock sidecar must be preserved"
    );
}

#[tokio::test]
async fn delete_work_item_plan_top_level_attempt_locks_purge_orphans_only() {
    // Task 5 端到端测试：顶层 `.*.lock` 只清理孤儿锁，保留 active attempt 的运行时锁。
    //
    // 真实场景：多 plan 共 issue 时，coding-attempts 顶层目录里其他 plan 可能有 active
    // coding attempt（json + 运行时 lock 同时存在）。整删顶层 `.*.lock` 会误删其锁，
    // 违反 spec「删除不得误伤其他 plan」。修正后只删对应 json 已不存在的孤儿锁。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));

    // 先创建一个真实 group attempt，拿到合法的 CodingExecutionAttempt JSON 主体
    // （直接写 `{}` 会被门禁的 list_json_records 解析失败，必须用真实记录）。
    let (create_status, created) =
        request_json(app.clone(), Method::POST, group_attempt_uri(), json!({})).await;
    assert_eq!(
        create_status,
        StatusCode::OK,
        "group attempt create: {created}"
    );
    let attempt_id = assert_global_attempt_id(&created);
    let attempt = prepare_attempt_with_worktree(
        &CodingAttemptStore::new(app_paths.clone()),
        repo.path(),
        GROUP_PROJECT_ID,
        GROUP_ISSUE_ID,
        &attempt_id,
    );

    let lifecycle_root = app_paths.issue_lifecycle_root(GROUP_PROJECT_ID, GROUP_ISSUE_ID);
    let coding_root = lifecycle_root.join("coding-attempts");
    let original_json = coding_root.join(format!("{attempt_id}.json"));

    // 改写为「另一个 plan 的 active attempt」：换 id + 换 work_item_group_id。
    // 门禁按 work_item_group_id == plan_id 过滤，会跳过这条记录从而放行本 plan 删除。
    // json 合法且存在 → 其运行时锁不可误删（spec「删除不得误伤其他 plan」）。
    let mut record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&original_json).expect("read attempt json"))
            .expect("parse attempt json");
    record["id"] = json!("other_plan_active_attempt");
    record["work_item_group_id"] = json!("work_item_plan_other");
    let other_json = coding_root.join("other_plan_active_attempt.json");
    fs::write(&other_json, record.to_string()).expect("seed other plan active attempt json");
    let active_lock = coding_root.join(".other_plan_active_attempt.json.lock");
    fs::write(&active_lock, "").expect("seed other plan active attempt runtime lock");

    // 孤儿锁：对应 json 已不存在（attempt 主体被删后留下的残锁，应清理）。
    let orphan_lock = coding_root.join(".ghost_attempt.json.lock");
    fs::write(&orphan_lock, "").expect("seed orphan attempt lock");

    // 删掉本 group 的真实 attempt 主体（门禁放行），并清掉 worktree 目录避免外部状态残留。
    fs::remove_file(&original_json).expect("remove original attempt json");
    let attempt_dir = coding_root.join(&attempt_id);
    if attempt_dir.exists() {
        fs::remove_dir_all(&attempt_dir).expect("remove original attempt dir");
    }
    if let Some(worktree_path) = attempt.worktree_path.as_ref().filter(|p| p.exists()) {
        fs::remove_dir_all(worktree_path).expect("remove original attempt worktree");
    }

    let (status, body) = request_json(app, Method::DELETE, group_plan_uri(), json!({})).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");
    assert_eq!(body["status"], "deleted");

    // active attempt 的 json 与运行时锁都必须保留——spec「删除不得误伤其他 plan」。
    assert!(
        other_json.exists(),
        "other plan active attempt json must be untouched"
    );
    assert!(
        active_lock.exists(),
        "other plan active attempt runtime lock must be preserved while its json exists"
    );
    // 孤儿锁必须被清理。
    assert!(
        !orphan_lock.exists(),
        "orphan attempt lock (no corresponding json) must be purged"
    );
}

// ---- REQ-WIGA-15 场景 1：删除工作项组先取消该实体在途 run 再取删除独占 ----

/// §1.5「可控假 run」：复刻生产语义——run 任务在跑期间持 scope **共享** flock
/// （`acquire_run`），被取消后先释放共享锁、再按 token 发**真实 completion**
/// （`finish_run`）。不依赖真实 provider、不靠 sleep 撞竞速；Task 2/3/6 复用。
struct FakeInflightRun {
    cancel_token: CancellationToken,
    completion_handle: tokio::task::JoinHandle<()>,
    shared_lease: std::sync::Arc<std::sync::Mutex<Option<EntityRunLease>>>,
}

impl FakeInflightRun {
    /// 手动释放 scope 共享 flock（幂等；本用例中假 run 任务被取消后自行
    /// 释放，Task 3 时序尾隙用例以显式调用倒转释放顺序）。
    /// 本 Task 尚无调用方（Task 2/3/6 复用），dead_code 显式豁免。
    #[allow(dead_code)]
    fn release_scope_lock(&self) {
        if let Some(lease) = self.shared_lease.lock().unwrap().take() {
            drop(lease);
        }
    }
}

/// 起一个确定性的「假 in-flight run」：先 `start_run` 登记 manager 活跃 run，
/// 再由调用侧同步取得 scope 共享 flock（返回时锁已确定在手），最后 spawn
/// 假 run 任务：等取消 → drop lease 释放 flock → `finish_run` 发真实 completion。
async fn spawn_fake_inflight_run(
    app_paths: &ProductAppPaths,
    manager: std::sync::Arc<WorkspaceSessionManager>,
    scope: EntityScope,
) -> FakeInflightRun {
    let (_run_id, token, cancel_token, _cmd_rx, _node_id) = manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("start fake in-flight run");
    let lease = EntityOperationStore::new(app_paths.clone())
        .acquire_run(&scope)
        .expect("fake run acquires scope shared lock");
    let shared_lease = std::sync::Arc::new(std::sync::Mutex::new(Some(lease)));
    let task_lease = shared_lease.clone();
    let cancel_in_task = cancel_token.clone();
    let completion_handle = tokio::spawn(async move {
        cancel_in_task.cancelled().await;
        if let Some(lease) = task_lease.lock().unwrap().take() {
            drop(lease);
        }
        manager.finish_run(token).await;
    });
    FakeInflightRun {
        cancel_token,
        completion_handle,
        shared_lease,
    }
}

#[tokio::test]
async fn delete_work_item_plan_cancels_inflight_run_before_delete_exclusive() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());

    // ①/② 绑定创建 entity_id == work_item_plan_0001 的 WorkItemPlan 会话。
    let session = lifecycle
        .create_workspace_session_bound(
            CreateWorkspaceSessionInput {
                project_id: GROUP_PROJECT_ID.to_string(),
                issue_id: GROUP_ISSUE_ID.to_string(),
                entity_id: GROUP_PLAN_ID.to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
            "workspace_session_9101".to_string(),
        )
        .expect("create bound plan workspace session");

    // ③ manager 挂进 registry（先例 web_lifecycle_api/part_05.rs:373-383）。
    let factory_state = state.clone();
    let factory_session_id = session.id.clone();
    state
        .workspace_sessions
        .get_or_create(&session.id, move || {
            let state = factory_state.clone();
            let session_id = factory_session_id.clone();
            async move { WorkspaceSessionManager::create(&state, &session_id).await }
        })
        .await
        .expect("plan manager attached");
    let manager = state
        .workspace_sessions
        .get(&session.id)
        .await
        .expect("plan manager in registry");

    // ④ 假 in-flight run：持 scope 共享 flock 直到被取消收口。
    let scope = EntityScope::new(
        GROUP_PROJECT_ID,
        GROUP_ISSUE_ID,
        DeletionEntityKind::Plan,
        GROUP_PLAN_ID,
    );
    let fake = spawn_fake_inflight_run(&app_paths, manager, scope.clone()).await;

    // 删除前事实快照：假 run 登记不等于启动 provider——actual-launch 单源
    // provider_start_ledger 必须无 actual-launch 条目。
    let ledger_before = lifecycle
        .get_workspace_session(&session.id)
        .expect("read durable plan session before delete")
        .provider_start_ledger;
    assert!(
        ledger_before.iter().all(|entry| !entry.started),
        "假 run 不得启动 provider（provider_start_ledger 条目数 {}）",
        ledger_before.len()
    );

    // ⑤ DELETE：必须先取消在途 run 并等真实收口，再取删除独占。
    let (status, body) = request_json(app, Method::DELETE, group_plan_uri(), json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "删除必须先取消在途 run 再取独占（409 entity_busy 即缺本修法）；body: {body}"
    );
    assert_eq!(body["status"], "deleted");

    // 取消真的发生了：该 run 的 CancellationToken 已被触发，且假 run 已按
    // token 发出真实 completion（任务收口）。
    assert!(
        fake.cancel_token.is_cancelled(),
        "删除路径必须取消该实体在途 run 的 CancellationToken"
    );
    fake.completion_handle
        .await
        .expect("假 run 任务必须已收口（真实 completion）");

    // 共享 flock 已释放：删除后再取独占不得被 Busy 挡住（成功删除后墓碑
    // 稳定拒绝 Deleted，恰好证明独占锁可获得、共享锁已放）。
    let exclusive = EntityOperationStore::new(app_paths.clone()).try_acquire_delete(&scope);
    assert!(
        matches!(exclusive, Err(EntityOperationError::Deleted)),
        "scope 共享 flock 必须已释放（再取独占不得 Busy，实际 {:?}）",
        exclusive.err()
    );

    // 既有删除语义未被削弱：plan 不再可读。
    assert!(
        lifecycle
            .get_issue_work_item_plan(GROUP_PROJECT_ID, GROUP_ISSUE_ID, GROUP_PLAN_ID)
            .is_err(),
        "plan 必须已被删除"
    );

    // 取消没有偷偷再起一次 provider：durable session 的 provider_start_ledger
    // 无新增 actual-launch 条目（会话记录随级联删除后不可读 ⇒ 残留才复核）。
    let sessions_after = lifecycle
        .list_workspace_sessions(GROUP_PROJECT_ID, GROUP_ISSUE_ID)
        .expect("list sessions after delete");
    if let Some(record) = sessions_after.iter().find(|item| item.id == session.id) {
        assert!(
            record.provider_start_ledger.len() <= ledger_before.len()
                && record
                    .provider_start_ledger
                    .iter()
                    .all(|entry| !entry.started),
            "provider_start_ledger 不得新增 actual-launch 条目（{} 条）",
            record.provider_start_ledger.len()
        );
    }

    // enrollment / 命令账本无自相矛盾条目：删除后不得残留 enabled 且未终止
    // 的绑定登记（本夹具未播种 enrollment ⇒ Ok(None) 为无矛盾正常态）。
    match IssueAutomationStore::new(app_paths.clone()).get(GROUP_PROJECT_ID, GROUP_ISSUE_ID) {
        Ok(None) => {}
        Ok(Some(enrollment)) => {
            assert!(
                enrollment.terminated_at.is_some() || !enrollment.enabled,
                "删除后不得残留 enabled 且未终止的 enrollment：{}",
                enrollment.enrollment_id
            );
        }
        Err(error) => panic!("读取自动化登记失败：{error}"),
    }
}
