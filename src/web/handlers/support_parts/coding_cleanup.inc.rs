pub(crate) fn abort_attempt_if_active(
    coding_store: &CodingAttemptStore,
    attempt: CodingExecutionAttempt,
) -> ApiResult<CodingExecutionAttempt> {
    if !attempt.status.is_active() {
        return Ok(attempt);
    }
    coding_store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Aborted,
        )
        .map_err(product_store_api_error)
}

pub(crate) async fn cleanup_coding_attempt_workspace(
    repository: &RepositoryRecord,
    attempt: &CodingExecutionAttempt,
) -> ApiResult<()> {
    let git = GitWorkspaceService::new();
    if let Some(worktree_path) = attempt.worktree_path.as_ref() {
        git.remove_worktree(&repository.path, worktree_path)
            .await
            .map_err(git_workspace_api_error)?;
    }
    git.prune_worktrees(&repository.path)
        .await
        .map_err(git_workspace_api_error)?;
    git.delete_local_branch(&repository.path, &attempt.branch_name)
        .await
        .map_err(git_workspace_api_error)?;
    Ok(())
}

/// 收集 attempt 持有 work-item-attempt-locks 的 work_item 集合。
///
/// 必须在 `delete_attempt` 之前调用（依赖 attempt 的 unit 目录可读）。
/// - single scope：直接取 `attempt.work_item_id`。
/// - group scope：取各 coding unit 的 `logical_work_item_id`（与 group 删除路径一致）。
///   unit 列表为空（未进入 plan 阶段或目录缺失）时回退到 `attempt.work_item_id`
///   以兜底清理 anchor 的锁，避免遗漏。
pub(crate) fn collect_attempt_work_item_ids(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Vec<String> {
    if matches!(attempt.scope, CodingAttemptScope::WorkItemGroup) {
        let unit_ids: Vec<String> = coding_store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .map(|units| {
                units
                    .into_iter()
                    .map(|unit| unit.logical_work_item_id)
                    .collect()
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

/// 清理 attempt 删除后的残留 lock（spec「清理残留 lock」）。
///
/// 每步 NotFound=OK（`remove_file_if_exists`），单项失败不中断其余清理：
/// - `.coding_attempt_<id>.json.lock`：attempt 自身运行时 lock（孤儿）。
/// - `.group-initialization-arbitration.lock`：仅 group scope（single 不持有）。
/// - `work-item-attempt-locks/<wi>` + `.<wi>.lock`：按 `work_item_ids` 精确删，
///   **不整目录**（其他 attempt 的 work_item lock 可能共存——spec「不误删」）。
pub(crate) fn purge_coding_attempt_lock_residue(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    work_item_ids: &[String],
) {
    use crate::product::lifecycle_store::remove_file_if_exists;

    let coding_attempts_root = app_paths
        .issue_root(&attempt.project_id, &attempt.issue_id)
        .join("coding-attempts");

    let _ = remove_file_if_exists(&coding_attempts_root.join(format!(".{}.json.lock", attempt.id)));

    if matches!(attempt.scope, CodingAttemptScope::WorkItemGroup) {
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

/// 删除 attempt 后条件清理 issue shared-worktree（spec「条件清理 shared-worktree」）。
///
/// 该 issue 无其他 attempt 记录（`list_attempts_for_issue` 为空，当前 attempt 已删）
/// → 删 `issue-shared-worktree.json` + `.lock`（复用 `delete_issue_shared_worktree`，
/// NotFound=OK）。有其他 attempt（仍持有 shared-worktree）→ 保留，避免误伤。
pub(crate) fn cleanup_issue_shared_worktree_if_no_attempts(
    coding_store: &CodingAttemptStore,
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> ApiResult<()> {
    let remaining = coding_store
        .list_attempts_for_issue(project_id, issue_id)
        .map_err(product_store_api_error)?;
    if !remaining.is_empty() {
        return Ok(());
    }
    LifecycleStore::new(app_paths.clone())
        .delete_issue_shared_worktree(project_id, issue_id)
        .map_err(product_store_api_error)?;
    Ok(())
}

/// 多仓 attempt 删除后的仓维 shared worktree 条件清理（REQ-COD-03 §4.2.4）。
///
/// 仅当同 logical repository 无其他活动 attempt 引用时才删 `shared-worktrees/{id}.json`
/// + `.lock`（同仓其他 item 可能复用，不能一个 attempt 删就清）。
fn cleanup_repo_shared_worktree_if_no_active_attempts(
    coding_store: &CodingAttemptStore,
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    repository_id: LogicalRepositoryId,
) -> ApiResult<()> {
    let other_active_in_repo = coding_store
        .list_attempts_for_issue(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .any(|attempt| {
            attempt
                .target_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.logical_repository_id == repository_id)
                && attempt.status.is_active()
        });
    if other_active_in_repo {
        return Ok(());
    }
    LifecycleStore::new(app_paths.clone())
        .delete_repo_shared_worktree(project_id, issue_id, repository_id)
        .map_err(product_store_api_error)?;
    Ok(())
}

/// issue/group plan 删除路径的 shared worktree 清理（REQ-COD-03 §4.2.4 issue 删除）。
///
/// 按 routing 分流：多仓（Logical）枚举 `shared-worktrees/` 下所有
/// `{repository_id}.json` 逐个清（含锁文件，经 `list_repo_shared_worktrees` +
/// `delete_repo_shared_worktree`）；单仓（Legacy）走老逻辑清 `issue-shared-worktree.json`
/// （不变，红线）。FailClosed 稳定码上抛，绝不静默回退物理仓库。
pub(crate) fn cleanup_shared_worktree_by_routing(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> ApiResult<()> {
    let lifecycle = LifecycleStore::new(app_paths.clone());
    match RepositoryRouting::load_for_issue(app_paths, project_id, issue_id)
        .map_err(product_store_api_error)?
    {
        RepositoryRouting::Legacy { .. } => lifecycle
            .delete_issue_shared_worktree(project_id, issue_id)
            .map_err(product_store_api_error)?,
        RepositoryRouting::Logical { .. } => {
            for repository_id in lifecycle
                .list_repo_shared_worktrees(project_id, issue_id)
                .map_err(product_store_api_error)?
            {
                lifecycle
                    .delete_repo_shared_worktree(project_id, issue_id, repository_id)
                    .map_err(product_store_api_error)?;
            }
        }
        RepositoryRouting::FailClosed { code, reason } => {
            return Err(routing_api_error(code, &reason));
        }
    }
    Ok(())
}

/// 完成 attempt 删除的尾部清理（worktree/handoff 清理之后）。
///
/// 顺序（spec `harden-coding-attempt-deletion`）：
/// 1. 先取该 attempt 的 work_item 集合（依赖 attempt 的 unit 目录可读，必须在
///    `delete_attempt` 之前）。
/// 2. `delete_attempt` 删 attempt 数据（json + 目录）。
/// 3. `purge_coding_attempt_lock_residue` 清残留 lock（NotFound=OK，按 work_item 精确）。
/// 4. `cleanup_issue_shared_worktree_if_no_attempts` 条件清 shared-worktree。
pub(crate) fn finalize_coding_attempt_deletion(
    coding_store: &CodingAttemptStore,
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> ApiResult<()> {
    let attempt_work_item_ids = collect_attempt_work_item_ids(coding_store, attempt);
    coding_store
        .delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    purge_coding_attempt_lock_residue(app_paths, attempt, &attempt_work_item_ids);
    if let Some(snapshot) = &attempt.target_snapshot { cleanup_repo_shared_worktree_if_no_active_attempts(
        coding_store,
        app_paths,
        &attempt.project_id,
        &attempt.issue_id,
        snapshot.logical_repository_id,
    )? }
    // 缺陷 #13 层2 存量出口：组入口路由分流修复前，Logical 路由的 group
    // create 会误写 issue 维 legacy 布局（与运行期 preflight 契约相反，恢复/
    // 完成门 fail-closed legacy_shared_worktree_present）。删除该 issue 的
    // attempt 后条件清理 issue 维布局文件，恢复迁移契约一致性；正常仓维
    // 布局下该文件不存在，清理幂等（NotFound=OK）。
    cleanup_issue_shared_worktree_if_no_attempts(
        coding_store,
        app_paths,
        &attempt.project_id,
        &attempt.issue_id,
    )?;
    Ok(())
}

