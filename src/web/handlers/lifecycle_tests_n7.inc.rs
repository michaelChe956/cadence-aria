// N7（REQ-CS-07，OpenSpec Task 3.2）：新 plan-run 删除闭包接线测试。经
// lifecycle.rs 的 tests mod include!（large_file_guard 1200 行红线拆分，
// 参照 lifecycle_recent_completion_tests.inc.rs 先例）。axum/Body/Request/
// StatusCode/ServiceExt/TempDir/Uuid/ProductAppPaths/LifecycleStore/
// LogicalRepositoryId/WorkItemRevisionStore/WorkItemPlanLineage/
// CreateProductIssueInput/ProjectStore/build_web_router/coding attempt 等
// 复用同 mod 内 lifecycle_tests.inc.rs 已有导入。

use crate::product::advance_store::AdvanceTargetAttemptBinding;
use crate::product::coding_plan_run::{
    CodingPlanRun, CodingPlanRunPhase, CodingPlanRunStore, CodingPlanSelection,
    PLAN_CODING_MODE_FIRST_READY_UNIT,
};
// ───────────────────────────────────────────────────────────────
// N7（REQ-CS-07）：新 plan-run 删除闭包接线测试
// ───────────────────────────────────────────────────────────────

const N7_PLAN_A: &str = "work_item_plan_0001";
const N7_PLAN_B: &str = "work_item_plan_0002";

/// 最小 project + issue 种子（N7 只关心 plan 删除闭包，不建 work item）。
fn n7_seed_project_issue(paths: &ProductAppPaths) -> LifecycleStore {
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "n7 deletion closure project".to_string(),
            description: None,
        })
        .expect("create project");
    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some(REPOSITORY_ID.to_string()),
            logical_codebase_id: None,
            title: "N7 删除闭包".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("create issue");
    assert_eq!(issue.id, ISSUE_ID);
    LifecycleStore::new(paths.clone())
}

fn n7_seed_plan(lifecycle: &LifecycleStore, plan_id: &str) {
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some(plan_id.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: crate::product::models::IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("seed plan");
}

/// group 绑定 attempt（work_item_group_id=plan）：plan 级门禁按它拒删。
fn n7_group_attempt(coding_store: &CodingAttemptStore, plan_id: &str) -> CodingExecutionAttempt {
    let created = coding_store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let mut attempt = created;
    attempt.work_item_group_id = Some(plan_id.to_string());
    coding_store
        .write_coding_attempt_for_test(&attempt)
        .expect("bind attempt to plan");
    attempt
}

/// 直写冻结地址 `{issue_root}/coding-plan-runs/{plan_id}.json` 的 run 记录。
fn n7_plan_run(
    plan_id: &str,
    targets: &[(LogicalRepositoryId, String)],
    selection_attempt: Option<&str>,
    phase: CodingPlanRunPhase,
) -> CodingPlanRun {
    let selection = selection_attempt.map(|attempt_id| CodingPlanSelection {
        unit_id: "unit_0001".to_string(),
        logical_work_item_id: "logical_work_item_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        target_repository_id: targets
            .first()
            .map(|(repository, _)| repository.0.to_string())
            .unwrap_or_default(),
        start_command_id: "plan_start_deterministic_0001".to_string(),
    });
    CodingPlanRun {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan_id.to_string(),
        command_id: "plan_command_0001".to_string(),
        bound_plan_revision_id: "plan_revision_0001".to_string(),
        dependency_graph_revision_id: "graph_revision_0001".to_string(),
        target_attempts: targets
            .iter()
            .map(|(repository, attempt_id)| AdvanceTargetAttemptBinding {
                target_repository_id: repository.0.to_string(),
                attempt_id: attempt_id.clone(),
            })
            .collect(),
        mode: PLAN_CODING_MODE_FIRST_READY_UNIT.to_string(),
        selection,
        phase,
        start_outcome: None,
        pause_reason: None,
        created_at: "2026-10-14T00:00:00Z".to_string(),
        updated_at: "2026-10-14T00:00:00Z".to_string(),
    }
}

fn n7_runs_root(paths: &ProductAppPaths) -> std::path::PathBuf {
    paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("coding-plan-runs")
}

fn n7_run_file(paths: &ProductAppPaths, plan_id: &str) -> std::path::PathBuf {
    n7_runs_root(paths).join(format!("{plan_id}.json"))
}

fn n7_run_lock(paths: &ProductAppPaths, plan_id: &str) -> std::path::PathBuf {
    n7_runs_root(paths).join(format!(".{plan_id}.json.lock"))
}

fn n7_write_run(paths: &ProductAppPaths, run: &CodingPlanRun) {
    std::fs::create_dir_all(n7_runs_root(paths)).expect("create runs root");
    crate::product::json_store::write_json(&n7_run_file(paths, &run.plan_id), run)
        .expect("seed plan run");
}

fn n7_repo_worktree_input(
    repository_id: LogicalRepositoryId,
    worktree_path: &str,
) -> UpsertRepoSharedWorktreeInput {
    UpsertRepoSharedWorktreeInput {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        repository_id,
        branch_name: format!("worktree-{}", repository_id.0),
        worktree_path: std::path::PathBuf::from(worktree_path),
        base_branch: "main".to_string(),
    }
}

/// 直接改写仓维 worktree 记录，模拟另一 plan attempt 持有活动锁。
fn n7_set_repo_worktree_owner(
    paths: &ProductAppPaths,
    repository_id: LogicalRepositoryId,
    owner_attempt_id: &str,
) {
    let path = paths
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("shared-worktrees")
        .join(format!("{}.json", repository_id.0));
    let mut record: crate::product::models::IssueSharedWorktree =
        crate::product::json_store::read_json(&path).expect("read repo worktree");
    record.current_active_work_item_id = Some("work_item_0009".to_string());
    record.current_lock_owner_id = Some(owner_attempt_id.to_string());
    record.status = crate::product::models::IssueSharedWorktreeStatus::Running;
    crate::product::json_store::write_json(&path, &record).expect("seed other owner");
}

async fn delete_work_item_plan_request(
    app: &axum::Router,
    plan_id: &str,
) -> (StatusCode, serde_json::Value) {
    // D2.2 迁移：n7_seed_plan 以 Confirmed 落档，REQ-DEL-04 下须显式确认一次。
    let uri = format!(
        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/work-item-plans/{plan_id}?confirmed=true"
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("send delete plan");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read delete body");
    let value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, value)
}

/// 有 coding workspace 时拒删 plan：run/plan/attempt 全部零改动（REQ-CS-07
/// 「先删 workspace 的门禁 MUST 保留」）。基线保护（门禁既有，防回归）。
#[tokio::test]
async fn plan_coding_n7_workspace_gate_zero_mutation() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = n7_seed_project_issue(&paths);
    n7_seed_plan(&lifecycle, N7_PLAN_A);
    let coding_store = CodingAttemptStore::new(paths.clone());
    let attempt = n7_group_attempt(&coding_store, N7_PLAN_A);
    let run = n7_plan_run(
        N7_PLAN_A,
        &[(LogicalRepositoryId(Uuid::new_v4()), attempt.id.clone())],
        Some(&attempt.id),
        CodingPlanRunPhase::StartRequested,
    );
    n7_write_run(&paths, &run);
    let run_before = std::fs::read(n7_run_file(&paths, N7_PLAN_A)).expect("run seeded");

    let (status, body) = delete_work_item_plan_request(&app, N7_PLAN_A).await;

    assert_eq!(status, StatusCode::CONFLICT, "gate must reject: {body}");
    assert_eq!(body["code"], "coding_workspace_exists");
    // run/plan/attempt 零改动。
    let run_after = std::fs::read(n7_run_file(&paths, N7_PLAN_A)).expect("run untouched");
    assert_eq!(
        run_before, run_after,
        "拒绝路径不得动 run 记录（早拒不 purge/不失效）"
    );
    assert!(
        lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, N7_PLAN_A)
            .is_ok(),
        "plan record must remain after rejection"
    );
    assert!(
        coding_store
            .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
            .is_ok(),
        "attempt record must remain after rejection"
    );
}

/// 先删本 workspaces 再删 plan（schema-v2 闭包）：本 plan run 文件+前导点锁清、
/// 冻结 target 资源精确清理；他 Plan run / 他 target / 他人持锁 owner 不误删。
#[tokio::test]
async fn plan_coding_n7_plan_purge_removes_run_and_preserves_other_owner() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = n7_seed_project_issue(&paths);
    n7_seed_plan(&lifecycle, N7_PLAN_A);
    n7_seed_plan(&lifecycle, N7_PLAN_B);
    // plan A 走 schema-v2 清理闭包。
    WorkItemRevisionStore::new(paths.clone())
        .put_plan_lineage(&WorkItemPlanLineage {
            id: N7_PLAN_A.to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_refs: Vec::new(),
            design_spec_refs: Vec::new(),
            active_revision_id: None,
            active_amendment_id: None,
            created_at: "2026-10-14T00:00:00Z".to_string(),
            updated_at: "2026-10-14T00:00:00Z".to_string(),
        })
        .expect("seed plan lineage");

    let repo_a = LogicalRepositoryId(Uuid::new_v4());
    let repo_b = LogicalRepositoryId(Uuid::new_v4());
    let repo_c = LogicalRepositoryId(Uuid::new_v4());
    lifecycle
        .upsert_repo_shared_worktree(n7_repo_worktree_input(repo_a, "/tmp/n7-wt-a"))
        .expect("seed repo a worktree");
    lifecycle
        .upsert_repo_shared_worktree(n7_repo_worktree_input(repo_b, "/tmp/n7-wt-b"))
        .expect("seed repo b worktree");
    lifecycle
        .upsert_repo_shared_worktree(n7_repo_worktree_input(repo_c, "/tmp/n7-wt-c"))
        .expect("seed repo c worktree");
    // repo_c 在冻结集内但被其他 plan 的 attempt 持有活动锁（真实 owner 复核）。
    n7_set_repo_worktree_owner(&paths, repo_c, "other_plan_attempt_0001");

    let coding_store = CodingAttemptStore::new(paths.clone());
    let attempt_a = n7_group_attempt(&coding_store, N7_PLAN_A);
    let run_a = n7_plan_run(
        N7_PLAN_A,
        &[
            (repo_a, attempt_a.id.clone()),
            (repo_c, "attempt_c_of_plan_a".to_string()),
        ],
        None,
        CodingPlanRunPhase::ReadyBound,
    );
    n7_write_run(&paths, &run_a);
    let run_b = n7_plan_run(
        N7_PLAN_B,
        &[(repo_b, "attempt_b_of_plan_b".to_string())],
        None,
        CodingPlanRunPhase::Prepared,
    );
    n7_write_run(&paths, &run_b);
    // 前导点锁残留（安全删除窗口必须一并清）。
    std::fs::write(n7_run_lock(&paths, N7_PLAN_A), b"").expect("seed run lock");

    // 门禁顺序：先删本 run 绑定的 coding workspace，再删 plan（invalidate 是
    // run 写准入——必须在墓碑立起之前完成）。
    finalize_coding_attempt_deletion(&coding_store, &paths, &attempt_a)
        .expect("delete member workspace first");

    // N2 修复轮语义：purge 的锁清理只在删除窗口执行——Plan 墓碑在位
    //（真实删除闭包形态：D1.2c 级联持 lease begin 后重入本删除）时
    // `purge_plan` 才清前导点锁。
    let scope = crate::product::entity_deletion_store::EntityScope::new(
        PROJECT_ID,
        ISSUE_ID,
        crate::product::entity_deletion_store::DeletionEntityKind::Plan,
        N7_PLAN_A,
    );
    let lease = crate::product::entity_operation_store::EntityOperationStore::new(paths.clone())
        .try_acquire_delete(&scope)
        .expect("acquire plan delete lease");
    crate::product::entity_deletion_store::EntityDeletionStore::new(paths.clone())
        .begin(&lease)
        .expect("begin plan deletion tombstone");
    drop(lease);
    let (status, body) = delete_work_item_plan_request(&app, N7_PLAN_A).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");
    assert_eq!(body["status"], "deleted");

    // 本 plan run 文件 + 前导点安全锁清。
    assert!(
        !n7_run_file(&paths, N7_PLAN_A).exists(),
        "plan run file must be purged with plan deletion"
    );
    assert!(
        !n7_run_lock(&paths, N7_PLAN_A).exists(),
        "plan run leading-dot lock must be purged in the safe deletion window"
    );
    // 他 plan run 保留（不整目录清空）。
    assert!(
        n7_run_file(&paths, N7_PLAN_B).exists(),
        "other plan run must be preserved"
    );
    let kept_run_b = CodingPlanRunStore::new(paths.clone())
        .get(PROJECT_ID, ISSUE_ID, N7_PLAN_B)
        .expect("load other plan run")
        .expect("other plan run persists");
    assert!(
        matches!(kept_run_b.phase, CodingPlanRunPhase::Prepared),
        "other plan run phase must be untouched"
    );
    // 冻结 target 内：无主 repo_a 清；他人持锁 repo_c 保留（复核真实 owner）。
    assert!(
        lifecycle
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, repo_a)
            .expect("read repo a")
            .is_none(),
        "frozen target without another owner must be cleaned"
    );
    assert!(
        lifecycle
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, repo_c)
            .expect("read repo c")
            .is_some(),
        "frozen target actively locked by another owner must be preserved"
    );
    // 非冻结集的 repo_b（他 target）不得被 issue-wide 扫掉。
    assert!(
        lifecycle
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, repo_b)
            .expect("read repo b")
            .is_some(),
        "targets outside the frozen set must not be swept by issue-wide cleanup"
    );
    // plan 记录：A 删、B 留。
    assert!(
        lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, N7_PLAN_A)
            .is_err()
    );
    assert!(
        lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, N7_PLAN_B)
            .is_ok(),
        "other plan record must be preserved"
    );
}

/// 缺失幂等 OK、IO/JSON 错误不吞、删除后迟到写不复活 run 目录（legacy 闭包）。
#[tokio::test]
async fn plan_coding_n7_missing_resource_reentry_and_late_start() {
    // A) 无 run 的 plan：删除成功（缺失 OK）；重入不复活任何 run 产物。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = n7_seed_project_issue(&paths);
        n7_seed_plan(&lifecycle, N7_PLAN_A);

        let (status, body) = delete_work_item_plan_request(&app, N7_PLAN_A).await;
        assert_eq!(status, StatusCode::OK, "missing run is OK: {body}");
        assert!(!n7_run_file(&paths, N7_PLAN_A).exists());

        // D2.2 迁移：Deleted 墓碑在位 → 幂等 200（REQ-DEL-02「重复请求幂等返回
        // 已删除」），且零复活。
        let (reentry_status, reentry_body) = delete_work_item_plan_request(&app, N7_PLAN_A).await;
        assert_eq!(
            reentry_status,
            StatusCode::OK,
            "re-entry on deleted plan must be idempotent success: {reentry_body}"
        );
        assert!(
            !n7_run_file(&paths, N7_PLAN_A).exists() && !n7_run_lock(&paths, N7_PLAN_A).exists(),
            "re-entry must not resurrect run artifacts"
        );
    }

    // B) run JSON 损坏：IO/JSON 错误不得被吞——删除失败上抛且 plan 记录保持。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = n7_seed_project_issue(&paths);
        n7_seed_plan(&lifecycle, N7_PLAN_A);
        std::fs::create_dir_all(n7_runs_root(&paths)).expect("runs root");
        std::fs::write(n7_run_file(&paths, N7_PLAN_A), b"{ not valid json")
            .expect("seed corrupt run");

        let (status, body) = delete_work_item_plan_request(&app, N7_PLAN_A).await;
        assert_ne!(
            status,
            StatusCode::OK,
            "corrupt run json must surface (never swallowed): {body}"
        );
        assert!(
            lifecycle
                .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, N7_PLAN_A)
                .is_ok(),
            "failed closure before mutation must keep the plan record"
        );
    }

    // C) 删除成功后：迟到写（旧 command 的首启标记）必须拒绝且不复活 run 文件。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = n7_seed_project_issue(&paths);
        n7_seed_plan(&lifecycle, N7_PLAN_A);
        let coding_store = CodingAttemptStore::new(paths.clone());
        let attempt = n7_group_attempt(&coding_store, N7_PLAN_A);
        let run = n7_plan_run(
            N7_PLAN_A,
            &[(LogicalRepositoryId(Uuid::new_v4()), attempt.id.clone())],
            Some(&attempt.id),
            CodingPlanRunPhase::ReadyBound,
        );
        n7_write_run(&paths, &run);
        // 残留前导点锁：无墓碑（非删除窗口）时 purge 只清 run 文件、保留锁
        //（N2 修复轮语义：锁 unlink 只在墓碑/lease 保护的安全窗口执行）。
        std::fs::write(n7_run_lock(&paths, N7_PLAN_A), b"").expect("seed run lock");
        let run_store = CodingPlanRunStore::new(paths.clone());

        finalize_coding_attempt_deletion(&coding_store, &paths, &attempt)
            .expect("delete member workspace");
        let stale = run_store
            .get(PROJECT_ID, ISSUE_ID, N7_PLAN_A)
            .expect("load run")
            .expect("run invalidated by member deletion");
        let (status, body) = delete_work_item_plan_request(&app, N7_PLAN_A).await;
        assert_eq!(status, StatusCode::OK, "delete body: {body}");
        assert!(!n7_run_file(&paths, N7_PLAN_A).exists());
        assert!(
            !n7_run_lock(&paths, N7_PLAN_A).exists(),
            "D2.2 统一闭包持 delete lease＋purging 墓碑（安全删除窗口）——前导点锁一并清"
        );

        let late = run_store.mark_start_requested(&stale);
        assert!(
            late.is_err(),
            "late write on purged run must be rejected, got {late:?}"
        );
        assert!(
            !n7_run_file(&paths, N7_PLAN_A).exists(),
            "late write must not resurrect the run file"
        );
    }
}
