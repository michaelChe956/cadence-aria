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
