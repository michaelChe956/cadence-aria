//! REQ-WPS-01~07 验收测试：单 plan 不变量 + 取代删除的完整闭包。
//!
//! 红态（实现前）复现 owner 现场残留：删除两个 plan 后 `coding-attempts`
//! 仍有 2 个、reviewer 窗口 7、快照 8、租约 1——即旧代产物未被清除。
//! 绿态（实现后）：取代删除后从属产物逐项归零，只留一条无内容删除事实。

use std::path::{Path, PathBuf};

use super::*;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use crate::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateWorkItemInput, LifecycleStore,
};
use crate::product::models::{IssueWorkItemPlanOptions, IssueWorkItemPlanStatus};

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const PLAN_ID: &str = "issue_work_item_plan_0001";
const WORK_ITEM_ID: &str = "work_item_0001";
const WINDOW_ID: &str = "attempt_0001-role_run_0001-w01";

fn seed_paths() -> (tempfile::TempDir, ProductAppPaths) {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(temp.path().join("aria"));
    (temp, paths)
}

/// 建一个存活 plan（legacy 路径）。
fn seed_plan(paths: &ProductAppPaths) {
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some(PLAN_ID.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: vec![WORK_ITEM_ID.to_string()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("seed plan");
    // plan 拥有的 work item（真实 owner-key：attempt 经 work item 归属本 plan）。
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some(WORK_ITEM_ID.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            source_work_item_plan_id: Some(PLAN_ID.to_string()),
            ..Default::default()
        })
        .expect("seed work item");
}

/// 建一个归属该 plan 的 coding attempt（work item scope）＋其运行目录。
fn seed_attempt(paths: &ProductAppPaths) {
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: crate::web::workspace_ws_types::ProviderConfigSnapshot {
                author: crate::product::models::provider::ProviderName::Codex,
                reviewer: None,
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("seed attempt");
    // 运行目录（units/{unit}/runs）——必须与 attempt 记录一并清除。
    let run_dir = paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("coding-attempts")
        .join(&attempt.id)
        .join("units")
        .join("coding_unit_0001")
        .join("runs");
    std::fs::create_dir_all(&run_dir).expect("run dir");
    std::fs::write(run_dir.join("unit_run_0001.json"), b"{}").expect("run record");
}

/// 建 reviewer 窗口（活跃 ＋ 归档）＋租约＋快照基线（活跃 ＋ 归档）。
fn seed_reviewer_resources(paths: &ProductAppPaths, attempt_id: &str) {
    let window_root = paths.root().join("reviewer-windows");
    let windows = window_root.join("windows");
    std::fs::create_dir_all(&windows).expect("windows dir");
    let common_dir = paths.root().join("reviewer-common");
    std::fs::create_dir_all(&common_dir).expect("common dir");
    let record = serde_json::json!({
        "schema": "reviewer_window_v1",
        "window_id": WINDOW_ID,
        "generation": 1,
        "identity": {
            "canonical_worktree": common_dir.join("wt"),
            "gitdir": common_dir.join("gitdir"),
            "common_dir": common_dir,
            "lease_id": "lease_0001",
        },
        "attempt_id": attempt_id,
        "run_id": "role_run_0001",
        "provider": "codex",
        "state": "running",
        "report_acceptance": "pending_acceptance",
        "platform_write_paths": [],
        "quiesced_scopes": [],
        "baseline": null,
        "restore": null,
        "recovery": null,
        "last_error": null,
        "violation_pending": null,
        "pre_restore_git_facts": [],
        "transitions": [],
        "created_at": "2026-10-24T00:00:00Z",
        "updated_at": "2026-10-24T00:00:00Z",
    });
    std::fs::write(
        windows.join(format!("{WINDOW_ID}.json")),
        serde_json::to_vec(&record).expect("window json"),
    )
    .expect("seed window");
    // 归档副本（归档 ≠ 删除）。
    let archive = window_root.join("window-archive");
    std::fs::create_dir_all(&archive).expect("archive dir");
    std::fs::copy(
        windows.join(format!("{WINDOW_ID}.json")),
        archive.join(format!("{WINDOW_ID}.archived-g1-20261024T000000000Z.json")),
    )
    .expect("seed archived window");
    // 树级租约。
    let leases = window_root.join("leases");
    std::fs::create_dir_all(&leases).expect("leases dir");
    let lease = serde_json::json!({
        "schema": "reviewer_window_lease_v1",
        "lease_id": "lease_0001",
        "tree_key": "tree_key_0001",
        "canonical_worktree": common_dir.join("wt"),
        "gitdir": common_dir.join("gitdir"),
        "common_dir": common_dir,
        "window_id": WINDOW_ID,
        "attempt_id": attempt_id,
        "run_id": "role_run_0001",
        "provider": "codex",
        "acquired_at": "2026-10-24T00:00:00Z",
        "owner_pid": 0,
        "owner_start_ticks": 0,
    });
    std::fs::write(
        leases.join("tree_key_0001.json"),
        serde_json::to_vec(&lease).expect("lease json"),
    )
    .expect("seed lease");
    // 快照基线（活跃 ＋ 归档）。
    let snapshot_root = paths
        .root()
        .join(crate::product::reviewer_resource_purge::REVIEWER_SNAPSHOT_STORE_DIR);
    let snapshot_window = snapshot_root.join("windows").join(WINDOW_ID);
    std::fs::create_dir_all(&snapshot_window).expect("snapshot window");
    std::fs::write(snapshot_window.join("manifest.json"), b"{}").expect("snapshot manifest");
    let snapshot_archive = snapshot_root
        .join("windows-archive")
        .join(format!("{WINDOW_ID}.20261024T000000000Z"));
    std::fs::create_dir_all(&snapshot_archive).expect("snapshot archive");
}

/// 建 shared-worktree 记录（锁持有者＝本 plan 的 attempt）。
fn seed_shared_worktree(paths: &ProductAppPaths, attempt_id: &str) {
    let repository_id = uuid::Uuid::from_u128(0x5150);
    let worktree_path = paths.root().join("shared-worktree-checkout");
    std::fs::create_dir_all(&worktree_path).expect("worktree dir");
    let record = serde_json::json!({
        "id": "shared_worktree_0001",
        "project_id": PROJECT_ID,
        "issue_id": ISSUE_ID,
        "repository_id": repository_id.to_string(),
        "target_repository_id": repository_id.to_string(),
        "path_schema_version": 1,
        "branch_name": "aria/work-items/work_item_0001",
        "worktree_path": worktree_path,
        "base_branch": "main",
        "status": "running",
        "current_active_work_item_id": "work_item_0001",
        "current_lock_owner_id": attempt_id,
        "last_completed_work_item_id": null,
        "created_at": "2026-10-24T00:00:00Z",
        "updated_at": "2026-10-24T00:00:00Z",
    });
    let path = paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("shared-worktrees")
        .join(format!("{repository_id}.json"));
    crate::product::json_store::write_json(&path, &record).expect("seed shared worktree");
}

fn exists(path: &Path) -> bool {
    path.exists()
}

/// 取代删除后旧代产物逐项归零，且只留一条带触发原因的无内容墓碑。
#[tokio::test]
async fn supersede_removes_full_closure_and_leaves_single_tombstone() {
    let (_temp, paths) = seed_paths();
    seed_plan(&paths);
    seed_attempt(&paths);
    let attempt_id = CodingAttemptStore::new(paths.clone())
        .list_attempts_for_work_item(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID)
        .expect("attempts")
        .first()
        .expect("attempt present")
        .id
        .clone();
    seed_reviewer_resources(&paths, &attempt_id);
    seed_shared_worktree(&paths, &attempt_id);

    let issue_root = paths.issue_root(PROJECT_ID, ISSUE_ID);
    let attempts_root = issue_root.join("coding-attempts");
    let window_root = paths.root().join("reviewer-windows");
    let snapshot_root = paths
        .root()
        .join(crate::product::reviewer_resource_purge::REVIEWER_SNAPSHOT_STORE_DIR);
    let repository_id = uuid::Uuid::from_u128(0x5150);

    // 前置：现场残留非零（owner 报告的形态）。
    let attempt_id = CodingAttemptStore::new(paths.clone())
        .list_attempts_for_work_item(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID)
        .expect("attempts")
        .first()
        .expect("attempt present")
        .id
        .clone();
    assert!(exists(&attempts_root.join(format!("{attempt_id}.json"))));
    assert!(exists(
        &window_root
            .join("windows")
            .join(format!("{WINDOW_ID}.json"))
    ));
    assert!(exists(&snapshot_root.join("windows").join(WINDOW_ID)));

    let plan_slot = acquire_issue_plan_slot(&paths, PROJECT_ID, ISSUE_ID)
        .await
        .expect("plan slot");
    let report = ensure_single_plan_generation(
        &paths,
        &plan_slot,
        PROJECT_ID,
        ISSUE_ID,
        &[],
        PlanSupersedeReason::Reprepare,
        None,
    )
    .await
    .expect("supersede");

    // 旧 plan 身份与从属产物逐项归零（残留清单必须为空）。
    assert_eq!(report.deleted_plan_ids, vec![PLAN_ID.to_string()]);
    let residue = vec![
        (
            "plan_record",
            exists(
                &issue_root
                    .join("issue-work-item-plans")
                    .join(format!("{PLAN_ID}.json")),
            ),
        ),
        (
            "attempt_record",
            exists(&attempts_root.join(format!("{attempt_id}.json"))),
        ),
        ("attempt_run_dirs", exists(&attempts_root.join(&attempt_id))),
        (
            "reviewer_window",
            exists(
                &window_root
                    .join("windows")
                    .join(format!("{WINDOW_ID}.json")),
            ),
        ),
        (
            "reviewer_window_archive",
            exists(
                &window_root
                    .join("window-archive")
                    .join(format!("{WINDOW_ID}.archived-g1-20261024T000000000Z.json")),
            ),
        ),
        (
            "reviewer_lease",
            exists(&window_root.join("leases").join("tree_key_0001.json")),
        ),
        (
            "reviewer_snapshot",
            exists(&snapshot_root.join("windows").join(WINDOW_ID)),
        ),
        (
            "reviewer_snapshot_archive",
            exists(
                &snapshot_root
                    .join("windows-archive")
                    .join(format!("{WINDOW_ID}.20261024T000000000Z")),
            ),
        ),
        (
            "shared_worktree_record",
            exists(
                &issue_root
                    .join("shared-worktrees")
                    .join(format!("{repository_id}.json")),
            ),
        ),
    ];
    let remaining: Vec<&str> = residue
        .iter()
        .filter(|(_, present)| *present)
        .map(|(name, _)| *name)
        .collect();
    assert!(
        remaining.is_empty(),
        "取代删除后旧代产物仍有残留: {remaining:?}（owner 现场：attempts/windows/snapshots/leases 不为零）"
    );

    // 单 plan 不变量：存活 plan 数归零。
    assert!(
        list_live_plan_ids(&paths, PROJECT_ID, ISSUE_ID)
            .expect("live plans")
            .is_empty()
    );

    // 只留一条无内容删除事实，且带触发原因。
    let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);
    let tombstone = EntityDeletionStore::new(paths.clone())
        .get(&scope)
        .expect("tombstone")
        .expect("tombstone present");
    assert_eq!(tombstone.phase, DeletionPhase::Deleted);
    assert_eq!(
        tombstone.trigger_reason.as_deref(),
        Some("reprepare"),
        "墓碑必须记录触发原因（REQ-WPS-07）"
    );
    assert!(tombstone.deleted_at.is_some());
}

/// 幂等：完成后重放不新增墓碑、不复活内容。
#[tokio::test]
async fn supersede_replay_is_idempotent() {
    let (_temp, paths) = seed_paths();
    seed_plan(&paths);
    seed_attempt(&paths);

    {
        let plan_slot = acquire_issue_plan_slot(&paths, PROJECT_ID, ISSUE_ID)
            .await
            .expect("plan slot");
        ensure_single_plan_generation(
            &paths,
            &plan_slot,
            PROJECT_ID,
            ISSUE_ID,
            &[],
            PlanSupersedeReason::Reprepare,
            None,
        )
        .await
        .expect("first supersede");
    }
    let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);
    let first = EntityDeletionStore::new(paths.clone())
        .get(&scope)
        .expect("tombstone")
        .expect("tombstone");

    let plan_slot = acquire_issue_plan_slot(&paths, PROJECT_ID, ISSUE_ID)
        .await
        .expect("plan slot");
    let report = ensure_single_plan_generation(
        &paths,
        &plan_slot,
        PROJECT_ID,
        ISSUE_ID,
        &[],
        PlanSupersedeReason::Reprepare,
        None,
    )
    .await
    .expect("replay supersede");
    assert!(report.deleted_plan_ids.is_empty(), "重放不得重复删除");
    let second = EntityDeletionStore::new(paths.clone())
        .get(&scope)
        .expect("tombstone")
        .expect("tombstone");
    assert_eq!(first, second, "重放不得改变墓碑（数量/时间不变）");
    let records_dir = paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("entity-deletions")
        .join("plan");
    let count = std::fs::read_dir(&records_dir)
        .expect("records dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|v| v.to_str()) == Some("json"))
        .count();
    assert_eq!(count, 1, "只落一条墓碑");
}

/// 不同 issue 的同名逻辑 plan 不受影响（作用域隔离）。
#[tokio::test]
async fn supersede_does_not_touch_other_issue() {
    let (_temp, paths) = seed_paths();
    seed_plan(&paths);
    let other_lifecycle = LifecycleStore::new(paths.clone());
    other_lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some(PLAN_ID.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: "issue_0002".to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: IssueWorkItemPlanOptions {
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
        .expect("seed other issue plan");

    let plan_slot = acquire_issue_plan_slot(&paths, PROJECT_ID, ISSUE_ID)
        .await
        .expect("plan slot");
    ensure_single_plan_generation(
        &paths,
        &plan_slot,
        PROJECT_ID,
        ISSUE_ID,
        &[],
        PlanSupersedeReason::Reprepare,
        None,
    )
    .await
    .expect("supersede");

    assert!(
        other_lifecycle
            .get_issue_work_item_plan(PROJECT_ID, "issue_0002", PLAN_ID)
            .is_ok(),
        "他 issue 的同名 plan 必须原样保留"
    );
}

/// 建旧代 advance 产物（record ＋ 外层 initialization journal）。
fn seed_advance_artifacts(paths: &ProductAppPaths, plan_id: &str) -> (PathBuf, PathBuf) {
    let root = paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("advance-records");
    std::fs::create_dir_all(&root).expect("advance root");
    let record_path = root.join("advance_0001.json");
    let journal_path = root.join(format!("{plan_id}.initialization.json"));
    let record = serde_json::json!({
        "id": "advance_0001",
        "command_id": "advance_command_0001",
        "project_id": PROJECT_ID,
        "issue_id": ISSUE_ID,
        "plan_id": plan_id,
        "plan_revision_id": "plan_revision_0001",
        "attempt_id": "attempt_0001",
        "status": "completed",
        "workspace_entry": null,
        "error": null,
        "created_at": "2026-10-24T00:00:00Z",
        "updated_at": "2026-10-24T00:00:00Z",
    });
    std::fs::write(
        &record_path,
        serde_json::to_vec(&record).expect("advance record"),
    )
    .unwrap();
    let journal = serde_json::json!({
        "advance_id": "advance_0001",
        "plan_id": plan_id,
        "attempt_id": "attempt_0001",
        "phase": "ready",
        "error": null,
        "created_at": "2026-10-24T00:00:00Z",
        "updated_at": "2026-10-24T00:00:00Z",
    });
    std::fs::write(
        &journal_path,
        serde_json::to_vec(&journal).expect("advance journal"),
    )
    .unwrap();
    (record_path, journal_path)
}

/// F2 回归：原地重编译（新代沿用同一 plan_id）必须清旧代 advance 产物——
///
/// 残留 record 会被 advance 按 plan 回放（`Replayed`），新代永远进不了 coding。
#[tokio::test]
async fn inplace_recompile_clears_old_generation_advance_artifacts() {
    let (_temp, paths) = seed_paths();
    seed_plan(&paths);
    let (record_path, journal_path) = seed_advance_artifacts(&paths, PLAN_ID);
    assert!(record_path.exists() && journal_path.exists());

    let plan_slot = acquire_issue_plan_slot(&paths, PROJECT_ID, ISSUE_ID)
        .await
        .expect("plan slot");
    let report = ensure_single_plan_generation(
        &paths,
        &plan_slot,
        PROJECT_ID,
        ISSUE_ID,
        &[PLAN_ID.to_string()],
        PlanSupersedeReason::RecompileConfirm,
        None,
    )
    .await
    .expect("inplace supersede");
    assert_eq!(report.runtime_purged_plan_ids, vec![PLAN_ID.to_string()]);

    assert!(
        !record_path.exists(),
        "旧代 advance record 必须清（新代不得被按 plan 回放旧记录）"
    );
    assert!(
        !journal_path.exists(),
        "旧代 advance initialization journal 必须清"
    );
    // 新代 advance 命令不得命中旧记录（否则 outcome 为 Replayed）。
    let store = crate::product::advance_store::AdvanceStore::new(paths.clone());
    assert!(
        store
            .get_advance_for_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .expect("advance for plan")
            .is_none(),
        "新代不得再命中旧代 advance record"
    );
}

/// F2 回归：无 attempt 时**不得提前返回**——旧代锁残留、初始化 journal 与
/// workspace lifecycle 仍须清（否则新代继承旧执行轮状态）。
#[tokio::test]
async fn inplace_recompile_without_attempts_still_clears_locks_and_lifecycle() {
    let (_temp, paths) = seed_paths();
    seed_plan(&paths);
    // 无 attempt（首次编译场景）。
    assert!(
        CodingAttemptStore::new(paths.clone())
            .list_attempts_for_work_item(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID)
            .expect("attempts")
            .is_empty()
    );

    let issue_root = paths.issue_root(PROJECT_ID, ISSUE_ID);
    // 旧代 attempt 锁残留。
    let lock_dir = issue_root
        .join("coding-attempts")
        .join("work-item-attempt-locks");
    std::fs::create_dir_all(&lock_dir).expect("lock dir");
    let lock_path = lock_dir.join(WORK_ITEM_ID);
    std::fs::write(&lock_path, b"owner").expect("lock");
    // 旧代 group 初始化 journal。
    let journal_dir = issue_root
        .join("coding-attempts")
        .join("group-initializations");
    std::fs::create_dir_all(&journal_dir).expect("journal dir");
    let journal_path = journal_dir.join(format!("{PLAN_ID}.json"));
    std::fs::write(&journal_path, b"{}").expect("journal");
    // 旧代 workspace lifecycle（执行轮 generation）。
    let lifecycle_root = issue_root.join("coding-plan-workspace-lifecycles");
    std::fs::create_dir_all(&lifecycle_root).expect("lifecycle root");
    let lifecycle_path = lifecycle_root.join(format!("{PLAN_ID}.json"));
    std::fs::write(
        &lifecycle_path,
        serde_json::to_vec(&serde_json::json!({
            "project_id": PROJECT_ID,
            "issue_id": ISSUE_ID,
            "plan_id": PLAN_ID,
            "generation": 3,
            "created_at": "2026-10-24T00:00:00Z",
            "updated_at": "2026-10-24T00:00:00Z",
        }))
        .expect("lifecycle json"),
    )
    .expect("lifecycle");

    let plan_slot = acquire_issue_plan_slot(&paths, PROJECT_ID, ISSUE_ID)
        .await
        .expect("plan slot");
    ensure_single_plan_generation(
        &paths,
        &plan_slot,
        PROJECT_ID,
        ISSUE_ID,
        &[PLAN_ID.to_string()],
        PlanSupersedeReason::RecompileConfirm,
        None,
    )
    .await
    .expect("inplace supersede");

    assert!(!lock_path.exists(), "无 attempt 也必须清 attempt 锁残留");
    assert!(!journal_path.exists(), "无 attempt 也必须清初始化 journal");
    assert!(
        !lifecycle_path.exists(),
        "无 attempt 也必须清 workspace lifecycle（执行轮 generation 复位）"
    );
}

/// 一个并发触发：plan-slot 互斥区内的「取代删除 + 建立新 plan」。
///
/// 建立动作与取代删除同处一个互斥区（单 plan 不变量 REQ-WPS-01）。
async fn supersede_and_establish(
    paths: &ProductAppPaths,
    plan_id: &str,
) -> Result<(), CascadeDeleteError> {
    let plan_slot = acquire_issue_plan_slot(paths, PROJECT_ID, ISSUE_ID).await?;
    ensure_single_plan_generation(
        paths,
        &plan_slot,
        PROJECT_ID,
        ISSUE_ID,
        &[],
        PlanSupersedeReason::Reprepare,
        None,
    )
    .await?;
    // 取代删除与建立之间让出调度（真实路径此处有 await：会话 manager 驱逐、
    // plan 创建）：并发触发的真实交错点。
    tokio::task::yield_now().await;
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some(plan_id.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: IssueWorkItemPlanOptions {
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
        .map_err(CascadeDeleteError::Store)?;
    Ok(())
}

/// F4 回归：并发两次「取代删除 + 建立 plan」（同一 issue）→ 存活 plan 恒为 1。
///
/// 「删旧 + 建立」必须在同一 issue 级 plan-slot 互斥区内——否则两个并发
/// 触发可各建一个 plan（不变量复核发生在建立**之前**，捕获不到）。
#[tokio::test]
async fn concurrent_supersede_and_establish_keeps_single_live_plan() {
    let (_temp, paths) = seed_paths();

    let (first, second) = tokio::join!(
        supersede_and_establish(&paths, "issue_work_item_plan_0002"),
        supersede_and_establish(&paths, "issue_work_item_plan_0003"),
    );
    first.expect("first supersede+establish");
    second.expect("second supersede+establish");

    let live = list_live_plan_ids(&paths, PROJECT_ID, ISSUE_ID).expect("live plans");
    assert_eq!(
        live.len(),
        1,
        "同一 issue 存活 plan 必须恒为 1（并发触发不得各建一个 plan），实际: {live:?}"
    );
}
