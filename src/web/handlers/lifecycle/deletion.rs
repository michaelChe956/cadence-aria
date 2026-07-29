use super::*;

pub async fn delete_work_item(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, work_item_id)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let app_paths = product_app_paths(&state);
    let store = LifecycleStore::new(app_paths.clone());
    delete_work_item_with_cleanup(&app_paths, &store, &project_id, &issue_id, &work_item_id)
        .await?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn delete_work_item_plan(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, plan_id)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let app_paths = product_app_paths(&state);
    let store = LifecycleStore::new(app_paths.clone());
    let plan = store
        .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    if let Some(lineage) = schema_v2_plan_lineage(&app_paths, &project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?
    {
        delete_schema_v2_work_item_plan_with_cleanup(
            &app_paths,
            &store,
            &project_id,
            &issue_id,
            &plan_id,
            &lineage,
        )
        .await?;
        return Ok(Json(json!({"status":"deleted"})));
    }
    for work_item_id in &plan.work_item_ids {
        delete_work_item_with_cleanup(&app_paths, &store, &project_id, &issue_id, work_item_id)
            .await?;
    }
    store
        .delete_issue_work_item_plan(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    purge_work_item_plan_store_artifacts(&app_paths, &project_id, &issue_id, &plan_id)?;
    Ok(Json(json!({"status":"deleted"})))
}

/// 清理 plan store 中该 plan 的 draft、compile transaction 与 outline context index。
///
/// 两条删除路径（schema v2 与 legacy）都必须调用：这些产物不属于 LifecycleStore，
/// 不随 plan 记录一起消失。
fn purge_work_item_plan_store_artifacts(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> ApiResult<()> {
    crate::product::work_item_plan_store::WorkItemPlanStore::new(app_paths.clone())
        .purge_plan_artifacts(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    Ok(())
}

pub(crate) async fn delete_work_item_with_cleanup(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
) -> ApiResult<()> {
    if schema_v2_plan_for_work_item(app_paths, store, project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?
        .is_some()
    {
        return Err(ApiError::validation(
            "schema_v2_group_delete_required",
            "schema v2 work items must be deleted through their work item plan",
        ));
    }
    let work_item = store
        .list_work_items(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|work_item| work_item.id == work_item_id)
        .ok_or_else(|| {
            product_store_api_error(ProductStoreError::NotFound {
                kind: "work_item",
                id: work_item_id.to_string(),
            })
        })?;
    let repository = find_repository(app_paths, project_id, &work_item.repository_id)?;
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let attempts = coding_store
        .list_attempts_for_work_item(project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?;
    for attempt in attempts {
        let attempt = abort_attempt_if_active(&coding_store, attempt)?;
        cleanup_coding_attempt_workspace(&repository, &attempt).await?;
    }
    coding_store
        .delete_attempts_for_work_item(project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?;
    store
        .delete_work_item(project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?;
    Ok(())
}

fn schema_v2_plan_lineage(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<Option<crate::product::models::WorkItemPlanLineage>, ProductStoreError> {
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    match revision_store.get_plan_lineage(project_id, issue_id, plan_id) {
        Ok(lineage) => Ok(Some(lineage)),
        Err(ProductStoreError::NotFound {
            kind: "work_item_plan_lineage",
            ..
        }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn schema_v2_plan_for_work_item(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
) -> Result<Option<crate::product::models::WorkItemPlanLineage>, ProductStoreError> {
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    for plan in store.list_issue_work_item_plans(project_id, issue_id)? {
        let Some(lineage) = schema_v2_plan_lineage(app_paths, project_id, issue_id, &plan.id)?
        else {
            continue;
        };
        let active_revision_id = lineage.active_revision_id.as_deref().ok_or_else(|| {
            ProductStoreError::IdentityMismatch {
                kind: "runtime_binding_missing",
                id: plan.id.clone(),
            }
        })?;
        let revision =
            revision_store.get_plan_revision(project_id, issue_id, &plan.id, active_revision_id)?;
        if revision.work_item_bindings.contains_key(work_item_id) {
            return Ok(Some(lineage));
        }
    }
    Ok(None)
}

async fn delete_schema_v2_work_item_plan_with_cleanup(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    _lineage: &crate::product::models::WorkItemPlanLineage,
) -> ApiResult<()> {
    // 门禁：存在 group coding attempt 时拒绝，要求用户先删 coding workspace。
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    if let Some(attempt) = coding_store
        .get_attempt_for_work_item_group(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?
    {
        return Err(coding_workspace_exists_error(plan_id, &attempt.id));
    }

    // 尽力清理 WorkItem 类型 session：扫描 session 自身的 plan_id 绑定，
    // 不依赖 plan revision 的 work_item_bindings 数量，半残状态也能定位。
    // 单项失败不阻断其余 session 的清理。
    let work_item_sessions: Vec<_> = store
        .list_workspace_sessions(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .filter(|session| {
            session.workspace_type == WorkspaceType::WorkItem
                && session
                    .work_item_runtime_binding
                    .as_ref()
                    .is_some_and(|binding| binding.plan_id == plan_id)
        })
        .collect();
    for session in work_item_sessions {
        let _ = store.delete_workspace_sessions_for_entity(
            project_id,
            issue_id,
            &session.entity_id,
            WorkspaceType::WorkItem,
        );
    }

    // 依次删产物：每步 NotFound=OK，绝不因缺失中断（spec「删除无残留」）。
    // 1. plan 元数据（plan json + WorkItemPlan 类型 session）
    store
        .delete_schema_v2_issue_work_item_plan_metadata(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    // 2. revisions + publications 整目录（Task 1）
    WorkItemRevisionStore::new(app_paths.clone())
        .purge_plan_revisions(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    // 3. plan store drafts/compiles/outlines
    purge_work_item_plan_store_artifacts(app_paths, project_id, issue_id, plan_id)?;
    // 4. issue shared worktree json + lock（Task 1）
    store
        .delete_issue_shared_worktree(project_id, issue_id)
        .map_err(product_store_api_error)?;
    // 5. coding attempt 初始化残留 lock（attempt json 已被门禁确认不存在，
    //    残留的 arbitration / journal / attempt lock 一并清理）
    purge_attempt_lock_residue(app_paths, project_id, issue_id, plan_id)?;

    Ok(())
}

/// 清理本 issue `coding-attempts/` 目录下与该 plan 相关的残留锁与初始化产物。
///
/// 删除路径在门禁放行后调用：此时 group attempt 的主体 json 已不存在，
/// 但 arbitration 文件、group initialization journal、work item attempt 锁、
/// attempt json 的 `.lock` 文件等可能残留（线上「半残」的典型形态）。
/// 全程容错：文件不存在视为成功，单项失败不影响其余清理。
fn purge_attempt_lock_residue(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> ApiResult<()> {
    use crate::product::lifecycle_store::{remove_dir_all_if_exists, remove_file_if_exists};

    let coding_attempts_root = app_paths
        .issue_root(project_id, issue_id)
        .join("coding-attempts");

    // group-initialization-arbitration 文件 + lock（issue 级仲裁，group 删除时安全清理）
    let _ = remove_file_if_exists(&coding_attempts_root.join("group-initialization-arbitration"));
    let _ =
        remove_file_if_exists(&coding_attempts_root.join(".group-initialization-arbitration.lock"));

    // group-initializations/<plan_id>.json + 其 lock（本 plan 的初始化 journal）
    let journal_dir = coding_attempts_root.join("group-initializations");
    let _ = remove_file_if_exists(&journal_dir.join(format!("{plan_id}.json")));
    let _ = remove_file_if_exists(&journal_dir.join(format!(".{plan_id}.json.lock")));

    // work-item-attempt-locks/ 子目录：single coding attempt 创建时的 per-work-item 锁。
    // group 删除重置 issue coding 状态，整目录清理。
    let _ = remove_dir_all_if_exists(&coding_attempts_root.join("work-item-attempt-locks"));

    // 顶层 `.lock` 残留：attempt json 已不存在，对应的 `.<attempt_id>.json.lock` 是孤儿。
    if let Ok(entries) = std::fs::read_dir(&coding_attempts_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|value| value.to_str())
                && name.starts_with('.')
                && name.ends_with(".lock")
            {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    Ok(())
}
