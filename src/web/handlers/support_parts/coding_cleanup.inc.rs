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

/// N7（REQ-CS-07）：plan-run 冻结 target 集合的精确仓维资源清理包装。
///
/// 供 plan 删除闭包（存在新 plan-run 时）在 purge run 前按冻结 target 集合
/// 逐仓清理 repo shared worktree——**只清本 run 冻结的仓**，不走旧
/// issue-wide helper（不扫其它仓，`cleanup_shared_worktree_by_routing` 的
/// 其它 caller 语义不变）。每仓先取 `try_acquire_shared_resource_cleanup`
/// 短 lease（与资源写者互斥，争用 fail-closed 上抛），锁内复核记录真实
/// owner：记录身份必须仍属本 (project, issue, target)；活动锁 owner 为他人
/// （不属本 run 冻结 attempt 集/selection）时保留不删。NotFound=OK（缺失
/// 视为已清理），IO/JSON 错误上抛不吞。D2.2 后行统一消费本包装，不重复实现。
pub(crate) fn cleanup_plan_run_shared_worktrees(
    app_paths: &ProductAppPaths,
    run: &crate::product::coding_plan_run::CodingPlanRun,
) -> ApiResult<()> {
    let operations =
        crate::product::entity_operation_store::EntityOperationStore::new(app_paths.clone());
    let lifecycle = LifecycleStore::new(app_paths.clone());
    for binding in &run.target_attempts {
        let Ok(repository_uuid) = uuid::Uuid::parse_str(&binding.target_repository_id) else {
            return Err(ApiError::runtime(
                "plan_coding_run_target_invalid",
                "plan run frozen target is not a logical repository id",
                json!({
                    "plan_id": run.plan_id,
                    "target_repository_id": binding.target_repository_id,
                }),
            ));
        };
        let repository_id = LogicalRepositoryId(repository_uuid);
        let key = crate::product::entity_operation_store::SharedResourceKey {
            project_id: run.project_id.clone(),
            issue_id: run.issue_id.clone(),
            repository_id: Some(repository_id),
        };
        // 清理短 lease：与写者互斥；争用（Busy）fail-closed 上抛，客户端可
        // 重试，绝不静默跳过留下孤儿授权资源。
        let _cleanup_lease = operations
            .try_acquire_shared_resource_cleanup(&key)
            .map_err(shared_resource_cleanup_api_error)?;
        let Some(record) = lifecycle
            .get_repo_shared_worktree(&run.project_id, &run.issue_id, repository_id)
            .map_err(product_store_api_error)?
        else {
            continue;
        };
        // 复核真实 owner（记录身份）：路径派生读回的记录若身份漂移则
        // fail-closed，绝不删除无法归属的记录。
        if record.project_id != run.project_id
            || record.issue_id != run.issue_id
            || record.target_repository_id != Some(repository_id)
        {
            return Err(product_store_api_error(
                ProductStoreError::IdentityMismatch {
                    kind: "repo_shared_worktree",
                    id: binding.target_repository_id.clone(),
                },
            ));
        }
        // 复核活动锁 owner：他人（另一 plan 的 attempt）持有的活动锁保留；
        // 空主或本 run 冻结 attempt 集/selection 内的残留 owner 才可清。
        let owner_within_run = record.current_lock_owner_id.is_none_or(|owner| {
            run.target_attempts
                .iter()
                .any(|frozen| frozen.attempt_id == owner)
                || run
                    .selection
                    .as_ref()
                    .is_some_and(|selection| selection.attempt_id == owner)
        });
        if !owner_within_run {
            continue;
        }
        lifecycle
            .delete_repo_shared_worktree(&run.project_id, &run.issue_id, repository_id)
            .map_err(product_store_api_error)?;
    }
    Ok(())
}

/// `try_acquire_shared_resource_cleanup` 的稳定错误映射（N7 包装专用）。
fn shared_resource_cleanup_api_error(
    error: crate::product::entity_operation_store::EntityOperationError,
) -> ApiError {
    use crate::product::entity_operation_store::EntityOperationError;
    match error {
        EntityOperationError::Busy => ApiError::runtime(
            "plan_coding_run_resource_busy",
            "plan run target resource is held by a concurrent writer",
            json!({}),
        ),
        EntityOperationError::Deleting | EntityOperationError::Deleted => ApiError::runtime(
            "plan_coding_run_resource_unavailable",
            "plan run target resource is being deleted or already deleted",
            json!({}),
        ),
        EntityOperationError::Store(error) => product_store_api_error(error),
    }
}

/// 完成 attempt 删除的尾部清理（worktree/handoff 清理之后）。
///
/// 顺序（spec `harden-coding-attempt-deletion` + N7 REQ-CS-07）：
/// 1. 先取该 attempt 的 work_item 集合（依赖 attempt 的 unit 目录可读，必须在
///    `delete_attempt` 之前）。
/// 2. `delete_attempt` 删 attempt 数据（json + 目录）。
/// 3. `invalidate_for_deleted_attempt` 精确失效引用该 attempt 的 plan run
///    （N7：member 删除后旧 command 不得再首启/补建不完整集合；无匹配 run
///    直接 OK，不自动补建）。
/// 4. `purge_coding_attempt_lock_residue` 清残留 lock（NotFound=OK，按 work_item 精确）。
/// 5. `cleanup_issue_shared_worktree_if_no_attempts` 条件清 shared-worktree。
pub(crate) fn finalize_coding_attempt_deletion(
    coding_store: &CodingAttemptStore,
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> ApiResult<()> {
    let attempt_work_item_ids = collect_attempt_work_item_ids(coding_store, attempt);
    coding_store
        .delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    // N7（REQ-CS-07）：冻结集合含该 attempt（或 selection 指向它）时精确
    // 失效本 plan run；身份错配/IO 错误上抛（不吞）。
    crate::product::coding_plan_run::CodingPlanRunStore::new(app_paths.clone())
        .invalidate_for_deleted_attempt(attempt)
        .map_err(product_store_api_error)?;
    purge_coding_attempt_lock_residue(app_paths, attempt, &attempt_work_item_ids);
    if let Some(snapshot) = &attempt.target_snapshot {
        cleanup_repo_shared_worktree_if_no_active_attempts(
            coding_store,
            app_paths,
            &attempt.project_id,
            &attempt.issue_id,
            snapshot.logical_repository_id,
        )?
    }
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
