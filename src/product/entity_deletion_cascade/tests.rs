//! 级联删除闭包的回归测试（F3/F5/F6）。
//!
//! - F3：worktree 实体删除失败时**记录必须保留**（重入靠它定位实体），
//!   重入后实体被补删、墓碑才落成功（禁止「根已删、产物还在」的孤儿）。
//! - F5：attempt worktree 的 `.git` gitfile 不可反解时**仍删目录**（否则
//!   目录成孤儿，重入无法定位）。
//! - F6：原地演进路径改写 shared-worktree 记录前**必须持 repo resource
//!   cleanup lease**（否则并发 takeover 会被覆盖）。

use std::path::{Path, PathBuf};

use super::*;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use crate::product::entity_deletion_store::DeletionPhase;
use crate::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateWorkItemInput, LifecycleStore,
};
use crate::product::models::{IssueWorkItemPlanOptions, IssueWorkItemPlanStatus};

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const PLAN_ID: &str = "issue_work_item_plan_0001";
const WORK_ITEM_ID: &str = "work_item_0001";

fn seed_paths() -> (tempfile::TempDir, ProductAppPaths) {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(temp.path().join("aria"));
    (temp, paths)
}

/// 建一个存活 plan（legacy 路径）＋其 work item。
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

/// 建一个归属该 plan 的 coding attempt（可带 worktree 路径）。
fn seed_attempt(
    paths: &ProductAppPaths,
    worktree_path: Option<PathBuf>,
) -> crate::product::coding_models::CodingExecutionAttempt {
    let store = CodingAttemptStore::new(paths.clone());
    store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path,
            provider_config_snapshot: crate::web::workspace_ws_types::ProviderConfigSnapshot {
                author: crate::product::models::provider::ProviderName::Codex,
                reviewer: None,
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("seed attempt")
}

/// 建 shared-worktree 记录（锁持有者＝本 plan 的 attempt）＋返回记录路径。
fn seed_shared_worktree(
    paths: &ProductAppPaths,
    attempt_id: &str,
    worktree_path: &Path,
) -> (uuid::Uuid, PathBuf) {
    let repository_id = uuid::Uuid::from_u128(0x5150);
    std::fs::create_dir_all(worktree_path).expect("worktree dir");
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
        "current_active_work_item_id": WORK_ITEM_ID,
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
    (repository_id, path)
}

/// F5 回归：attempt worktree 的 `.git` gitfile 不可反解时**仍必须删目录**。
///
/// 旧行为直接 `Ok(())` 跳过 → 目录成孤儿，且重入时 attempt 记录已删、无任何
/// 路径能再定位它。
#[tokio::test]
async fn attempt_worktree_without_resolvable_gitfile_is_removed() {
    let (temp, paths) = seed_paths();
    let worktree = temp.path().join("orphan-attempt-worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    std::fs::write(worktree.join("file.txt"), b"x").expect("file");
    // 无 `.git` gitfile → 主仓路径不可反解。
    let attempt = seed_attempt(&paths, Some(worktree.clone()));

    remove_attempt_worktree_entity(&paths, &attempt)
        .await
        .expect("remove attempt worktree entity");

    assert!(
        !worktree.exists(),
        "反解失败也必须删目录（否则目录成孤儿，重入无法定位）"
    );
}

/// F3 回归：worktree 实体删除失败时记录必须保留（重入靠它定位实体），
/// 重入后实体被补删、墓碑才落成功。
#[tokio::test]
async fn worktree_entity_delete_failure_keeps_record_and_reentry_removes_entity() {
    let (temp, paths) = seed_paths();
    seed_plan(&paths);
    let attempt = seed_attempt(&paths, None);
    let worktree = temp.path().join("shared-worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    // 注入实体删除失败：`.git` gitfile 指向不存在的仓 → `git worktree remove`
    // 失败（绝不猜测仓路径）。
    std::fs::write(
        worktree.join(".git"),
        b"gitdir: /nonexistent/repo/.git/worktrees/wt\n",
    )
    .expect("gitfile");
    let (_repository_id, record_path) = seed_shared_worktree(&paths, &attempt.id, &worktree);
    let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);

    // ① 第一次：实体删除失败 → 整链 fail-closed（plan 根保留、墓碑未完成）。
    let failure = crate::product::plan_generation_supersede::delete_plan_generation(
        &paths,
        PROJECT_ID,
        ISSUE_ID,
        PLAN_ID,
        PlanSupersedeReason::Reprepare,
        None,
    )
    .await
    .expect_err("实体删除失败必须中止整链");
    assert!(
        format!("{failure:?}").contains("worktree") || format!("{failure:?}").contains("repo"),
        "失败原因必须指向 worktree 实体删除: {failure:?}"
    );
    assert!(
        record_path.exists(),
        "实体删除失败时 shared-worktree 记录必须保留（重入靠它定位实体）"
    );
    let tombstone = EntityDeletionStore::new(paths.clone())
        .get(&scope)
        .expect("tombstone")
        .expect("tombstone present");
    assert_eq!(
        tombstone.phase,
        DeletionPhase::Purging,
        "实体未清时墓碑不得落成功（禁止根已删、产物还在）"
    );

    // ② 修掉注入点后重入：实体被补删、记录被删、墓碑落成功。
    std::fs::remove_file(worktree.join(".git")).expect("remove gitfile");
    crate::product::plan_generation_supersede::delete_plan_generation(
        &paths,
        PROJECT_ID,
        ISSUE_ID,
        PLAN_ID,
        PlanSupersedeReason::Reprepare,
        None,
    )
    .await
    .expect("重入必须补删 worktree 实体");

    assert!(!worktree.exists(), "重入必须补删 worktree 实体（不留孤儿）");
    assert!(!record_path.exists(), "重入必须删除 shared-worktree 记录");
    let tombstone = EntityDeletionStore::new(paths.clone())
        .get(&scope)
        .expect("tombstone")
        .expect("tombstone present");
    assert_eq!(tombstone.phase, DeletionPhase::Deleted);
}

/// F6 回归：原地演进路径改写 shared-worktree 记录前必须取 repo resource
/// cleanup lease（读→校验→写全程不被并发 takeover 覆盖）。
#[test]
fn release_plan_shared_worktree_locks_takes_cleanup_lease() {
    let (temp, paths) = seed_paths();
    seed_plan(&paths);
    let attempt = seed_attempt(&paths, None);
    let worktree = temp.path().join("shared-worktree");
    let (repository_id, _record_path) = seed_shared_worktree(&paths, &attempt.id, &worktree);

    // 资源锁文件（hook 需要已存在的规范路径）。
    let lock_path = paths
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("entity-operations")
        .join("resource-locks")
        .join(format!("{}.lock", repository_id));
    std::fs::create_dir_all(lock_path.parent().expect("lock parent")).expect("lock dir");
    std::fs::write(&lock_path, b"").expect("lock file");
    let (_guard, attempts) =
        crate::product::coding_attempt_store::locking::register_exact_lock_attempt_hook(&lock_path);

    release_plan_shared_worktree_locks(
        &paths,
        PROJECT_ID,
        ISSUE_ID,
        std::slice::from_ref(&attempt.id),
    )
    .expect("release shared worktree locks");

    assert!(
        attempts.try_recv().is_ok(),
        "原地路径改写 shared-worktree 记录前必须取 repo resource cleanup lease"
    );
}
