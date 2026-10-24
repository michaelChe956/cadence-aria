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
#[cfg(test)] // 生产删除路径走 issue/group 专用清理;宽 helper 仅 T11 单测覆盖。
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

/// Bug1(owner 2026-10-16 裁定):plan 级删除后的仓侧 issue worktree 残留清扫。
///
/// 成员 worktree 拆除只处理 `<repo>/.worktrees/aria-issues/<issue>/<uuid>`
/// 精确路径;父目录 `<repo>/.worktrees/aria-issues/<issue>/`(含 provider
/// 边界的 `.provider-session-cache` 宿主领土缓存)从不被清理,形成
/// 「注册已删、目录残留」。本清扫在删除动作收尾统一执行:
/// - 候选:按 routing 枚举 issue 所属各物理仓的 issue worktree 根目录;
/// - 保护:该 issue 仍有 attempt 记录(任何 plan/轮次,含 parked 冻结路径)
///   指向此目录,或目录下仍有 git 注册 worktree(存活执行面)→ 保留;
/// - 其余:整目录删除(缺失=已清,幂等)。IO 失败显式上抛,不静默。
///   routing FailClosed 时不阻断已完成的删除结果,仅告警跳过(删除准入
///   已在前序步骤对 routing 做过 fail-closed)。
pub(crate) async fn sweep_issue_worktree_residue(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> ApiResult<()> {
    validate_relative_id(issue_id).map_err(|error| {
        ApiError::validation(
            "plan_coding_workspace_sweep_invalid_id",
            format!("invalid issue worktree residue sweep id: {error}"),
        )
    })?;

    // 候选仓根:routing 枚举(Logical=manifest 成员;Legacy=issue 物理仓)。
    let routing = RepositoryRouting::load_for_issue(app_paths, project_id, issue_id)
        .map_err(product_store_api_error)?;
    let repositories = crate::product::repository_store::RepositoryStore::new(app_paths.clone())
        .list(project_id)
        .map_err(product_store_api_error)?;
    let mut repo_roots: Vec<std::path::PathBuf> = match &routing {
        RepositoryRouting::Legacy { repository_id } => repositories
            .iter()
            .filter(|record| {
                !repository_id.is_empty() && record.id.as_str() == repository_id.as_str()
            })
            .map(|record| record.path.clone())
            .collect(),
        // Bug1 fix-loop r2(v1.3):物理仓根优先取 LC 子树 checkouts/*.json 的
        // canonical_path(原生项目无 repos.json);checkouts 解析不到成员时
        // 退回 repos.json 原路径(混布夹具)。Legacy 分支保持逐字不变。
        RepositoryRouting::Logical { manifest, .. } => {
            // lc_id 与 `load_for_issue` 同源(issue 记录→None 走 legacy 根),
            // 不依赖 manifest 自带 id(legacy 根 manifest 的 id 不等于子树名)。
            let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
                app_paths, project_id, issue_id,
            )
            .map_err(product_store_api_error)?;
            let lc_store = match lc_id.as_deref() {
                Some(lc_id) => LogicalCodebaseStore::for_lc(app_paths.clone(), lc_id),
                None => LogicalCodebaseStore::new(app_paths.clone()),
            };
            let mut roots: Vec<std::path::PathBuf> = lc_store
                .list_checkouts(project_id)
                .map_err(product_store_api_error)?
                .into_iter()
                .filter(|checkout| {
                    manifest
                        .member_ids
                        .contains(&checkout.logical_repository_id)
                })
                .map(|checkout| checkout.canonical_path.clone())
                .collect();
            if roots.is_empty() {
                roots = repositories
                    .iter()
                    .filter(|record| {
                        record
                            .logical_repository_id
                            .as_ref()
                            .is_some_and(|logical| manifest.member_ids.contains(logical))
                    })
                    .map(|record| record.path.clone())
                    .collect();
            }
            roots
        }
        RepositoryRouting::FailClosed { code, reason } => {
            tracing::warn!(
                code = ?code,
                reason = %reason,
                "skip issue worktree residue sweep: repository routing failed"
            );
            return Ok(());
        }
    };
    repo_roots.sort();
    repo_roots.dedup();
    if repo_roots.is_empty() {
        tracing::warn!(
            project_id,
            issue_id,
            "issue worktree residue sweep resolved no repository roots; nothing to sweep"
        );
    }

    // 保护面:issue 仍有 attempt 记录(其 worktree_path 父目录即受保护)。
    let protected: Vec<std::path::PathBuf> = CodingAttemptStore::new(app_paths.clone())
        .list_attempts_for_issue(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .filter_map(|attempt| attempt.worktree_path.clone())
        .filter_map(|worktree_path| {
            let parent = worktree_path.parent()?.to_path_buf();
            let canonical = parent.canonicalize().unwrap_or_else(|_| parent.clone());
            Some(vec![parent, canonical])
        })
        .flatten()
        .collect();

    let git = GitWorkspaceService::new();
    for repo_root in repo_roots {
        let candidate = repo_root
            .join(".worktrees")
            .join("aria-issues")
            .join(issue_id);
        if !candidate.exists() {
            continue;
        }
        let candidate_canonical = candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.clone());
        if protected
            .iter()
            .any(|path| path == &candidate || path == &candidate_canonical)
        {
            continue;
        }
        // 目录下仍有注册 worktree(其他 plan/轮次存活):保护,不整删。
        let registered_in_candidate = git
            .list_worktree_paths(&repo_root)
            .await
            .map_err(git_workspace_api_error)?
            .into_iter()
            .any(|worktree| {
                worktree.starts_with(&candidate) || worktree.starts_with(&candidate_canonical)
            });
        if registered_in_candidate {
            continue;
        }
        std::fs::remove_dir_all(&candidate).map_err(|error| {
            ApiError::runtime(
                "issue_worktree_residue_sweep_failed",
                format!(
                    "failed to remove issue worktree residue {}: {error}",
                    candidate.display()
                ),
                json!({ "path": candidate.display().to_string() }),
            )
        })?;
        tracing::info!(
            path = %candidate.display(),
            "swept issue worktree residue directory"
        );
    }

    Ok(())
}

/// `try_acquire_shared_resource_cleanup` 的稳定错误映射（资源清理 lease 争用：
/// D2.2 `with_held_or_acquired_cleanup_lease` 现取通道消费）。
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

/// H2a 稳定码直通（设计 2026-10-14 B 节）：闭包自身的执行轮状态冲突码
/// （`coding_plan_workspace_clearing` / `coding_plan_run_retired` /
/// `coding_plan_workspace_generation_conflict` /
/// `coding_plan_workspace_members_present`）原样透出给调用方/前端；
/// HTTP 409 映射归 H1 白名单（error.rs）。其余 store 错误归既有通用映射。
fn workspace_reset_api_error(error: crate::product::json_store::ProductStoreError) -> ApiError {
    if let crate::product::json_store::ProductStoreError::Conflict { kind, .. } = error
        && matches!(
            kind,
            "coding_plan_workspace_clearing"
                | "coding_plan_run_retired"
                | "coding_plan_workspace_generation_conflict"
                | "coding_plan_workspace_members_present"
        )
    {
        return ApiError::runtime(
            kind,
            "coding workspace round reset rejected the request",
            json!({}),
        );
    }
    product_store_api_error(error)
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
    cleanup_issue_shared_worktree_if_no_attempts(
        coding_store,
        app_paths,
        &attempt.project_id,
        &attempt.issue_id,
    )?;
    // H2a（设计 2026-10-14 D 节）：最后一个成员删除完成时接同一零成员闭包
    // ——成功即清活动执行引用（run/advance 引用/初始化引用/专属资源，保留
    // Plan/WI/revisions）；清理 IO/资源 busy 失败保持 clearing 耐久并如实
    // 上抛，下一次产品动作同 command 续清（missing=OK 不重复副作用）。
    if let Some(plan_id) = attempt.work_item_group_id.as_deref()
        && coding_store
            .list_attempts_for_plan(&attempt.project_id, &attempt.issue_id, plan_id)
            .map_err(product_store_api_error)?
            .is_empty()
    {
        let generation =
            crate::product::coding_plan_run::CodingPlanRunStore::new(app_paths.clone())
                .current_workspace_generation(&attempt.project_id, &attempt.issue_id, plan_id)
                .map_err(product_store_api_error)?;
        let command_id = format!("plan_workspace_close_{plan_id}_g{generation}");
        close_plan_coding_workspace_round(
            app_paths,
            &attempt.project_id,
            &attempt.issue_id,
            plan_id,
            &command_id,
            Some(generation),
            false,
        )?;
    }
    Ok(())
}

// ─── H2a（设计 2026-10-14 A/C/D 节）：零成员执行轮完整关闭闭包 ───

/// 零成员闭包的分类/执行结果。
#[derive(Debug)]
pub(crate) enum PlanCodingWorkspaceResetOutcome {
    /// 完整重置完成（或同 command 幂等重放）；`generation` 为重置后的新轮次。
    Reset {
        /// 重置后轮次:供响应扩展预留,当前解码方只判变体。
        #[allow(dead_code)]
        generation: u64,
    },
    /// 仅存在未越 AttemptPersisted 的初始化前缀——继续原协议，不重置
    /// （设计 A 节：不能仅因 Prepared.target_attempts 为空便重置）。
    InitializationPrefix,
    /// 无任何执行引用（run/advance/journal 均无）——零变更。
    Clean,
}

/// plan 执行引用分类（设计 A 节「journal 在而 attempts 亡」）：断裂证明 =
/// run 冻结集非空（零存活即全亡）∪ H2b `classify_stale_references` 的
/// advance 侧 `AllTargetsDead`（Ready record 绑定 ∪ 已越 AttemptPersisted
/// 的外层 journal 绑定，全缺）∪ 已越 AttemptPersisted 的 per-target
/// journal；仅 Prepared 前缀不计入断裂集（继续原协议）。存活集由闭包预检
/// 枚举（零成员是闭包前置），此处恒以空集判定 advance 侧。
struct PlanExecutionReferenceState {
    run: Option<crate::product::coding_plan_run::CodingPlanRun>,
    references: crate::product::advance_store::AdvancePlanExecutionReferences,
    journals: Vec<crate::product::coding_attempt_store::CodingGroupInitializationJournal>,
    broken: bool,
    any_reference: bool,
}

fn classify_plan_execution_references(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> ApiResult<PlanExecutionReferenceState> {
    use crate::product::advance_store::AdvanceStaleReferenceVerdict;

    let run = crate::product::coding_plan_run::CodingPlanRunStore::new(app_paths.clone())
        .get(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    let references = crate::product::advance_store::AdvanceStore::new(app_paths.clone())
        .plan_execution_references(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    let journals = CodingAttemptStore::new(app_paths.clone())
        .list_group_initialization_journals_for_plan(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    let run_broken = run
        .as_ref()
        .is_some_and(|run| !run.target_attempts.is_empty());
    let advance_broken = matches!(
        references.classify_stale_references(&[]),
        AdvanceStaleReferenceVerdict::AllTargetsDead { .. }
    );
    let journal_broken = journals.iter().any(|journal| {
        journal.phase.has_reached(
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted,
        )
    });
    Ok(PlanExecutionReferenceState {
        any_reference: run.is_some()
            || !references.records.is_empty()
            || references.initialization_journal.is_some()
            || !journals.is_empty(),
        broken: run_broken || advance_broken || journal_broken,
        run,
        references,
        journals,
    })
}

/// H2a（设计 2026-10-14 A/C/D 节）：零成员执行轮完整关闭闭包——检测、耐久
/// clearing、重新准入是同一受保护操作（Plan 独占租约经 `try_acquire_delete`
/// 取得，**不立 Plan 墓碑**：不得给仍保留的 Plan 立永久删除事实）。
///
/// 顺序（设计「完整重置闭包」）：只读预检（零成员/分类/Plan 删除互斥）→
/// 独占租约内复核 → 冻结纯 ID 清单＋耐久 clearing＋退役围栏事实＋原授权
/// Invalidated → 清 group 初始化 journal（含 attempts 已亡的 journal）与
/// WI 孤儿锁 → 目标仓专属资源清理（他人仍持有保留）→ advance retry →
/// 外层 journal → AdvanceRecords → 活动 run → 复核归零 → 轮次切换
/// （generation 单调 +1，纯 ID 回执）。任一步失败保持 clearing，同 command
/// 重入续清（missing=OK），异 command 409 `coding_plan_workspace_clearing`。
///
/// `require_broken_proof=true`（自动补清，如开始动作零成员路径）仅已证旧
/// 执行引用断裂才重置；`=false`（显式整组删除/最后成员删除尾部）零成员即
/// 关闭。Planning 会话、Plan/WI/revision/source/compile、上游 Story/Design
/// 不删（保留 plan/WI/revisions）。
pub(crate) fn close_plan_coding_workspace_round(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    command_id: &str,
    expected_generation: Option<u64>,
    require_broken_proof: bool,
) -> ApiResult<PlanCodingWorkspaceResetOutcome> {
    use crate::product::entity_deletion_store::{
        DeletionEntityKind, EntityDeletionStore, EntityScope,
    };
    use crate::product::entity_operation_store::EntityOperationStore;
    use crate::product::json_store::ProductStoreError;

    // H2a 稳定码直通：本函数内全部 store 错误经 workspace_reset_api_error
    // 映射（执行轮状态冲突码原样透出，其余归通用映射）。
    fn product_store_api_error(error: ProductStoreError) -> ApiError {
        workspace_reset_api_error(error)
    }

    let run_store = crate::product::coding_plan_run::CodingPlanRunStore::new(app_paths.clone());
    let lifecycle = run_store
        .get_workspace_lifecycle(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    let current_generation = lifecycle.as_ref().map_or(1, |metadata| metadata.generation);
    let mut resuming = false;
    if let Some(clearing) = lifecycle
        .as_ref()
        .and_then(|metadata| metadata.clearing.as_ref())
    {
        // 清理失败保持 clearing：同 command 重入续清；异 command 拒绝穿越。
        if clearing.command_id != command_id {
            return Err(product_store_api_error(ProductStoreError::Conflict {
                kind: "coding_plan_workspace_clearing",
                id: plan_id.to_string(),
            }));
        }
        if expected_generation.is_some_and(|generation| generation != clearing.generation) {
            return Err(product_store_api_error(ProductStoreError::Conflict {
                kind: "coding_plan_workspace_generation_conflict",
                id: plan_id.to_string(),
            }));
        }
        resuming = true;
    } else {
        if let Some(to_generation) = lifecycle.as_ref().and_then(|metadata| {
            metadata
                .receipts
                .iter()
                .find(|receipt| receipt.command_id == command_id)
                .map(|receipt| receipt.to_generation)
        }) {
            // 同 command 幂等重放：返回这一次操作已完成的轮次切换，不再次清。
            return Ok(PlanCodingWorkspaceResetOutcome::Reset {
                generation: to_generation,
            });
        }
        if expected_generation.is_some_and(|generation| generation != current_generation) {
            return Err(product_store_api_error(ProductStoreError::Conflict {
                kind: "coding_plan_workspace_generation_conflict",
                id: plan_id.to_string(),
            }));
        }
    }

    let scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
    // Plan 真删除闭包（REQ-CS-07/D2）own 本清理——墓碑在位时不越权重置。
    if EntityDeletionStore::new(app_paths.clone())
        .get(&scope)
        .map_err(product_store_api_error)?
        .is_some()
    {
        return Err(ApiError::runtime(
            "plan_coding_run_resource_unavailable",
            "plan deletion closure owns this cleanup; coding workspace reset is unavailable",
            json!({ "plan_id": plan_id }),
        ));
    }

    let coding_store = CodingAttemptStore::new(app_paths.clone());
    if !coding_store
        .list_attempts_for_plan(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?
        .is_empty()
    {
        // 部分删除状态走 Invalidated＋显式清剩余出口，不由本闭包缩水收养。
        return Err(product_store_api_error(ProductStoreError::Conflict {
            kind: "coding_plan_workspace_members_present",
            id: plan_id.to_string(),
        }));
    }
    if !resuming {
        let state = classify_plan_execution_references(app_paths, project_id, issue_id, plan_id)?;
        if !state.any_reference {
            return Ok(PlanCodingWorkspaceResetOutcome::Clean);
        }
        if !state.broken && require_broken_proof {
            return Ok(PlanCodingWorkspaceResetOutcome::InitializationPrefix);
        }
    }

    let lease = EntityOperationStore::new(app_paths.clone())
        .try_acquire_delete(&scope)
        .map_err(shared_resource_cleanup_api_error)?;
    close_plan_coding_workspace_round_with_lease(
        app_paths,
        &lease,
        plan_id,
        command_id,
        require_broken_proof,
        resuming,
    )
}

/// 闭包执行体：调用方持 Plan 独占租约（全临界区），每步幂等（missing=OK）。
#[allow(clippy::too_many_arguments)]
fn close_plan_coding_workspace_round_with_lease(
    app_paths: &ProductAppPaths,
    lease: &crate::product::entity_operation_store::EntityDeletionLease,
    plan_id: &str,
    command_id: &str,
    require_broken_proof: bool,
    resuming: bool,
) -> ApiResult<PlanCodingWorkspaceResetOutcome> {
    use crate::product::advance_store::{AdvancePlanArtifacts, AdvanceStore};
    use crate::product::coding_plan_run::{CodingPlanRunStore, CodingPlanWorkspaceResetChecklist};
    use crate::product::json_store::ProductStoreError;

    // H2a 稳定码直通：本函数内全部 store 错误经 workspace_reset_api_error
    // 映射（执行轮状态冲突码原样透出，其余归通用映射）。
    fn product_store_api_error(error: ProductStoreError) -> ApiError {
        workspace_reset_api_error(error)
    }

    let scope = lease.scope();
    let (project_id, issue_id) = (scope.project_id.as_str(), scope.issue_id.as_str());
    let lifecycle_store = LifecycleStore::new(app_paths.clone());
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let run_store = CodingPlanRunStore::new(app_paths.clone());
    let advance_store = AdvanceStore::new(app_paths.clone());

    // ① 锁内复核：plan 仍在、零成员、分类成立（预检与取租约之间的变更在此收敛）。
    lifecycle_store
        .get_issue_work_item_plan(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    if !coding_store
        .list_attempts_for_plan(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?
        .is_empty()
    {
        return Err(product_store_api_error(ProductStoreError::Conflict {
            kind: "coding_plan_workspace_members_present",
            id: plan_id.to_string(),
        }));
    }
    let state = classify_plan_execution_references(app_paths, project_id, issue_id, plan_id)?;
    if !resuming {
        if !state.any_reference {
            return Ok(PlanCodingWorkspaceResetOutcome::Clean);
        }
        if !state.broken && require_broken_proof {
            return Ok(PlanCodingWorkspaceResetOutcome::InitializationPrefix);
        }
    }

    // ② 冻结纯 ID 清单＋耐久 clearing＋退役围栏事实（merge 不覆盖既有退役）。
    let manifest = lifecycle_store
        .collect_entity_deletion_manifest(scope)
        .map_err(product_store_api_error)?;
    let mut retired_command_ids: Vec<String> = Vec::new();
    let mut retired_attempt_ids: Vec<String> = Vec::new();
    let mut pending_target_repository_ids: Vec<String> = manifest.target_repository_ids.clone();
    if let Some(run) = &state.run {
        retired_command_ids.push(run.command_id.clone());
        for binding in &run.target_attempts {
            retired_attempt_ids.push(binding.attempt_id.clone());
            pending_target_repository_ids.push(binding.target_repository_id.clone());
        }
        if let Some(selection) = &run.selection {
            retired_attempt_ids.push(selection.attempt_id.clone());
            pending_target_repository_ids.push(selection.target_repository_id.clone());
        }
    }
    for record in &state.references.records {
        retired_command_ids.push(record.command_id.clone());
    }
    for journal in &state.journals {
        retired_attempt_ids.push(journal.attempt.id.clone());
    }
    let checklist = CodingPlanWorkspaceResetChecklist {
        command_id: command_id.to_string(),
        retired_command_ids: retired_command_ids.clone(),
        retired_attempt_ids: retired_attempt_ids.clone(),
        retired_advance_ids: manifest.advance_ids.clone(),
        pending_attempt_ids: retired_attempt_ids,
        pending_advance_ids: manifest.advance_ids.clone(),
        pending_retry_command_ids: manifest.advance_retry_command_ids.clone(),
        pending_target_repository_ids,
    };
    let frozen = run_store
        .retire_workspace_round_with_lease(lease, &checklist)
        .map_err(product_store_api_error)?;
    // P2 修复轮（审查派回）：entry 的生命周期判定是无锁读——并发同 command
    // （双最后成员钩子/H4 沿用同 command）在读→租约间交错时，此处以租约内
    // retire 返回的 frozen 为权威核验 clearing 安装：clearing 缺失＝同
    // command 回执已在（另一并发调用已完整重置）→ 幂等返回轮次切换，
    // **绝不以陈旧快照继续 ③-⑦ 破坏性清理清掉新轮引用**；异 command
    // clearing 由 retire 拒绝（防御性复核同样拒绝）。
    match frozen.clearing.as_ref() {
        None => {
            return Ok(PlanCodingWorkspaceResetOutcome::Reset {
                generation: frozen.generation,
            });
        }
        Some(installed) if installed.command_id != command_id => {
            return Err(product_store_api_error(ProductStoreError::Conflict {
                kind: "coding_plan_workspace_clearing",
                id: plan_id.to_string(),
            }));
        }
        Some(_) => {}
    }
    // 原授权 Invalidated（幂等；阻断清理窗口内的本轮首启；run 已清则跳过）。
    if let Some(run) = &state.run
        && run_store
            .get(project_id, issue_id, plan_id)
            .map_err(product_store_api_error)?
            .is_some()
    {
        run_store
            .invalidate_run_for_reset_with_lease(lease, run)
            .map_err(product_store_api_error)?;
    }

    // ③ 清 group 初始化 journal（含 attempts 已亡的 journal；H2b 租约消费
    //    闭包：legacy 原路径 + per-target 子目录）＋WI 孤儿锁。
    for journal in &state.journals {
        purge_coding_attempt_lock_residue(
            app_paths,
            &journal.attempt,
            std::slice::from_ref(&journal.lock_work_item_id),
        );
    }
    coding_store
        .purge_group_initialization_journals_with_lease(lease)
        .map_err(product_store_api_error)?;

    // ④ 目标仓专属资源清理（真实 owner 复核：他人仍持有保留；缺失=已清）。
    //    本口径**只删记录、保留 worktree 实体**（实体被下一轮复用）。
    let shared_worktree_cleanup =
        crate::product::entity_deletion_cascade::cleanup_plan_shared_worktrees(
            app_paths,
            lease,
            &manifest,
            state.run.as_ref(),
            &[],
            &[],
            crate::product::entity_deletion_cascade::SharedWorktreeEntityMode::RecordOnly,
        )
        .map_err(product_store_api_error)?;
    crate::product::entity_deletion_cascade::delete_shared_worktree_records(
        app_paths,
        &shared_worktree_cleanup,
    )
    .map_err(product_store_api_error)?;

    // ⑤ advance retry → 外层初始化 journal → AdvanceRecords → 活动 run。
    advance_store
        .purge_plan_artifacts(
            lease,
            &AdvancePlanArtifacts {
                advance_ids: manifest.advance_ids.clone(),
                retry_command_ids: manifest.advance_retry_command_ids.clone(),
            },
        )
        .map_err(product_store_api_error)?;
    run_store
        .purge_plan_with_lease(lease)
        .map_err(product_store_api_error)?;

    // ⑥ 复核本轮活动执行引用归零（专属资源可能因他人保留，不在复核列）。
    let residue = advance_store
        .list_plan_artifacts(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    if !residue.advance_ids.is_empty()
        || !residue.retry_command_ids.is_empty()
        || !coding_store
            .list_group_initialization_journals_for_plan(project_id, issue_id, plan_id)
            .map_err(product_store_api_error)?
            .is_empty()
        || run_store
            .get(project_id, issue_id, plan_id)
            .map_err(product_store_api_error)?
            .is_some()
    {
        return Err(product_store_api_error(ProductStoreError::Conflict {
            kind: "coding_plan_workspace_clearing",
            id: plan_id.to_string(),
        }));
    }

    // ⑦ 轮次切换：generation 单调 +1、清 pending、纯 ID 回执。
    let metadata = run_store
        .complete_workspace_reset_with_lease(lease, command_id)
        .map_err(product_store_api_error)?;
    Ok(PlanCodingWorkspaceResetOutcome::Reset {
        generation: metadata.generation,
    })
}

// ─── D2.2（fix-cascade-deletion Task 3.2）：三类 DELETE 的目标仓资源清理 ───

// ─── H2a（设计 2026-10-14 A/C/D 节）：零成员闭包与最后成员钩子测试 ───

#[cfg(test)]
mod h2a_workspace_reset_tests {
    use super::*;
    use crate::product::coding_plan_run::workspace_reset_fixtures::{
        ISSUE_ID, PLAN_ID, PROJECT_ID, n2_fixture,
    };
    use crate::product::coding_plan_run::{CodingPlanRunPhase, CodingPlanRunStore};
    use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
    use crate::product::entity_operation_store::EntityOperationStore;

    fn plan_scope() -> EntityScope {
        EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID)
    }

    fn advance_root(paths: &ProductAppPaths) -> std::path::PathBuf {
        paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("advance-records")
    }

    fn attempt_file(paths: &ProductAppPaths, attempt_id: &str) -> std::path::PathBuf {
        paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(format!("{attempt_id}.json"))
    }

    fn per_target_journal_file(
        paths: &ProductAppPaths,
        target_repository_id: &str,
    ) -> std::path::PathBuf {
        paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join("group-initializations")
            .join(PLAN_ID)
            .join(format!("{target_repository_id}.json"))
    }

    /// H2a T4（设计 E 节验收 1/2 的存储面）：全删后完整关闭旧执行轮——
    /// 最后成员删除自动接零成员闭包；run/advance/初始化引用清、
    /// Plan/WI/revisions 保留；轮次切换（generation 2、回执、退役围栏）；
    /// 同 command 幂等重放不再次清新轮。
    #[tokio::test]
    async fn h2a_last_member_delete_closes_old_round_completely() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();
        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_h2a_w_round1",
                &fixture.revision_id,
            )
            .unwrap();
        run_store.bind_ready(&run, &fixture.bindings, None).unwrap();
        let coding_store = fixture.coding_store();
        let api = fixture.api_attempt();
        let web = fixture.web_attempt();

        // 部分删除：run Invalidated、闭包不触发、剩余成员保留（partial 拒收养）。
        finalize_coding_attempt_deletion(&coding_store, &fixture.paths, &api).unwrap();
        let stored = run_store
            .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .expect("run kept for the partial state");
        assert!(matches!(stored.phase, CodingPlanRunPhase::Invalidated));
        assert!(
            run_store
                .get_workspace_lifecycle(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .is_none(),
            "partial delete must not start clearing"
        );
        assert_eq!(
            coding_store
                .list_attempts_for_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .len(),
            1
        );

        // 最后成员删除：接同一零成员闭包，完整关闭旧执行轮。
        finalize_coding_attempt_deletion(&coding_store, &fixture.paths, &web).unwrap();

        assert!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .is_none()
        );
        let artifacts = crate::product::advance_store::AdvanceStore::new(fixture.paths.clone())
            .list_plan_artifacts(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert!(artifacts.advance_ids.is_empty());
        assert!(artifacts.retry_command_ids.is_empty());
        assert!(
            !advance_root(&fixture.paths)
                .join(format!("{PLAN_ID}.initialization.json"))
                .exists()
        );
        assert!(
            coding_store
                .list_group_initialization_journals_for_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .is_empty()
        );
        let lifecycle = run_store
            .get_workspace_lifecycle(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .expect("lifecycle metadata after reset");
        assert_eq!(lifecycle.generation, 2);
        assert!(lifecycle.clearing.is_none());
        assert_eq!(lifecycle.receipts.len(), 1);
        assert_eq!(lifecycle.receipts[0].to_generation, 2);
        assert!(
            lifecycle
                .retired_command_ids
                .contains(&"cmd_h2a_w_round1".to_string())
        );
        // 保留 Plan/WI/revisions。
        LifecycleStore::new(fixture.paths.clone())
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        let lineage = crate::product::work_item_revision_store::WorkItemRevisionStore::new(
            fixture.paths.clone(),
        )
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap();
        assert_eq!(
            lineage.active_revision_id.as_deref(),
            Some(fixture.revision_id.as_str())
        );
        // 旧 command 复跑拒绝；新 command 开新轮。
        let error = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_h2a_w_round1",
                &fixture.revision_id,
            )
            .unwrap_err();
        assert!(
            matches!(
                error,
                crate::product::json_store::ProductStoreError::Conflict { kind, .. }
                    if kind == "coding_plan_run_retired"
            ),
            "retired command must not re-prepare after full reset: {error}"
        );
        run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_h2a_w_round2",
                &fixture.revision_id,
            )
            .unwrap();

        // 同 command 幂等重放：返回已完成的轮次切换，不再次清新轮
        //（新 run 已存在且不得被触碰）。
        let replay_command = format!("plan_workspace_close_{PLAN_ID}_g1");
        match close_plan_coding_workspace_round(
            &fixture.paths,
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            &replay_command,
            None,
            false,
        )
        .unwrap()
        {
            PlanCodingWorkspaceResetOutcome::Reset { generation } => assert_eq!(generation, 2),
            other => panic!("expected idempotent replay, got {other:?}"),
        }
        assert!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .is_some(),
            "replay must not clear the fresh round"
        );
    }

    /// H2a T5（设计 A 节「journal 在而 attempts 亡」）：仅 Prepared 前缀
    /// journal 时自动补清不重置（前缀继续原协议、零变更）；显式关闭
    ///（require_broken_proof=false）清前缀 journal 并完成轮次切换。
    #[tokio::test]
    async fn h2a_initialization_prefix_is_not_reset_automatically() {
        let fixture = n2_fixture().await;
        let coding_store = fixture.coding_store();
        let journals = coding_store
            .list_group_initialization_journals_for_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(journals.len(), 2);
        let mut prefix_journals = Vec::new();
        for journal in &journals {
            let mut prefix = journal.clone();
            prefix.phase =
                crate::product::coding_attempt_store::CodingGroupInitializationPhase::Prepared;
            let target_repository_id = prefix
                .attempt
                .target_snapshot
                .as_ref()
                .expect("target snapshot")
                .logical_repository_id
                .0
                .to_string();
            prefix_journals.push((target_repository_id, prefix));
        }

        // 半残现场：attempt JSON 已亡（外部路径删除）、run/advance 全清，
        // 仅回放一个 Prepared 前缀 journal。
        for attempt in [fixture.api_attempt(), fixture.web_attempt()] {
            std::fs::remove_file(attempt_file(&fixture.paths, &attempt.id)).unwrap();
        }
        let scope = plan_scope();
        let lease = EntityOperationStore::new(fixture.paths.clone())
            .try_acquire_delete(&scope)
            .unwrap();
        let advance_store = crate::product::advance_store::AdvanceStore::new(fixture.paths.clone());
        let artifacts = advance_store
            .list_plan_artifacts(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        advance_store
            .purge_plan_artifacts(&lease, &artifacts)
            .unwrap();
        drop(lease);
        for (target_repository_id, prefix) in &prefix_journals {
            let path = per_target_journal_file(&fixture.paths, target_repository_id);
            crate::product::json_store::write_json(&path, prefix).unwrap();
        }
        let journal_path = per_target_journal_file(&fixture.paths, &prefix_journals[0].0);

        // 自动补清（require_broken_proof=true）：前缀保护，零变更。
        match close_plan_coding_workspace_round(
            &fixture.paths,
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "cmd_h2a_prefix_auto",
            None,
            true,
        )
        .unwrap()
        {
            PlanCodingWorkspaceResetOutcome::InitializationPrefix => {}
            other => panic!("expected prefix protection, got {other:?}"),
        }
        assert!(journal_path.is_file());
        assert!(
            CodingPlanRunStore::new(fixture.paths.clone())
                .get_workspace_lifecycle(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .is_none()
        );

        // 显式关闭（require_broken_proof=false）：清前缀 journal + 轮次切换。
        match close_plan_coding_workspace_round(
            &fixture.paths,
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "cmd_h2a_prefix_close",
            None,
            false,
        )
        .unwrap()
        {
            PlanCodingWorkspaceResetOutcome::Reset { generation } => assert_eq!(generation, 2),
            other => panic!("expected explicit close, got {other:?}"),
        }
        assert!(!journal_path.is_file());
    }

    /// H2a T6（设计 A 节状态表「重置清理中断」）：清单已冻结、run 已单独
    /// 清掉的中断形态——同 command 重入续清补完（advance/journal 残留清、
    /// generation 切换），异 command 409 `coding_plan_workspace_clearing`
    /// 拒绝穿越。
    #[tokio::test]
    async fn h2a_clearing_resume_and_command_exclusivity() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();
        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_h2a_r_round1",
                &fixture.revision_id,
            )
            .unwrap();
        run_store.bind_ready(&run, &fixture.bindings, None).unwrap();
        let coding_store = fixture.coding_store();

        // 半残现场（原事故形态）：attempt JSON 已亡、journals/advance 残留、
        // 清单冻结后中断、run 文件已被单独清掉。
        for attempt in [fixture.api_attempt(), fixture.web_attempt()] {
            std::fs::remove_file(attempt_file(&fixture.paths, &attempt.id)).unwrap();
        }
        let scope = plan_scope();
        let lease = EntityOperationStore::new(fixture.paths.clone())
            .try_acquire_delete(&scope)
            .unwrap();
        let advance_store = crate::product::advance_store::AdvanceStore::new(fixture.paths.clone());
        let artifacts = advance_store
            .list_plan_artifacts(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(artifacts.advance_ids.len(), 1);
        use crate::product::coding_plan_run::CodingPlanWorkspaceResetChecklist;
        let checklist = CodingPlanWorkspaceResetChecklist {
            command_id: "cmd_h2a_r_close".to_string(),
            retired_command_ids: vec!["cmd_h2a_r_round1".to_string()],
            retired_attempt_ids: fixture
                .bindings
                .iter()
                .map(|binding| binding.attempt_id.clone())
                .collect(),
            retired_advance_ids: artifacts.advance_ids.clone(),
            pending_attempt_ids: fixture
                .bindings
                .iter()
                .map(|binding| binding.attempt_id.clone())
                .collect(),
            pending_advance_ids: artifacts.advance_ids.clone(),
            pending_retry_command_ids: artifacts.retry_command_ids.clone(),
            pending_target_repository_ids: fixture
                .bindings
                .iter()
                .map(|binding| binding.target_repository_id.clone())
                .collect(),
        };
        run_store
            .retire_workspace_round_with_lease(&lease, &checklist)
            .unwrap();
        run_store.purge_plan_with_lease(&lease).unwrap();
        drop(lease);

        // 异 command 拒绝穿越 clearing。
        let error = close_plan_coding_workspace_round(
            &fixture.paths,
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "cmd_h2a_r_other",
            None,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, "coding_plan_workspace_clearing");

        // 同 command 续清：journal/advance 残留清完 + 轮次切换。
        match close_plan_coding_workspace_round(
            &fixture.paths,
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "cmd_h2a_r_close",
            None,
            false,
        )
        .unwrap()
        {
            PlanCodingWorkspaceResetOutcome::Reset { generation } => assert_eq!(generation, 2),
            other => panic!("expected resume completion, got {other:?}"),
        }
        let artifacts_after = advance_store
            .list_plan_artifacts(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert!(artifacts_after.advance_ids.is_empty());
        assert!(
            coding_store
                .list_group_initialization_journals_for_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .is_empty()
        );
        let lifecycle = run_store
            .get_workspace_lifecycle(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .expect("lifecycle after resume");
        assert_eq!(lifecycle.generation, 2);
        assert!(lifecycle.clearing.is_none());
    }

    /// H2a P2 修复轮（审查派回）：并发同 command 交错窗——第一个同 command
    /// 调用已完成轮次切换、新轮 run 已重建后，持陈旧「无 clearing」判定的
    /// 第二个调用进入租约：锁内核验 retire 返回的 frozen，clearing 缺失
    /// （同 command 回执已在）＝幂等返回 Reset，**绝不继续 ③-⑦ 破坏性
    /// 清理清掉新轮引用**；陈旧 expected_generation 被 generation 守卫拒。
    #[tokio::test]
    async fn h2a_concurrent_same_command_interleave_keeps_new_round() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();
        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_h2a_x_round1",
                &fixture.revision_id,
            )
            .unwrap();
        run_store.bind_ready(&run, &fixture.bindings, None).unwrap();
        let _coding_store = fixture.coding_store();

        // 零成员半残（原事故形态）：attempt JSON 已亡、journal/advance 残留。
        for attempt in [fixture.api_attempt(), fixture.web_attempt()] {
            std::fs::remove_file(attempt_file(&fixture.paths, &attempt.id)).unwrap();
        }

        // 第一个同 command 调用完整重置（generation 1→2）。
        match close_plan_coding_workspace_round(
            &fixture.paths,
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "cmd_h2a_x_close",
            None,
            false,
        )
        .unwrap()
        {
            PlanCodingWorkspaceResetOutcome::Reset { generation } => assert_eq!(generation, 2),
            other => panic!("expected first reset, got {other:?}"),
        }

        // 新轮已重建（新 command 的 prepared run）。
        let fresh = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_h2a_x_round2",
                &fixture.revision_id,
            )
            .unwrap();
        assert_eq!(fresh.command_id, "cmd_h2a_x_round2");

        // 交错窗：第二个同 command 调用已在旧状态完成 entry 判定（无
        // clearing、非 resuming），现取到租约——直接进执行体模拟该窗口。
        let scope = plan_scope();
        let lease = EntityOperationStore::new(fixture.paths.clone())
            .try_acquire_delete(&scope)
            .unwrap();
        match close_plan_coding_workspace_round_with_lease(
            &fixture.paths,
            &lease,
            PLAN_ID,
            "cmd_h2a_x_close",
            false,
            false,
        )
        .unwrap()
        {
            PlanCodingWorkspaceResetOutcome::Reset { generation } => assert_eq!(generation, 2),
            other => panic!("expected idempotent replay under lease, got {other:?}"),
        }
        drop(lease);
        // 新轮引用不被清（破坏性清理不得以陈旧快照清掉新轮 run）。
        let kept = run_store
            .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .expect("new round run must survive the interleaved replay");
        assert_eq!(kept.command_id, "cmd_h2a_x_round2");

        // 陈旧 expected_generation：generation 守卫拒（版本过期刷新后重新决策）。
        let error = close_plan_coding_workspace_round(
            &fixture.paths,
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "cmd_h2a_x_stale",
            Some(1),
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, "coding_plan_workspace_generation_conflict");
    }
}
