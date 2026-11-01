//! 级联删除闭包的回归测试（F3/F5/R3/F6）。
//!
//! - F3：worktree 实体删除失败时**记录必须保留**（重入靠它定位实体），
//!   重入后实体被补删、墓碑才落成功（禁止「根已删、产物还在」的孤儿）。
//! - F5：attempt worktree 的 `.git` gitfile 不可反解时**仍删目录**（否则
//!   目录成孤儿，重入无法定位）——但只限 aria worktree 命名空间内的实体。
//! - R3（数据安全）：反解失败兜底 `remove_dir_all` 前必须校验路径位于
//!   `<repo>/.worktrees/aria-work-items|aria-issues|aria-pointer/…`；
//!   指向主仓根/他处目录一律 fail-closed（记录保留待人工），绝不整仓删除。
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

/// 某仓下的 aria worktree 实体路径（`<repo>/.worktrees/<prefix>/<name>`）。
fn aria_worktree_path(repo: &Path, prefix: &str, name: &str) -> PathBuf {
    repo.join(".worktrees").join(prefix).join(name)
}

/// F5 回归：attempt worktree 的 `.git` gitfile 不可反解时**仍必须删目录**。
///
/// 旧行为直接 `Ok(())` 跳过 → 目录成孤儿，且重入时 attempt 记录已删、无任何
/// 路径能再定位它。R3：兜底删除只对 **aria worktree 命名空间内**的实体生效
/// （故此处路径必须落在 `.worktrees/aria-work-items/…`）。
#[tokio::test]
async fn attempt_worktree_without_resolvable_gitfile_is_removed() {
    let (temp, paths) = seed_paths();
    let worktree = aria_worktree_path(&temp.path().join("repo"), "aria-work-items", "wt-1");
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

/// F5/R3 回归（数据安全）：反解失败兜底**MUST NOT** 删主仓根。
///
/// 现场：`worktree_path` 被记录为主仓根（`.git` 是目录 → 反解失败 → 旧实现
/// 直接 `remove_dir_all`）→ 整仓删除。修法：兜底前校验路径必须落在
/// `<repo>/.worktrees/aria-work-items|aria-issues|aria-pointer` 命名空间内，
/// 不满足即 fail-closed（返回错误、保留记录待人工）。
#[tokio::test]
async fn fallback_worktree_removal_refuses_main_repo_root() {
    let (temp, paths) = seed_paths();
    let main_repo = temp.path().join("main-repo");
    std::fs::create_dir_all(main_repo.join(".git")).expect("git dir");
    std::fs::create_dir_all(main_repo.join("src")).expect("src dir");
    std::fs::write(main_repo.join("README.md"), b"must survive").expect("readme");
    // `.git` 是目录 → `resolve_main_repo_from_worktree` 反解失败。
    let attempt = seed_attempt(&paths, Some(main_repo.clone()));

    let error = remove_attempt_worktree_entity(&paths, &attempt)
        .await
        .expect_err("指向主仓根的兜底删除必须 fail-closed");

    let message = format!("{error:?}");
    assert!(
        message.contains("worktrees") || message.contains("namespace"),
        "失败原因必须指向命名空间守卫: {message}"
    );
    assert!(
        main_repo.join("README.md").exists(),
        "主仓根文件绝不能被兜底删除"
    );
    assert!(main_repo.join(".git").is_dir(), "主仓根 `.git` 绝不能被删");
    assert!(main_repo.join("src").is_dir(), "主仓根子树绝不能被删");
}

/// F5/R3 回归：shared-worktree 实体的兜底删除同样受命名空间守卫约束。
///
/// - 非 aria 命名空间（`<repo>/.worktrees/manual/…`、仓内普通目录）→
///   fail-closed，目录保留；
/// - 命名空间内的 aria worktree 实体 → 正常删除（不留孤儿）。
#[tokio::test]
async fn shared_worktree_entity_removal_enforces_aria_namespace_guard() {
    let (temp, _paths) = seed_paths();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");

    // ① 仓内普通目录（非 worktree 实体）。
    let plain_dir = repo.join("src");
    std::fs::create_dir_all(&plain_dir).expect("plain dir");
    std::fs::write(plain_dir.join("main.rs"), b"fn main() {}").expect("rs");
    let error = remove_shared_worktree_entity(&plain_dir)
        .await
        .expect_err("仓内普通目录必须 fail-closed");
    assert!(
        format!("{error:?}").contains("worktrees"),
        "失败原因必须指向命名空间守卫: {error:?}"
    );
    assert!(plain_dir.join("main.rs").exists(), "仓内普通目录绝不能被删");

    // ② `.worktrees` 下非 aria 前缀。
    let manual = aria_worktree_path(&repo, "manual", "wt");
    std::fs::create_dir_all(&manual).expect("manual dir");
    std::fs::write(manual.join("keep.txt"), b"x").expect("file");
    remove_shared_worktree_entity(&manual)
        .await
        .expect_err("非 aria 前缀必须 fail-closed");
    assert!(
        manual.join("keep.txt").exists(),
        "非 aria 前缀目录绝不能被删"
    );

    // ③ 命名空间根本身（不得整层删）。
    let namespace_root = repo.join(".worktrees").join("aria-issues");
    std::fs::create_dir_all(namespace_root.join("other-wt")).expect("other wt");
    remove_shared_worktree_entity(&namespace_root)
        .await
        .expect_err("命名空间根不是实体，必须 fail-closed");
    assert!(
        namespace_root.join("other-wt").exists(),
        "命名空间根绝不能被删"
    );

    // ④ 合法 aria 实体：正常删除。
    let safe = aria_worktree_path(&repo, "aria-issues", "issue-1");
    std::fs::create_dir_all(&safe).expect("safe dir");
    std::fs::write(safe.join("file.txt"), b"x").expect("file");
    remove_shared_worktree_entity(&safe)
        .await
        .expect("命名空间内的 aria 实体必须可删");
    assert!(!safe.exists(), "合法 aria worktree 实体必须被删除");
}

/// F3 回归：worktree 实体删除失败时记录必须保留（重入靠它定位实体），
/// 重入后实体被补删、墓碑才落成功。
#[tokio::test]
async fn worktree_entity_delete_failure_keeps_record_and_reentry_removes_entity() {
    let (temp, paths) = seed_paths();
    seed_plan(&paths);
    let attempt = seed_attempt(&paths, None);
    // R3：兜底删除只对 aria worktree 命名空间内的实体生效。
    let worktree = aria_worktree_path(&temp.path().join("repo"), "aria-work-items", "wt-f3");
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
