//! plan 代际取代（单 plan 不变量，REQ-WPS-01~07）。
//!
//! # 契约要点
//!
//! - **单 plan 不变量**：同一 issue 任一时刻至多一个存活 plan（legacy 与
//!   schema-v2 两路都算）。任何建立 plan 的路径 MUST 先把旧 plan 完整删除，
//!   旧 plan 删除未成功前 MUST NOT 建立新 plan（REQ-WPS-01）。
//! - **三个触发点**：re-enable 轮换 `prepare_intent_id`、重新 prepare、重新
//!   编译确认。三者共用本模块的同一取代编排（同一闭包、同一失败语义、同一
//!   幂等语义），并记录触发原因与旧 plan 身份（REQ-WPS-02）。
//! - **fail-closed**：任一子树失败 → 不报成功、不建立新 plan、plan 根保留为
//!   可达的 `purging`（needs_human）入口并持久化失败清单（REQ-WPS-04）。
//! - **幂等**：完成后重放返回「已删除」，墓碑数量/时间不变；进行中再次提交不
//!   并发第二份清理（REQ-WPS-05）。
//! - **只留无内容事实**：`entity-deletions/plan/{id}.json` 只含 scope/kind/id/
//!   时间/状态/触发原因，绝不保留历史 plan 实体（REQ-WPS-07）。

use std::path::Path;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_plan_run::CodingPlanRunStore;
use crate::product::entity_deletion_cascade::{
    CodingGateMode, PlanSupersedeReason, PlanSupersedeReport, list_live_plan_ids,
    purge_attempt_lock_residue, purge_entity_closure, record_pending_failure,
    terminate_plan_coding_attempts,
};
use crate::product::entity_deletion_store::{
    DeletionEntityKind, DeletionPhase, EntityDeletionManifest, EntityDeletionStore, EntityScope,
};
use crate::product::entity_operation_store::{
    EntityDeletionLease, EntityOperationError, EntityOperationStore, SharedResourceCleanupLease,
    SharedResourceKey,
};
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::LogicalRepositoryId;

use super::entity_deletion_cascade::CascadeDeleteError;

/// 代际取代入口：删除 issue 下所有**非保留**的存活 plan（先删旧，再建新）。
///
/// `keep_plan_ids` 是本代要保留的 plan（原地演进路径传入本 plan id；建立
/// 新代时传空集）。保留项只清运行资源（attempts/worktree/reviewer/锁），
/// 不删其根。
///
/// 返回后调用方**才可**建立新 plan；删除失败时调用方 MUST NOT 建立新 plan。
pub(crate) async fn ensure_single_plan_generation(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    keep_plan_ids: &[String],
    reason: PlanSupersedeReason,
    workspace_root: Option<&Path>,
) -> Result<PlanSupersedeReport, CascadeDeleteError> {
    // 保留项是否清运行资源：只有「重新编译确认」的原地演进路径需要（旧代
    // 运行资源必须随旧代清掉）；其它触发点的保留项是**当前存活**的 plan
    // （bound prepare 的既有绑定），清它的运行资源会误杀在跑的 coding。
    let purge_kept_runtime = reason == PlanSupersedeReason::RecompileConfirm;
    let operations = EntityOperationStore::new(app_paths.clone());
    let mut report = PlanSupersedeReport::default();

    // ① issue 级 plan 作用域互斥内采集存活 plan（两路存储）。
    let live_plan_ids = operations
        .with_issue_relations(project_id, issue_id, || {
            list_live_plan_ids(app_paths, project_id, issue_id)
        })
        .map_err(CascadeDeleteError::Store)?;

    // ② 逐个完整删除旧 plan（先子树后根；失败即 fail-closed）。
    for plan_id in live_plan_ids {
        if keep_plan_ids.iter().any(|kept| kept == &plan_id) {
            // 原地演进：只清旧代运行资源，plan 根保留（仅重新编译确认路径）。
            if purge_kept_runtime {
                purge_plan_runtime_resources(app_paths, project_id, issue_id, &plan_id, workspace_root)
                    .await?;
                report.runtime_purged_plan_ids.push(plan_id);
            }
            continue;
        }
        let outcome = delete_plan_generation(
            app_paths,
            project_id,
            issue_id,
            &plan_id,
            reason,
            workspace_root,
        )
        .await?;
        report.deleted_plan_ids.push(plan_id);
        report.evicted_session_ids.extend(outcome);
    }

    // ③ 不变量复核：存活 plan 数 MUST ≤ 1（保留项至多一个）。
    let remaining = operations
        .with_issue_relations(project_id, issue_id, || {
            list_live_plan_ids(app_paths, project_id, issue_id)
        })
        .map_err(CascadeDeleteError::Store)?;
    let surviving = remaining
        .iter()
        .filter(|plan_id| !keep_plan_ids.iter().any(|kept| kept == *plan_id))
        .count();
    if surviving > 0 {
        return Err(CascadeDeleteError::Store(ProductStoreError::Conflict {
            kind: "plan_supersede_invariant",
            id: surviving.to_string(),
        }));
    }
    Ok(report)
}

/// 完整删除一个旧 plan 代（子树先清、根最后；fail-closed + 幂等）。
///
/// 返回被删 plan 的 workspace session id（供调用方驱逐内存 manager）。
pub(crate) async fn delete_plan_generation(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    reason: PlanSupersedeReason,
    workspace_root: Option<&Path>,
) -> Result<Vec<String>, CascadeDeleteError> {
    let scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
    let deletion_store = EntityDeletionStore::new(app_paths.clone());
    let operations = EntityOperationStore::new(app_paths.clone());
    let store = LifecycleStore::new(app_paths.clone());

    // ① 墓碑：已删 → 幂等成功（零动作）；purging → 重入续清。
    let reentry = match deletion_store.get(&scope).map_err(CascadeDeleteError::Store)? {
        Some(record) if record.phase == DeletionPhase::Deleted => return Ok(Vec::new()),
        Some(_) => true,
        None => false,
    };

    // ② 删除独占（非阻塞）：争用 → Busy（fail-closed：不建立新 plan）。
    let lease = match operations.try_acquire_delete(&scope) {
        Ok(lease) => lease,
        Err(EntityOperationError::Deleted) => return Ok(Vec::new()),
        Err(EntityOperationError::Busy | EntityOperationError::Deleting) => {
            return Err(CascadeDeleteError::Busy);
        }
        Err(EntityOperationError::Store(error)) => return Err(CascadeDeleteError::Store(error)),
    };

    // ③ plan run 冻结集（purge 后不可再读）。
    let plan_run = CodingPlanRunStore::new(app_paths.clone())
        .get(project_id, issue_id, plan_id)
        .map_err(CascadeDeleteError::Store)?;

    // ④ 目标仓资源 cleanup lease（固定序）。
    let manifest_preview = store
        .collect_entity_deletion_manifest(&scope)
        .map_err(CascadeDeleteError::Store)?;
    let resource_leases = resource_cleanup_leases(
        app_paths,
        &store,
        &scope,
        &manifest_preview,
        plan_run.as_ref(),
    )?;

    // ⑤ 关系短锁内：manifest 采集 → pending → owned-index → purging 墓碑
    //    （含触发原因）。
    let manifest = operations
        .with_issue_relations(project_id, issue_id, || {
            collect_and_begin(
                app_paths,
                &deletion_store,
                &lease,
                &scope,
                reentry,
                reason,
            )
        })
        .map_err(CascadeDeleteError::Store)?;

    // ⑥ 未终结 attempt 收口（REQ-WPS-06：先收口再删除；失败零副作用——收口
    //    失败时未删任何从属产物，plan 根保留为可达的待清理入口）。
    if let Err(error) = terminate_plan_coding_attempts(
        app_paths,
        project_id,
        issue_id,
        plan_id,
        &manifest.work_item_ids,
    ) {
        let _ = record_pending_failure(
            app_paths,
            &lease,
            vec![format!("attempt_terminate:{plan_id}")],
            format!("{error}"),
            Some(reason.as_str()),
        );
        return Err(CascadeDeleteError::Store(error));
    }

    // ⑦ 子树清理（先子树后根）。任一失败 → 记录失败事实、保留 plan 根。
    if let Err(error) = purge_entity_closure(
        app_paths,
        &lease,
        &manifest,
        plan_run.as_ref(),
        &resource_leases,
        CodingGateMode::Cascade,
        workspace_root,
    )
    .await
    {
        let _ = record_pending_failure(
            app_paths,
            &lease,
            vec![format!("plan_closure:{plan_id}")],
            format!("{error}"),
            Some(reason.as_str()),
        );
        return Err(CascadeDeleteError::Store(error));
    }

    // ⑧ 根最后。
    if let Err(error) = store.delete_entity_root(&lease) {
        let _ = record_pending_failure(
            app_paths,
            &lease,
            vec![format!("plan_root:{plan_id}")],
            format!("{error}"),
            Some(reason.as_str()),
        );
        return Err(CascadeDeleteError::Store(error));
    }

    // ⑨ 残留复核（目标独占内容为空才允许完成）。
    let residue = store
        .list_entity_owned_residue(&scope)
        .map_err(CascadeDeleteError::Store)?;
    if !manifest_sets_empty(&residue) {
        let error = ProductStoreError::Io(format!(
            "entity_deletion_residue: plan {plan_id} closure still has residue"
        ));
        let _ = record_pending_failure(
            app_paths,
            &lease,
            vec![format!("plan_residue:{plan_id}")],
            format!("{error}"),
            Some(reason.as_str()),
        );
        return Err(CascadeDeleteError::Store(error));
    }
    if CodingPlanRunStore::new(app_paths.clone())
        .get(project_id, issue_id, plan_id)
        .map_err(CascadeDeleteError::Store)?
        .is_some()
    {
        let error = ProductStoreError::Io(format!(
            "entity_deletion_residue: plan run {plan_id} still present"
        ));
        let _ = record_pending_failure(
            app_paths,
            &lease,
            vec![format!("plan_run:{plan_id}")],
            format!("{error}"),
            Some(reason.as_str()),
        );
        return Err(CascadeDeleteError::Store(error));
    }

    // ⑩ 收口：remove pending → prune owned-index → remove failure fact → complete。
    deletion_store
        .remove_pending_manifest(&lease)
        .map_err(CascadeDeleteError::Store)?;
    deletion_store
        .prune_owned_artifact_ids_for_completion(&lease)
        .map_err(CascadeDeleteError::Store)?;
    deletion_store
        .remove_pending_failure(&lease)
        .map_err(CascadeDeleteError::Store)?;
    deletion_store
        .complete(&lease)
        .map_err(CascadeDeleteError::Store)?;
    Ok(manifest.session_ids)
}

/// 原地演进的运行资源清理（REQ-WPS-02 Scenario「原地重编译确认不得跳过旧代
/// 运行资源清理」）。
///
/// 清除旧代：coding attempts 及其运行目录、worktree 实体、reviewer
/// windows/snapshots/leases（含归档）、attempt locks、workspace lifecycle。
/// **plan 根保留**（本代沿用同一 plan_id 原地演进）。
pub(crate) async fn purge_plan_runtime_resources(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    workspace_root: Option<&Path>,
) -> Result<(), CascadeDeleteError> {
    let store = LifecycleStore::new(app_paths.clone());
    let scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
    let manifest = store
        .collect_entity_deletion_manifest(&scope)
        .map_err(CascadeDeleteError::Store)?;

    // 本 plan 拥有的 attempt 集合（真实 owner-key）。无 attempt ＝没有旧代
    // 运行事实（首次编译）→ 无旧代运行资源可清，零副作用放行。
    let coding_store = crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());
    let attempts = super::entity_deletion_cascade::plan_owned_attempts(
        &coding_store,
        project_id,
        issue_id,
        plan_id,
        &manifest.work_item_ids,
    )
    .map_err(CascadeDeleteError::Store)?;
    if attempts.is_empty() {
        return Ok(());
    }

    // 收口未终结 attempt（失败即中止，零副作用）。
    terminate_plan_coding_attempts(
        app_paths,
        project_id,
        issue_id,
        plan_id,
        &manifest.work_item_ids,
    )
    .map_err(CascadeDeleteError::Store)?;

    // coding attempts 及其运行资源（attempts/worktree/reviewer）。
    let attempt_ids: Vec<String> = attempts
        .iter()
        .map(|attempt| attempt.id.clone())
        .collect();
    super::entity_deletion_cascade::purge_plan_coding_attempts(
        app_paths,
        project_id,
        issue_id,
        plan_id,
        &manifest,
    )
    .await
    .map_err(CascadeDeleteError::Store)?;

    // 旧代 attempt 持有的 shared-worktree 锁：释放（owner 置空 ＋ Ready），
    // 否则新代永远拿不到锁。
    super::entity_deletion_cascade::release_plan_shared_worktree_locks(
        app_paths,
        project_id,
        issue_id,
        &attempt_ids,
    )
    .map_err(CascadeDeleteError::Store)?;

    // attempt 残留锁。
    purge_attempt_lock_residue(
        app_paths,
        project_id,
        issue_id,
        plan_id,
        &manifest.work_item_ids,
    )
    .map_err(CascadeDeleteError::Store)?;

    // workspace lifecycle 元数据（执行轮 generation 复位）。
    CodingPlanRunStore::new(app_paths.clone())
        .purge_workspace_lifecycle(project_id, issue_id, plan_id)
        .map_err(CascadeDeleteError::Store)?;

    let _ = workspace_root;
    Ok(())
}

/// 关系短锁内：manifest 采集 → pending → owned-index → purging 墓碑。
fn collect_and_begin(
    app_paths: &ProductAppPaths,
    deletion_store: &EntityDeletionStore,
    lease: &EntityDeletionLease,
    scope: &EntityScope,
    reentry: bool,
    reason: PlanSupersedeReason,
) -> Result<EntityDeletionManifest, ProductStoreError> {
    let store = LifecycleStore::new(app_paths.clone());
    let fresh = store.collect_entity_deletion_manifest(scope)?;
    let manifest = match deletion_store.load_pending_manifest(lease)? {
        Some(pending) => pending.merged_with(&fresh)?,
        None => fresh,
    };
    deletion_store.save_pending_manifest(lease, &manifest)?;
    deletion_store.index_owned_artifact_ids(lease, &manifest)?;
    deletion_store.begin_with_reason(lease, Some(reason.as_str()))?;
    let _ = (app_paths, reentry);
    Ok(manifest)
}

/// 固定 resource keys 排序 try cleanup lease（scope → resource → 关系短锁）。
///
/// key 集合与闭包清理用的目标仓集合**同源**
/// （`plan_shared_worktree_repository_ids`：manifest 目标仓 ∪ run 冻结 target
/// ∪ 本 issue 全部 repo 级 shared-worktree 记录 ∪ legacy issue 布局）。
fn resource_cleanup_leases(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    scope: &EntityScope,
    manifest: &EntityDeletionManifest,
    plan_run: Option<&crate::product::coding_plan_run::CodingPlanRun>,
) -> Result<Vec<(SharedResourceKey, SharedResourceCleanupLease)>, CascadeDeleteError> {
    let (project_id, issue_id) = (scope.project_id.as_str(), scope.issue_id.as_str());
    let repositories =
        super::entity_deletion_cascade::plan_shared_worktree_repository_ids(
            store, scope, manifest, plan_run,
        )
        .map_err(|error| match error {
            ProductStoreError::InvalidRecord {
                kind: "entity_target_repository",
                ..
            } => CascadeDeleteError::InvalidTargetRepository {
                target_repository_id: manifest.target_repository_ids.join(","),
            },
            other => CascadeDeleteError::Store(other),
        })?;
    let operations = EntityOperationStore::new(app_paths.clone());
    let mut leases = Vec::new();
    for repository_id in repositories {
        let key = SharedResourceKey {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: repository_id.map(LogicalRepositoryId),
        };
        let lease = operations
            .try_acquire_shared_resource_cleanup(&key)
            .map_err(|error| match error {
                EntityOperationError::Store(inner) => CascadeDeleteError::Store(inner),
                _ => CascadeDeleteError::Busy,
            })?;
        leases.push((key, lease));
    }
    Ok(leases)
}

/// manifest 全部纯 ID 集合为空（残留复核用；scope 字段不算集合）。
fn manifest_sets_empty(manifest: &EntityDeletionManifest) -> bool {
    manifest.session_ids.is_empty()
        && manifest.work_item_ids.is_empty()
        && manifest.review_ids.is_empty()
        && manifest.provider_run_ids.is_empty()
        && manifest.verification_plan_ids.is_empty()
        && manifest.repository_profile_ids.is_empty()
        && manifest.target_repository_ids.is_empty()
        && manifest.advance_ids.is_empty()
        && manifest.advance_retry_command_ids.is_empty()
}

#[cfg(test)]
mod tests;
