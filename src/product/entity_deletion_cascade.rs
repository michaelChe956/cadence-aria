//! 实体级联删除编排（产品层核心）与 plan 代际取代（REQ-WPS-01~07）。
//!
//! # 复用关系
//!
//! 用户面 DELETE（Story/Design/Plan）与「plan 代际取代」（re-enable 轮换
//! `prepare_intent_id` / 重新 prepare / 重新编译确认）**共用同一闭包实现**
//! （本模块 [`purge_entity_closure`] ＋ [`cleanup_plan_shared_worktrees`]）：
//! 先子树后根、真实 owner-key 定位、缺失即视为已清、失败 fail-closed、
//! 重复请求幂等、只留一条无内容删除事实。
//!
//! # 两种 coding 门禁
//!
//! - [`CodingGateMode::Refuse`]：用户面删除——存在 coding attempt 即拒删
//!   （REQ-DEL-05 不变；本能力不放宽该语义）。
//! - [`CodingGateMode::Cascade`]：代际取代——先收口未终结 attempt，再级联
//!   删除 attempts 及其运行目录、worktree 实体、shared-worktree 记录与锁、
//!   attempt locks、reviewer windows/snapshots/leases（REQ-WPS-06）。
//!
//! # fail-closed
//!
//! 任一从属产物删除失败 → 不报成功、不建立新 plan、**plan 根保留**为可达的
//! `purging` 入口，并把失败项持久化（[`EntityDeletionFailure`]）供人工重入
//! （REQ-WPS-04）。绝不出现「根已删、产物还在」的中间态。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionAttempt};
use crate::product::coding_plan_run::{CodingPlanRun, CodingPlanRunStore};
use crate::product::entity_deletion_store::{
    DeletionEntityKind, EntityDeletionFailure, EntityDeletionManifest, EntityDeletionStore,
    EntityScope,
};
use crate::product::entity_operation_store::{
    EntityDeletionLease, EntityOperationError, EntityOperationStore, SharedResourceCleanupLease,
    SharedResourceKey,
};
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::reviewer_resource_purge::ReviewerResourcePurger;
use crate::product::work_item_plan_source_store::WorkItemPlanSourceStore;
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

/// coding 门禁模式（见模块文档）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodingGateMode {
    /// 用户面删除：存在 coding attempt 即拒删（REQ-DEL-05）。
    Refuse,
    /// 代际取代：级联删除 coding attempts 及其运行资源（REQ-WPS-06）。
    Cascade,
}

/// 代际取代的触发原因（REQ-WPS-02：三类触发点共用同一闭包与语义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanSupersedeReason {
    /// terminated enrollment 人工重新开启并轮换 `prepare_intent_id`。
    ReEnablePrepareIntentRotation,
    /// 重新 prepare（人工或自动链再次建立 plan）。
    Reprepare,
    /// 重新编译确认（沿用同一 plan_id 原地演进或建立新代）。
    RecompileConfirm,
}

impl PlanSupersedeReason {
    /// 稳定短码（写入删除事实，供排障区分「用户删除」与「代际取代」）。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReEnablePrepareIntentRotation => "re_enable_prepare_intent_rotation",
            Self::Reprepare => "reprepare",
            Self::RecompileConfirm => "recompile_confirm",
        }
    }
}

/// 级联删除的产品层失败原因（web 层映射为稳定 ApiError 码）。
///
/// 用户面删除的业务门禁（coding 拒删 / 存活引用 / 分级确认）仍由 web 层
/// 判定（`lifecycle::deletion`），本类型只承载**取代路径**会真实产生的失败。
#[derive(Debug)]
pub(crate) enum CascadeDeleteError {
    Store(ProductStoreError),
    /// 同 scope 仍有 writer / 删除准入争用（REQ-DEL-06；取代路径据此
    /// fail-closed：不建立新 plan）。
    Busy,
    /// 冻结 target 不是合法逻辑仓 id（数据损坏，fail-closed）。
    InvalidTargetRepository {
        target_repository_id: String,
    },
}

/// 代际取代结果（供调用方审计与断言）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PlanSupersedeReport {
    /// 被完整删除的旧 plan id（按序）。
    pub deleted_plan_ids: Vec<String>,
    /// 被删除的旧 plan 的 workspace session id（供内存 manager 驱逐）。
    pub evicted_session_ids: Vec<String>,
    /// 原地演进路径被清理运行资源的 plan id（plan 根保留）。
    pub runtime_purged_plan_ids: Vec<String>,
}

/// 实体从属闭包清理（**先子树后根**的全部子树；plan 根由调用方最后删）。
///
/// 顺序：生命周期 owned children → source/IR/报告 → drafts/compiles/outlines →
/// revisions/publications → coding attempts 及其运行资源（仅 Cascade）→
/// attempt 残留锁 → 目标仓 shared-worktree 记录/锁/worktree 实体 → coding plan
/// run → workspace lifecycle → advance 独立 checkpoint（session 索引）。
pub(crate) async fn purge_entity_closure(
    app_paths: &ProductAppPaths,
    lease: &EntityDeletionLease,
    manifest: &EntityDeletionManifest,
    plan_run: Option<&CodingPlanRun>,
    resource_leases: &[(SharedResourceKey, SharedResourceCleanupLease)],
    coding_gate: CodingGateMode,
    workspace_root: Option<&Path>,
) -> Result<(), ProductStoreError> {
    let scope = lease.scope();
    let store = LifecycleStore::new(app_paths.clone());
    store.purge_entity_owned_children(lease, manifest)?;

    if scope.kind == DeletionEntityKind::Plan {
        let (project_id, issue_id, plan_id) = (
            scope.project_id.as_str(),
            scope.issue_id.as_str(),
            scope.entity_id.as_str(),
        );
        // source/IR/机械报告/provenance 子树（缺失=已清，幂等重入）。
        WorkItemPlanSourceStore::new(app_paths.clone())
            .purge_plan_sources(project_id, issue_id, plan_id)
            .map_err(|error| {
                ProductStoreError::Io(format!("purge plan sources {plan_id}: {error:?}"))
            })?;
        // plan store drafts / compile transaction / outline context index。
        WorkItemPlanStore::new(app_paths.clone())
            .purge_plan_artifacts(project_id, issue_id, plan_id)?;
        // revisions + publications。
        WorkItemRevisionStore::new(app_paths.clone())
            .purge_plan_revisions(project_id, issue_id, plan_id)?;
        // 本 plan 拥有的 attempt 集合（真实 owner-key；删除前先采集——删除后
        // 无法再由 attempt 记录反推 shared-worktree 锁的持有者）。
        let plan_attempt_ids: Vec<String> = if coding_gate == CodingGateMode::Cascade {
            let coding_store = CodingAttemptStore::new(app_paths.clone());
            plan_owned_attempts(
                &coding_store,
                project_id,
                issue_id,
                plan_id,
                &manifest.work_item_ids,
            )?
            .into_iter()
            .map(|attempt| attempt.id)
            .collect()
        } else {
            Vec::new()
        };
        // 目标仓资源（shared-worktree 记录 + 锁 + worktree 实体）。
        // **先于 attempt 清理**：实体删除失败即中止 → attempt 记录仍在 →
        // 重入仍能按真实 owner-key 判定归属并补删实体（attempt 先删则归属
        // 复核失配，实体残留成孤儿）。
        //
        // 实体先删、记录后删（记录删除即释放其锁；实体删除失败时锁原样保留）。
        //
        // ① 失败重入补删：上次遗留的待删实体（记录可能已被他路径删掉、归属
        //    复核也可能失配——实体目录路径必须被记住，否则残留成孤儿）。
        //
        //    F3/R4 已知边界（有意保留）：本清单只有**路径**、没有仓库身份，
        //    故补删不再取 repo resource cleanup lease，也不做「记录仍在且仍
        //    归属本 plan」的复核——记录缺失正是需要本清单的原因。可删除面由
        //    两道硬约束封住：
        //    · 写入侧：清单只在 ② 中、持有 deletion lease ＋ 候选仓
        //      resource_leases ＋ issue 关系短锁时落盘，内容即本次授权的
        //      实体集合；
        //    · 删除侧：兜底删除强制 aria worktree 命名空间守卫
        //      （`ensure_worktree_namespace_path`，见 `remove_shared_worktree_entity`）
        //      ——路径不在 `<repo>/.worktrees/aria-*/<name>` 内即 fail-closed。
        //    交错窗口极窄：新 plan 必须在本函数（及其持锁的取代删除）完成后
        //    才可建立（单 plan 不变量 REQ-WPS-01 ＋ issue plan-slot 互斥），
        //    期间不可能出现「本 issue 同仓同一 worktree 路径被新 plan 重新
        //    启用」——新启用要经过同一闭包的前置删除，必然先消费或重写本清单。
        let deletion_store = EntityDeletionStore::new(app_paths.clone());
        let pending_entities = deletion_store.load_pending_worktree_entities(lease)?;
        if !pending_entities.is_empty() {
            remove_shared_worktree_entity_paths(&pending_entities).await?;
            deletion_store.remove_pending_worktree_entities(lease)?;
        }
        // ② 正常判定 → 落盘待删路径 → 真删实体 → 删记录 → 清待删清单。
        let shared_worktree_cleanup = cleanup_plan_shared_worktrees(
            app_paths,
            lease,
            manifest,
            plan_run,
            resource_leases,
            &plan_attempt_ids,
            SharedWorktreeEntityMode::DeleteEntity,
        )?;
        deletion_store
            .save_pending_worktree_entities(lease, &shared_worktree_cleanup.worktree_entities)?;
        remove_shared_worktree_entities(&shared_worktree_cleanup).await?;
        delete_shared_worktree_records(app_paths, &shared_worktree_cleanup)?;
        deletion_store.remove_pending_worktree_entities(lease)?;
        // worktree 实体残留复核（禁止「记录已删、实体目录还在」）。
        for entity_path in &shared_worktree_cleanup.worktree_entities {
            if entity_path.exists() {
                return Err(ProductStoreError::Io(format!(
                    "entity_deletion_residue: shared worktree entity {} still present",
                    entity_path.display()
                )));
            }
        }
        // coding attempts 及其运行资源（仅代际取代路径；用户面删除在门禁已拒）。
        if coding_gate == CodingGateMode::Cascade {
            purge_plan_coding_attempts(app_paths, project_id, issue_id, plan_id, manifest).await?;
        }
        // coding attempt 初始化残留锁与 journal 残留。
        purge_attempt_lock_residue(
            app_paths,
            project_id,
            issue_id,
            plan_id,
            &manifest.work_item_ids,
        )?;
        // run 本体与 workspace lifecycle 元数据（安全删除窗口内 unlink）。
        let run_store = CodingPlanRunStore::new(app_paths.clone());
        run_store.purge_plan_with_lease(lease)?;
        run_store.purge_workspace_lifecycle_with_lease(lease)?;
    }

    // advance 独立 engine checkpoint：纯 session 索引枚举，不整删全局根。
    if let Some(workspace_root) = workspace_root {
        let checkpoint_session_root = workspace_root
            .join("advance-engine-checkpoints")
            .join("workspace-sessions");
        for session_id in &manifest.session_ids {
            crate::product::lifecycle_store::remove_dir_all_if_exists(
                &checkpoint_session_root.join(session_id),
            )?;
        }
    }
    Ok(())
}

/// 该 plan 拥有的 coding attempts（group scope 锚定 plan ＋ 真实 work item 归属）。
///
/// 归属判定用真实 owner-key：group attempt 以 `work_item_group_id == plan_id`
/// 归属；single scope attempt 以其 `work_item_id` 落在本 plan 的 owned WI 集合
/// 归属。摘要缺项但真实绑定在位者照样纳入（REQ-WPS-03「摘要残缺但仍须清全」）。
///
/// **兜底（孤儿 attempt）**：attempt 记录自身不携带 plan 字段，归属只能经
/// 「group 绑定」或「work item 记录的 `source_work_item_plan_id`」反查。传入
/// 的 `work_item_ids` 来自 manifest 采集（plan 根/WI 记录已清时该集合为空），
/// 此时无 WI 记录的孤儿 attempt 会被漏掉 → 其运行目录/reviewer 资源/
/// worktree 实体成孤儿。故此处再按 WI 记录的真实 plan 归属兜底枚举。
pub(crate) fn plan_owned_attempts(
    coding_store: &CodingAttemptStore,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    work_item_ids: &[String],
) -> Result<Vec<CodingExecutionAttempt>, ProductStoreError> {
    let mut attempts: BTreeSet<String> = BTreeSet::new();
    for attempt in coding_store.list_attempts_for_plan(project_id, issue_id, plan_id)? {
        attempts.insert(attempt.id);
    }
    for work_item_id in work_item_ids {
        for attempt in
            coding_store.list_attempts_for_work_item(project_id, issue_id, work_item_id)?
        {
            // 他 plan 的 group attempt（group 绑定优先）不构成本 plan 的 attempt。
            if attempt
                .work_item_group_id
                .as_deref()
                .is_some_and(|group| group != plan_id)
            {
                continue;
            }
            attempts.insert(attempt.id);
        }
    }
    // 兜底：按 work item 记录的真实 plan 归属反查（覆盖无 WI 记录的孤儿
    // attempt——其 WI 记录仍在，只是没进 manifest 的 owned WI 集合）。
    let lifecycle = LifecycleStore::new(coding_store.paths());
    let plan_owned_work_items: BTreeSet<String> = lifecycle
        .list_work_items(project_id, issue_id)?
        .into_iter()
        .filter(|item| item.source_work_item_plan_id.as_deref() == Some(plan_id))
        .map(|item| item.id)
        .collect();
    if !plan_owned_work_items.is_empty() {
        for attempt in coding_store.list_attempts_for_issue(project_id, issue_id)? {
            // group attempt 的归属由 group 绑定决定（他 plan 的 group attempt
            // 不构成本 plan 的 attempt）。
            if attempt.work_item_group_id.is_some() {
                continue;
            }
            if plan_owned_work_items.contains(&attempt.work_item_id) {
                attempts.insert(attempt.id);
            }
        }
    }
    let mut resolved = Vec::with_capacity(attempts.len());
    for attempt_id in attempts {
        resolved.push(coding_store.get_attempt(project_id, issue_id, &attempt_id)?);
    }
    resolved.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(resolved)
}

/// 未终结 attempt 收口（REQ-WPS-06：删除前先收口到终止终态）。
///
/// 复用既有 attempt 终止语义（`update_attempt_status` → Aborted）。已终结
/// attempt（Completed/Failed/Aborted）不动。收口失败即上抛——调用方在任何
/// 删除动作之前失败（零副作用）。
pub(crate) fn terminate_plan_coding_attempts(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    work_item_ids: &[String],
) -> Result<(), ProductStoreError> {
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    for attempt in plan_owned_attempts(&coding_store, project_id, issue_id, plan_id, work_item_ids)?
    {
        if matches!(
            attempt.status,
            CodingAttemptStatus::Completed
                | CodingAttemptStatus::Failed
                | CodingAttemptStatus::Aborted
        ) {
            continue;
        }
        coding_store.update_attempt_status(
            project_id,
            issue_id,
            &attempt.id,
            CodingAttemptStatus::Aborted,
        )?;
    }
    Ok(())
}

/// coding attempts 及其运行资源的物理清除（REQ-WPS-03/REQ-WPS-06）。
///
/// 每条 attempt：worktree 实体（`git worktree remove --force` ＋ prune）→
/// attempt 记录与其全部运行子目录（units/runs/events/…）→ attempt 残留锁。
/// 随后按 attempt 集合物理清除 reviewer windows/snapshots/leases（含归档）。
pub(crate) async fn purge_plan_coding_attempts(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    manifest: &EntityDeletionManifest,
) -> Result<(), ProductStoreError> {
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let attempts = plan_owned_attempts(
        &coding_store,
        project_id,
        issue_id,
        plan_id,
        &manifest.work_item_ids,
    )?;
    let attempt_ids: Vec<String> = attempts.iter().map(|attempt| attempt.id.clone()).collect();
    for attempt in &attempts {
        remove_attempt_worktree_entity(app_paths, attempt).await?;
    }
    for attempt in &attempts {
        let work_item_ids = collect_attempt_work_item_ids(&coding_store, attempt);
        coding_store.delete_attempt(project_id, issue_id, &attempt.id)?;
        purge_attempt_lock_residue_for_attempt(app_paths, attempt, &work_item_ids);
    }
    // reviewer 资源（窗口/租约/通道/快照，含归档）按 attempt 归属物理清除。
    ReviewerResourcePurger::new(app_paths).purge_for_attempts(&attempt_ids)?;
    Ok(())
}

/// attempt 覆盖的 work item 集合（group 取物化 units；single 取锚点）。
fn collect_attempt_work_item_ids(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Vec<String> {
    if matches!(
        attempt.scope,
        crate::product::coding_models::CodingAttemptScope::WorkItemGroup
    ) {
        let unit_ids = coding_store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .map(|units| {
                units
                    .into_iter()
                    .map(|unit| unit.logical_work_item_id)
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        if unit_ids.is_empty() {
            vec![attempt.work_item_id.clone()]
        } else {
            unit_ids
        }
    } else {
        vec![attempt.work_item_id.clone()]
    }
}

/// 删除 attempt 的 worktree 实体（git 注册项 ＋ 目录）。
///
/// 物理仓路径由 worktree 自身的 `.git` gitfile 反解（linked worktree：
/// `<main>/.git/worktrees/<name>` → `<main>`）；**不可反解时仍删目录残留**
/// （与 [`remove_shared_worktree_entity`] 同口径）——直接跳过会把目录留成
/// 孤儿：重入时 attempt 记录已删，无任何路径能再定位并清掉它。
///
/// **兜底删除受 aria worktree 命名空间守卫约束**（R3，数据安全）：路径必须
/// 位于 `<repo>/.worktrees/aria-work-items|aria-issues|aria-pointer/…`，
/// 否则 fail-closed（错误上抛、记录保留待人工），绝不 `remove_dir_all`。
async fn remove_attempt_worktree_entity(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<(), ProductStoreError> {
    let Some(worktree_path) = attempt.worktree_path.as_ref() else {
        return Ok(());
    };
    if !worktree_path.exists() {
        return Ok(());
    }
    if let Some(repo_path) = resolve_main_repo_from_worktree(worktree_path) {
        let git = GitWorkspaceService::new();
        git.remove_worktree(&repo_path, worktree_path)
            .await
            .map_err(|error| {
                ProductStoreError::Io(format!(
                    "remove worktree {}: {error}",
                    worktree_path.display()
                ))
            })?;
        git.prune_worktrees(&repo_path).await.map_err(|error| {
            ProductStoreError::Io(format!("prune worktrees {}: {error}", repo_path.display()))
        })?;
    }
    // git 注册项已删但目录仍在（反解失败/删除未清空）→ 目录兜底删除，
    // 杜绝「记录已删、实体目录成孤儿」。
    //
    // R3（数据安全）：反解失败时记录里的路径未经任何校验，若它指向主仓根
    // （`.git` 是目录 → 反解失败）直接 `remove_dir_all` 会**整仓删除**。
    // 故兜底前强制命名空间守卫：只删
    // `<repo>/.worktrees/aria-work-items|aria-issues|aria-pointer/…` 内的实体；
    // 不满足即 fail-closed（错误上抛 → attempt 记录保留，待人工处理）。
    if worktree_path.exists() {
        crate::product::git_workspace_service::ensure_worktree_namespace_path(worktree_path)
            .map_err(|error| {
                ProductStoreError::Io(format!(
                    "refuse to remove worktree entity {}: {error} (record kept for manual recovery)",
                    worktree_path.display()
                ))
            })?;
        std::fs::remove_dir_all(worktree_path).map_err(|error| {
            ProductStoreError::Io(format!("remove {}: {error}", worktree_path.display()))
        })?;
    }
    let _ = app_paths;
    Ok(())
}

/// 由 worktree 的 `.git` gitfile 反解主仓路径；不可反解返回 None。
fn resolve_main_repo_from_worktree(worktree_path: &Path) -> Option<PathBuf> {
    let dot_git = worktree_path.join(".git");
    let content = std::fs::read_to_string(&dot_git).ok()?;
    let target = content
        .lines()
        .find_map(|line| line.trim().strip_prefix("gitdir:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let gitdir = PathBuf::from(target);
    // linked worktree：`<main>/.git/worktrees/<name>` → `<main>`。
    let worktrees_dir = gitdir.parent()?;
    if worktrees_dir.file_name().and_then(|value| value.to_str()) != Some("worktrees") {
        return None;
    }
    let git_common = worktrees_dir.parent()?;
    let main = git_common.parent()?;
    Some(main.to_path_buf())
}

/// attempt 删除后的残留锁清理（不误伤其他 plan 的锁）。
fn purge_attempt_lock_residue_for_attempt(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    work_item_ids: &[String],
) {
    use crate::product::lifecycle_store::remove_file_if_exists;

    let coding_attempts_root = app_paths
        .issue_root(&attempt.project_id, &attempt.issue_id)
        .join("coding-attempts");
    let _ = remove_file_if_exists(&coding_attempts_root.join(format!(".{}.json.lock", attempt.id)));
    if matches!(
        attempt.scope,
        crate::product::coding_models::CodingAttemptScope::WorkItemGroup
    ) {
        let _ = remove_file_if_exists(
            &coding_attempts_root.join(".group-initialization-arbitration.lock"),
        );
    }
    let locks_dir = coding_attempts_root.join("work-item-attempt-locks");
    for work_item_id in work_item_ids {
        let _ = remove_file_if_exists(&locks_dir.join(work_item_id));
        let _ = remove_file_if_exists(&locks_dir.join(format!(".{work_item_id}.lock")));
    }
}

/// 本 issue `coding-attempts/` 目录下与该 plan 相关的残留锁与初始化产物。
///
/// 全程容错：文件不存在视为成功，单项失败不影响其余清理。
/// `work_item_ids` 限定 `work-item-attempt-locks` 的清理范围（不误伤他 plan）。
pub(crate) fn purge_attempt_lock_residue(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    work_item_ids: &[String],
) -> Result<(), ProductStoreError> {
    use crate::product::lifecycle_store::remove_file_if_exists;

    let coding_attempts_root = app_paths
        .issue_root(project_id, issue_id)
        .join("coding-attempts");

    // group-initialization-arbitration 文件 + lock（issue 级仲裁）。
    let _ = remove_file_if_exists(&coding_attempts_root.join("group-initialization-arbitration"));
    let _ =
        remove_file_if_exists(&coding_attempts_root.join(".group-initialization-arbitration.lock"));

    // group-initializations/<plan_id>.json + lock + per-target 子目录。
    let journal_dir = coding_attempts_root.join("group-initializations");
    let _ = remove_file_if_exists(&journal_dir.join(format!("{plan_id}.json")));
    let _ = remove_file_if_exists(&journal_dir.join(format!(".{plan_id}.json.lock")));
    let plan_journal_dir = journal_dir.join(plan_id);
    if plan_journal_dir.is_dir() {
        let _ = std::fs::remove_dir_all(&plan_journal_dir);
    }

    // work-item-attempt-locks/：按本 plan 的 work_item 精确删除（不整目录清空）。
    let locks_dir = coding_attempts_root.join("work-item-attempt-locks");
    for work_item_id in work_item_ids {
        let _ = remove_file_if_exists(&locks_dir.join(work_item_id));
        let _ = remove_file_if_exists(&locks_dir.join(format!(".{work_item_id}.lock")));
    }

    // 顶层 `.*.lock` 孤儿清理：目标文件已不存在才删（保留 active attempt 运行时锁）。
    if let Ok(entries) = std::fs::read_dir(&coding_attempts_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(target_name) = name
                .strip_prefix('.')
                .and_then(|suffix| suffix.strip_suffix(".lock"))
            else {
                continue;
            };
            if !coding_attempts_root.join(target_name).exists() {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    Ok(())
}

/// shared-worktree 实体的处置口径（[`cleanup_plan_shared_worktrees`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedWorktreeEntityMode {
    /// 真删 worktree 实体（git 注册项 ＋ 目录）：代际取代闭包口径。
    ///
    /// **实体先删、记录后删**：实体删除失败即中止，记录仍在 → 重入仍能枚举
    /// 到该实体并补删。先删记录会让重入失去定位，实体残留成孤儿（违反
    /// 「禁止根已删、产物还在」）。
    DeleteEntity,
    /// 只删记录、保留实体：round-close 重置口径（worktree 目录被下一轮复用）。
    RecordOnly,
}

/// shared-worktree 清理的待办（判定已在 cleanup lease 内完成）。
///
/// **记录删除推迟到实体真删之后**（[`delete_shared_worktree_records`]）：
/// 先删记录会让重入失去实体定位——实体删除失败后重入已无记录可枚举，实体
/// 残留成孤儿（违反「禁止根已删、产物还在」）。
pub(crate) struct SharedWorktreeCleanupOutcome<'a> {
    /// cleanup lease 守卫（持有到记录删除完成——并发 takeover 被挡住）。
    _guards: Vec<CleanupLeaseGuard<'a>>,
    /// 待真删的 worktree 实体路径（`DeleteEntity` 口径；`RecordOnly` 为空）。
    worktree_entities: Vec<PathBuf>,
    /// 待删的记录键（`None`＝legacy issue 布局）。
    record_keys: Vec<Option<LogicalRepositoryId>>,
    project_id: String,
    issue_id: String,
}

/// 目标仓资源清理（shared-worktree 记录 ＋ 锁 ＋ worktree 实体）——**判定阶段**。
///
/// 仅 manifest 目标仓 ∪ plan-run 冻结 target ∪ legacy issue 布局（None 键），
/// **不整 issue 枚举删除**（REQ-WPS-03「共享工作树仍被其他 plan 使用」）。
/// 每个资源在稳定 repo resource cleanup lease 内复核真实 owners：他 Plan 仍
/// 使用（存活、非本 scope、非墓碑 scope）则保留记录，但**释放本 plan 持有的
/// 锁**（置空 owner ＋ status=Ready）；目标专属才进入待删集合。
///
/// 本阶段**不删记录**：调用方须先真删 worktree 实体
/// （[`remove_shared_worktree_entities`]），再删记录
/// （[`delete_shared_worktree_records`]）——顺序不可颠倒。
pub(crate) fn cleanup_plan_shared_worktrees<'a>(
    app_paths: &ProductAppPaths,
    lease: &EntityDeletionLease,
    manifest: &EntityDeletionManifest,
    plan_run: Option<&CodingPlanRun>,
    resource_leases: &'a [(SharedResourceKey, SharedResourceCleanupLease)],
    plan_attempt_ids: &[String],
    entity_mode: SharedWorktreeEntityMode,
) -> Result<SharedWorktreeCleanupOutcome<'a>, ProductStoreError> {
    let scope = lease.scope();
    if scope.kind != DeletionEntityKind::Plan {
        return Err(ProductStoreError::InvalidRecord {
            kind: "entity_deletion_manifest",
            reason: "shared worktree cleanup only belongs to plan deletions".to_string(),
        });
    }
    let (project_id, issue_id) = (scope.project_id.as_str(), scope.issue_id.as_str());
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let operations = EntityOperationStore::new(app_paths.clone());
    let deletion_records = EntityDeletionStore::new(app_paths.clone());
    let mut guards: Vec<CleanupLeaseGuard<'a>> = Vec::new();
    let mut worktree_entities: Vec<PathBuf> = Vec::new();
    let mut record_keys: Vec<Option<LogicalRepositoryId>> = Vec::new();

    let repositories = plan_shared_worktree_repository_ids(&lifecycle, scope, manifest, plan_run)?;
    let candidates = plan_shared_worktree_candidate_ids(&lifecycle, scope, manifest, plan_run)?;

    for repository_id in repositories {
        let logical_repository_id = repository_id.map(LogicalRepositoryId);
        let key = SharedResourceKey {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: logical_repository_id,
        };
        // cleanup lease 全程持有（含实体真删）——并发 takeover 被挡住。
        let guard = acquire_cleanup_lease(&operations, resource_leases, &key)?;
        let record = match logical_repository_id {
            Some(repository_id) => {
                lifecycle.get_repo_shared_worktree(project_id, issue_id, repository_id)?
            }
            None => lifecycle.get_issue_shared_worktree(project_id, issue_id)?,
        };
        if let Some(record) = record {
            let ownership = classify_shared_worktree_ownership(
                app_paths,
                &lifecycle,
                &deletion_records,
                scope,
                &record,
                plan_run,
                plan_attempt_ids,
                candidates.contains(&repository_id),
            )?;
            match ownership {
                SharedWorktreeOwnership::Unrelated => {}
                SharedWorktreeOwnership::OtherLiveOwner => {
                    // 共享资源仍被他人使用：保留记录，但释放本 plan 持有的锁。
                    release_shared_worktree_lock(
                        app_paths,
                        project_id,
                        issue_id,
                        logical_repository_id,
                        record.current_lock_owner_id.as_deref(),
                        plan_attempt_ids,
                    )?;
                }
                SharedWorktreeOwnership::ClaimedByThisPlan => {
                    // 目标专属：**不提前释放锁**——记录即将删除（删除即释放其
                    // 锁）；实体删除失败时锁原样保留，重入仍能按真实 owner-key
                    // 判定归属并补删实体。
                    if entity_mode == SharedWorktreeEntityMode::DeleteEntity {
                        worktree_entities.push(record.worktree_path.clone());
                    }
                    record_keys.push(logical_repository_id);
                }
            }
        }
        guards.push(guard);
    }
    Ok(SharedWorktreeCleanupOutcome {
        _guards: guards,
        worktree_entities,
        record_keys,
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
    })
}

/// 真删已判定的 worktree 实体（git 注册项 ＋ 目录）。
///
/// 失败即中止：记录仍在 → 重入仍能判定到该实体并补删（绝不留下「记录已删、
/// 实体还在」的孤儿）。
pub(crate) async fn remove_shared_worktree_entities(
    outcome: &SharedWorktreeCleanupOutcome<'_>,
) -> Result<(), ProductStoreError> {
    remove_shared_worktree_entity_paths(&outcome.worktree_entities).await
}

/// 按路径清单真删 worktree 实体（失败重入补删用）。
pub(crate) async fn remove_shared_worktree_entity_paths(
    worktree_paths: &[PathBuf],
) -> Result<(), ProductStoreError> {
    for worktree_path in worktree_paths {
        remove_shared_worktree_entity(worktree_path).await?;
    }
    Ok(())
}

/// 删除已判定的 shared-worktree 记录（实体已清，或本口径明确保留实体）。
pub(crate) fn delete_shared_worktree_records(
    app_paths: &ProductAppPaths,
    outcome: &SharedWorktreeCleanupOutcome<'_>,
) -> Result<(), ProductStoreError> {
    let lifecycle = LifecycleStore::new(app_paths.clone());
    for repository_id in &outcome.record_keys {
        match repository_id {
            Some(repository_id) => lifecycle.delete_repo_shared_worktree(
                &outcome.project_id,
                &outcome.issue_id,
                *repository_id,
            )?,
            None => {
                lifecycle.delete_issue_shared_worktree(&outcome.project_id, &outcome.issue_id)?
            }
        }
    }
    Ok(())
}

/// cleanup lease 守卫：编排层已持有则借用（不重复取锁），否则就地 try 取。
///
/// 未持有的 key 现取（try 非阻塞，争用 Busy fail-closed——不静默跳过留孤儿
/// 资源）。借用态不释放编排层的 lease（其生命周期由编排层决定）。
enum CleanupLeaseGuard<'a> {
    Held {
        _lease: &'a SharedResourceCleanupLease,
    },
    Acquired {
        _lease: SharedResourceCleanupLease,
    },
}

fn acquire_cleanup_lease<'a>(
    operations: &EntityOperationStore,
    resource_leases: &'a [(SharedResourceKey, SharedResourceCleanupLease)],
    key: &SharedResourceKey,
) -> Result<CleanupLeaseGuard<'a>, ProductStoreError> {
    if let Some((_, lease)) = resource_leases.iter().find(|(held, _)| held == key) {
        return Ok(CleanupLeaseGuard::Held { _lease: lease });
    }
    let lease = operations
        .try_acquire_shared_resource_cleanup(key)
        .map_err(|error| match error {
            EntityOperationError::Store(inner) => inner,
            _ => ProductStoreError::Conflict {
                kind: "entity_busy",
                id: key
                    .repository_id
                    .map(|id| id.0.to_string())
                    .unwrap_or_default(),
            },
        })?;
    Ok(CleanupLeaseGuard::Acquired { _lease: lease })
}

/// 释放 shared-worktree 锁（owner 置空 ＋ status=Ready）——**取代删除闭包内的
/// 专属释放**。
///
/// 只释放**本 plan 持有**的锁（owner ∈ `plan_attempt_ids`）：他人 attempt
/// 持有的锁不动（共享资源仍被他人使用）。owner 不匹配即幂等返回。
///
/// 与 `LifecycleStore::release_*_worktree_lock_by_owner` 的差别：后者走
/// shared-worktree **写准入**，而写准入对本 scope 的 purging 墓碑 claim
/// fail-closed 拒绝（`entity_deleting`）——删除闭包正是该 claim 的持有者，
/// 走写准入会把自己锁在门外（真机 D2 根因：锁 owner 残留）。此处按删除
/// 闭包的专属窗口直接改写记录（owner 置空 ＋ status=Ready），语义与写准入
/// 版本一致，资源 key 的 cleanup lease 仍由编排层持有。
fn release_shared_worktree_lock(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    repository_id: Option<LogicalRepositoryId>,
    owner_id: Option<&str>,
    plan_attempt_ids: &[String],
) -> Result<(), ProductStoreError> {
    let Some(owner_id) = owner_id else {
        return Ok(());
    };
    if !plan_attempt_ids
        .iter()
        .any(|attempt_id| attempt_id == owner_id)
    {
        return Ok(());
    }
    let path = match repository_id {
        Some(repository_id) => app_paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("shared-worktrees")
            .join(format!("{}.json", repository_id.0)),
        None => app_paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("issue-shared-worktree.json"),
    };
    if !path.is_file() {
        return Ok(());
    }
    let mut record: crate::product::models::IssueSharedWorktree = read_json(&path)?;
    if record.current_lock_owner_id.as_deref() != Some(owner_id) {
        return Ok(());
    }
    record.current_active_work_item_id = None;
    record.current_lock_owner_id = None;
    record.status = crate::product::models::IssueSharedWorktreeStatus::Ready;
    record.updated_at = chrono::Utc::now().to_rfc3339();
    write_json(&path, &record)
}

/// shared-worktree 记录的归属判定（真实 owner-key）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SharedWorktreeOwnership {
    /// 本 plan 独占（归属解析命中本 scope，或锁持有者是本 plan 的 attempt）。
    ClaimedByThisPlan,
    /// 他 plan 仍使用（存活 owner 或他人持锁）→ 记录保留。
    OtherLiveOwner,
    /// 与本 plan 无关 → 完全不动（绝不误伤）。
    Unrelated,
}

/// shared-worktree 归属判定：谁在用这条记录。
fn classify_shared_worktree_ownership(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    deletion_records: &EntityDeletionStore,
    scope: &EntityScope,
    record: &crate::product::models::IssueSharedWorktree,
    plan_run: Option<&CodingPlanRun>,
    plan_attempt_ids: &[String],
    is_candidate: bool,
) -> Result<SharedWorktreeOwnership, ProductStoreError> {
    let mut claims_this_plan = false;
    let mut has_other_owner = false;
    for owner in lifecycle.resolve_shared_resource_owners(
        &scope.project_id,
        &scope.issue_id,
        record.target_repository_id,
    )? {
        if owner == *scope {
            claims_this_plan = true;
            continue;
        }
        // 墓碑（purging/deleted）scope 不是存活引用方——同波删除或已删实体。
        if deletion_records.get(&owner)?.is_some() {
            continue;
        }
        has_other_owner = true;
    }
    if has_other_owner {
        return Ok(SharedWorktreeOwnership::OtherLiveOwner);
    }
    if let Some(owner_id) = record.current_lock_owner_id.as_deref() {
        let own_lock = plan_attempt_ids
            .iter()
            .any(|attempt_id| attempt_id == owner_id);
        // 本 plan 的 run 冻结集/selection 同样构成本 plan 持有的锁。
        let frozen = plan_run.is_some_and(|run| {
            run.target_attempts
                .iter()
                .any(|frozen| frozen.attempt_id == owner_id)
                || run
                    .selection
                    .as_ref()
                    .is_some_and(|selection| selection.attempt_id == owner_id)
        });
        if own_lock || frozen {
            return Ok(SharedWorktreeOwnership::ClaimedByThisPlan);
        }
        // 锁 owner 不属本 plan 的已知 attempt 集合：
        // ① 记录归属未知（`claims_this_plan=false`）→ 按在途 writer 保留
        //    （REQ-WPS-03 不误伤：他 plan 直接改写 owner 的记录绝不清）；
        // ② 记录已被本 plan 的其他证据主张（`claims_this_plan`：记录 active/
        //    last-completed WI 归属本 plan，或路径界定的 journal 归属本 plan）
        //    且该 owner 已无 attempt 记录 → 半残现场的孤儿残留（attempt 主体被
        //    外部清掉、锁 owner 悬空），随闭包清。判据可靠：真实 writer MUST 先
        //    持久化 attempt 记录再取 worktree 锁（journal `AttemptPersisted`
        //    先于 `WorktreeBound`），故「无 attempt 记录」的 owner 不可能是存活
        //    writer；按「在途 writer」保留会让半残现场永远收敛不了
        //    （违反 REQ-WPS-03「半残仍须清全」）。
        if claims_this_plan && !lock_owner_attempt_exists(app_paths, scope, owner_id)? {
            return Ok(SharedWorktreeOwnership::ClaimedByThisPlan);
        }
        return Ok(SharedWorktreeOwnership::OtherLiveOwner);
    }
    if claims_this_plan {
        return Ok(SharedWorktreeOwnership::ClaimedByThisPlan);
    }
    // 候选仓（manifest 目标仓 ∪ run 冻结 target）：本 plan 的目标资源，无他
    // 存活 owner 即随闭包清（既有 D1.3c 语义）。
    if is_candidate {
        return Ok(SharedWorktreeOwnership::ClaimedByThisPlan);
    }
    Ok(SharedWorktreeOwnership::Unrelated)
}

/// 锁 owner 是否有对应的 attempt 记录（真实在途 writer 的前提）。
///
/// 只有 attempt 记录仍在的 owner 才可能是在途 writer（先 `AttemptPersisted`
/// 后 `WorktreeBound`）；记录缺失即残留孤儿 owner。真实 IO 错误仍上抛。
fn lock_owner_attempt_exists(
    app_paths: &ProductAppPaths,
    scope: &EntityScope,
    owner_id: &str,
) -> Result<bool, ProductStoreError> {
    let attempts = CodingAttemptStore::new(app_paths.clone());
    match attempts.get_attempt(&scope.project_id, &scope.issue_id, owner_id) {
        Ok(_) => Ok(true),
        Err(ProductStoreError::NotFound { .. }) => Ok(false),
        Err(error) => Err(error),
    }
}

/// 目标仓集合（shared-worktree 记录的候选 key）。
///
/// manifest 目标仓 ∪ plan-run 冻结 target ∪ **本 issue 全部 repo 级
/// shared-worktree 记录**（真实归属由 [`classify_shared_worktree_ownership`]
/// 判定：与本 plan 无关者完全不动）。legacy issue 布局（None 键）记录存在
/// 才含 None 键。
pub(crate) fn plan_shared_worktree_repository_ids(
    store: &LifecycleStore,
    scope: &EntityScope,
    manifest: &EntityDeletionManifest,
    plan_run: Option<&CodingPlanRun>,
) -> Result<BTreeSet<Option<uuid::Uuid>>, ProductStoreError> {
    let (project_id, issue_id) = (scope.project_id.as_str(), scope.issue_id.as_str());
    let mut candidate: BTreeSet<Option<uuid::Uuid>> = BTreeSet::new();
    let mut repositories: BTreeSet<Option<uuid::Uuid>> = BTreeSet::new();
    for target in &manifest.target_repository_ids {
        let parsed =
            uuid::Uuid::parse_str(target).map_err(|_| ProductStoreError::InvalidRecord {
                kind: "entity_target_repository",
                reason: format!("owned work item target repository is not a logical id: {target}"),
            })?;
        candidate.insert(Some(parsed));
    }
    if let Some(run) = plan_run {
        for binding in &run.target_attempts {
            let parsed = uuid::Uuid::parse_str(&binding.target_repository_id).map_err(|_| {
                ProductStoreError::InvalidRecord {
                    kind: "coding_plan_run_target",
                    reason: format!(
                        "plan run frozen target is not a logical repository id: {}",
                        binding.target_repository_id
                    ),
                }
            })?;
            candidate.insert(Some(parsed));
        }
    }
    repositories.extend(candidate.iter().copied());
    for repository_id in store.list_repo_shared_worktrees(project_id, issue_id)? {
        repositories.insert(Some(repository_id.0));
    }
    if store
        .get_issue_shared_worktree(project_id, issue_id)?
        .is_some()
    {
        repositories.insert(None);
    }
    Ok(repositories)
}

/// 候选目标仓集合（manifest 目标仓 ∪ plan-run 冻结 target ∪ legacy issue 布局）。
///
/// 候选＝本 plan 的目标资源：无他存活 owner 即随闭包清；非候选（仅因 issue
/// 内存在而枚举到的记录）只在真实归属本 plan 时才动。
pub(crate) fn plan_shared_worktree_candidate_ids(
    store: &LifecycleStore,
    scope: &EntityScope,
    manifest: &EntityDeletionManifest,
    plan_run: Option<&CodingPlanRun>,
) -> Result<BTreeSet<Option<uuid::Uuid>>, ProductStoreError> {
    let (project_id, issue_id) = (scope.project_id.as_str(), scope.issue_id.as_str());
    let mut candidate: BTreeSet<Option<uuid::Uuid>> = BTreeSet::new();
    for target in &manifest.target_repository_ids {
        let parsed =
            uuid::Uuid::parse_str(target).map_err(|_| ProductStoreError::InvalidRecord {
                kind: "entity_target_repository",
                reason: format!("owned work item target repository is not a logical id: {target}"),
            })?;
        candidate.insert(Some(parsed));
    }
    if let Some(run) = plan_run {
        for binding in &run.target_attempts {
            let parsed = uuid::Uuid::parse_str(&binding.target_repository_id).map_err(|_| {
                ProductStoreError::InvalidRecord {
                    kind: "coding_plan_run_target",
                    reason: format!(
                        "plan run frozen target is not a logical repository id: {}",
                        binding.target_repository_id
                    ),
                }
            })?;
            candidate.insert(Some(parsed));
        }
    }
    if store
        .get_issue_shared_worktree(project_id, issue_id)?
        .is_some()
    {
        candidate.insert(None);
    }
    Ok(candidate)
}

/// 单个 shared worktree 实体的物理清除（git 注册项 ＋ 目录）。
///
/// 主仓路径由 worktree 的 `.git` gitfile 反解；不可反解时只删目录残留——
/// 且**必须落在 aria worktree 命名空间内**（R3，数据安全：反解失败时记录里
/// 的原始路径未经校验，指向主仓根即整仓删除），否则 fail-closed。
async fn remove_shared_worktree_entity(worktree_path: &Path) -> Result<(), ProductStoreError> {
    if !worktree_path.exists() {
        return Ok(());
    }
    if let Some(repo_path) = resolve_main_repo_from_worktree(worktree_path) {
        let git = GitWorkspaceService::new();
        git.remove_worktree(&repo_path, worktree_path)
            .await
            .map_err(|error| {
                ProductStoreError::Io(format!(
                    "remove shared worktree {}: {error}",
                    worktree_path.display()
                ))
            })?;
        git.prune_worktrees(&repo_path).await.map_err(|error| {
            ProductStoreError::Io(format!("prune worktrees {}: {error}", repo_path.display()))
        })?;
    }
    // R3（数据安全）：同 [`remove_attempt_worktree_entity`]——兜底删除前强制
    // 命名空间守卫，指向主仓根/他处目录一律 fail-closed（记录保留待人工）。
    if worktree_path.exists() {
        crate::product::git_workspace_service::ensure_worktree_namespace_path(worktree_path)
            .map_err(|error| {
                ProductStoreError::Io(format!(
                    "refuse to remove shared worktree entity {}: {error} (record kept for manual recovery)",
                    worktree_path.display()
                ))
            })?;
        std::fs::remove_dir_all(worktree_path).map_err(|error| {
            ProductStoreError::Io(format!("remove {}: {error}", worktree_path.display()))
        })?;
    }
    Ok(())
}

/// 释放本 plan 持有的 shared-worktree 锁（原地演进路径专用）。
///
/// 旧代 attempt 被删除后，其持有的 shared-worktree 锁 owner 字段成为孤儿
/// （owner 指向不存在的 attempt）——必须释放（置空 owner ＋ status=Ready），
/// 否则新代永远拿不到锁。只释放 owner ∈ `plan_attempt_ids` 的锁；他人持有
/// 的锁不动。记录本身保留（他 plan 仍可能使用）。
///
/// **持 repo resource cleanup lease**：`release_shared_worktree_lock` 是
/// 「读 → 校验 → 写」，记录自身不取 flock——不持 lease 时并发的 takeover
/// 会在读改写之间被覆盖（锁 owner 被写回旧值，新代仍拿不到锁）。
pub(crate) fn release_plan_shared_worktree_locks(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_attempt_ids: &[String],
) -> Result<(), ProductStoreError> {
    if plan_attempt_ids.is_empty() {
        return Ok(());
    }
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let operations = EntityOperationStore::new(app_paths.clone());
    for repository_id in lifecycle.list_repo_shared_worktrees(project_id, issue_id)? {
        let key = SharedResourceKey {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: Some(repository_id),
        };
        // cleanup lease 全程持有（读记录前取）——并发 takeover 被挡住。
        let _lease = operations
            .try_acquire_shared_resource_cleanup(&key)
            .map_err(|error| match error {
                EntityOperationError::Store(inner) => inner,
                _ => ProductStoreError::Conflict {
                    kind: "entity_busy",
                    id: repository_id.0.to_string(),
                },
            })?;
        let Some(record) =
            lifecycle.get_repo_shared_worktree(project_id, issue_id, repository_id)?
        else {
            continue;
        };
        release_shared_worktree_lock(
            app_paths,
            project_id,
            issue_id,
            Some(repository_id),
            record.current_lock_owner_id.as_deref(),
            plan_attempt_ids,
        )?;
    }
    // legacy issue 布局（None 键）。
    let key = SharedResourceKey {
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
        repository_id: None,
    };
    let _lease = operations
        .try_acquire_shared_resource_cleanup(&key)
        .map_err(|error| match error {
            EntityOperationError::Store(inner) => inner,
            _ => ProductStoreError::Conflict {
                kind: "entity_busy",
                id: issue_id.to_string(),
            },
        })?;
    if let Some(record) = lifecycle.get_issue_shared_worktree(project_id, issue_id)? {
        release_shared_worktree_lock(
            app_paths,
            project_id,
            issue_id,
            None,
            record.current_lock_owner_id.as_deref(),
            plan_attempt_ids,
        )?;
    }
    Ok(())
}

/// 收集 issue 下全部存活 plan id（legacy 与 schema-v2 两路）。
///
/// legacy：`{issue_root}/issue-work-item-plans/{plan_id}.json`。
/// schema-v2：`{issue_root}/work-item-revisions/{plan_id}/lineage.json`。
pub(crate) fn list_live_plan_ids(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> Result<Vec<String>, ProductStoreError> {
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for plan in lifecycle.list_issue_work_item_plans(project_id, issue_id)? {
        ids.insert(plan.id);
    }
    let revisions_root = app_paths
        .issue_root(project_id, issue_id)
        .join("work-item-revisions");
    if let Ok(entries) = std::fs::read_dir(&revisions_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if path.join("lineage.json").is_file() {
                ids.insert(name.to_string());
            }
        }
    }
    Ok(ids.into_iter().collect())
}

/// 落盘失败事实（REQ-WPS-04：可定位的失败清单 ＋ needs_human 入口）。
pub(crate) fn record_pending_failure(
    app_paths: &ProductAppPaths,
    lease: &EntityDeletionLease,
    failed_items: Vec<String>,
    message: String,
    trigger_reason: Option<&str>,
) -> Result<(), ProductStoreError> {
    let scope = lease.scope();
    let failure = EntityDeletionFailure {
        scope: scope.clone(),
        needs_human: true,
        failed_items,
        message,
        trigger_reason: trigger_reason.map(str::to_string),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    EntityDeletionStore::new(app_paths.clone()).save_pending_failure(lease, &failure)
}

#[cfg(test)]
mod tests;
