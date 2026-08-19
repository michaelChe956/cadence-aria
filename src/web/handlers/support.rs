use super::*;
use crate::product::logical_codebase::{
    LogicalCodebaseStore, LogicalRepositoryId, RepositoryRouting, RepositoryRoutingErrorCode,
};
pub(crate) use crate::web::handlers::gateway_error_mapping::{
    coding_gateway_api_error, provider_gateway_error_code,
};

#[derive(Debug, Deserialize)]
pub struct ProjectionQuery {
    pub workspace_id: Option<String>,
    pub task_id: Option<String>,
    pub node_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FileContentQuery {
    pub workspace_id: Option<String>,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct FileDiffQuery {
    pub workspace_id: Option<String>,
    pub base_checkpoint: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceQuery {
    pub workspace_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GateResolveQuery {
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    pub cursor: Option<u64>,
}
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct ProviderWorkspaceConfig {
    pub(crate) author_provider: ProviderName,
    pub(crate) reviewer_provider: ProviderName,
    pub(crate) author_status_code: &'static str,
    pub(crate) reviewer_status_code: &'static str,
    pub(crate) review_rounds: u32,
    pub(crate) superpowers_enabled: bool,
    pub(crate) openspec_enabled: bool,
}
pub(crate) fn canonical_provider_input_path(
    workspace_root: &StdPath,
    runtime_tasks_root: &StdPath,
    task_root: &StdPath,
    file_name: &str,
) -> ApiResult<PathBuf> {
    let workspace_root = canonical_provider_input_component(workspace_root)?;
    let runtime_tasks_root = canonical_provider_input_component(runtime_tasks_root)?;
    if !runtime_tasks_root.starts_with(&workspace_root) {
        return Err(provider_input_path_escape());
    }
    let task_root = canonical_provider_input_component(task_root)?;
    if !task_root.starts_with(&runtime_tasks_root) {
        return Err(provider_input_path_escape());
    }

    let provider_inputs_root = task_root.join("provider-inputs");
    let provider_inputs_root = canonical_provider_input_component(&provider_inputs_root)?;
    if !provider_inputs_root.starts_with(&task_root) {
        return Err(provider_input_path_escape());
    }

    let candidate = provider_inputs_root.join(file_name);
    let candidate = canonical_provider_input_component(&candidate)?;
    if !candidate.starts_with(&provider_inputs_root) {
        return Err(provider_input_path_escape());
    }

    Ok(candidate)
}

pub(crate) fn canonical_provider_input_component(path: &StdPath) -> ApiResult<PathBuf> {
    fs::canonicalize(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => {
            ApiError::runtime("artifact_not_found", "provider input not found", json!({}))
        }
        _ => ApiError::runtime(
            "provider_input_read_failed",
            "provider input read failed",
            json!({}),
        ),
    })
}

pub(crate) fn provider_input_path_escape() -> ApiError {
    ApiError::validation(
        "provider_input_path_escape",
        "provider input path escapes task root",
    )
}

pub async fn events(
    State(state): State<WebAppState>,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (replay_events, receiver) = state
        .events
        .subscribe_with_replay_after(query.cursor.unwrap_or(0));
    let replay_stream = stream::iter(replay_events);
    let live_stream = BroadcastStream::new(receiver).filter_map(|event| async move { event.ok() });
    let sse_stream = replay_stream
        .chain(live_stream)
        .map(|event| Ok::<Event, Infallible>(sse_event(event)));
    Sse::new(sse_stream).keep_alive(KeepAlive::default())
}

pub(crate) fn sse_event(event: WebEvent) -> Event {
    Event::default()
        .id(event.cursor.to_string())
        .event(event.event_type.clone())
        .json_data(event)
        .expect("serialize web event")
}
pub(crate) fn resolve_workspace_root(
    app_root: &std::path::Path,
    workspace_id: Option<&str>,
    task_id: Option<&str>,
) -> ApiResult<std::path::PathBuf> {
    let workspace_registry = WorkspaceRegistry::new(app_root.to_path_buf());
    if let Some(workspace_id) = workspace_id {
        match workspace_registry.get(workspace_id) {
            Ok(workspace) => return Ok(workspace.path),
            Err(error) if error.code() == "workspace_not_found" => {
                if let Some((project_id, repository_id)) =
                    parse_product_execution_workspace_id(workspace_id)
                {
                    let app_paths = ProductAppPaths::new(app_root.join(".aria"));
                    return Ok(find_repository(&app_paths, project_id, repository_id)?.path);
                }
                return Err(error.into());
            }
            Err(error) => return Err(error.into()),
        }
    }
    if let Some(task_id) = task_id {
        match IssueRegistry::new(app_root.to_path_buf()).find_by_task(task_id) {
            Ok(link) => return Ok(workspace_registry.get(&link.workspace_id)?.path),
            Err(error) if error.code() == "task_workspace_not_found" => {
                return Ok(app_root.to_path_buf());
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(app_root.to_path_buf())
}

pub(crate) fn provider_input_file_name(input_ref: &str) -> ApiResult<String> {
    if input_ref.is_empty()
        || input_ref.contains('/')
        || input_ref.contains('\\')
        || input_ref.contains("..")
        || !input_ref
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return Err(ApiError::validation(
            "invalid_file_path",
            "invalid provider input ref",
        ));
    }
    Ok(if input_ref.ends_with(".json") {
        input_ref.to_string()
    } else {
        format!("{input_ref}.json")
    })
}
pub(crate) fn find_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    repository_id: &str,
) -> ApiResult<RepositoryRecord> {
    let project = ProjectStore::new(app_paths.clone())
        .get(project_id)
        .map_err(product_store_api_error)?;
    RepositoryStore::for_project(app_paths.clone(), &project)
        .list(project_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|repository| repository.id == repository_id)
        .ok_or_else(|| {
            product_store_api_error(ProductStoreError::NotFound {
                kind: "repository",
                id: repository_id.to_string(),
            })
        })
}

pub(crate) fn product_execution_workspace_id(project_id: &str, repository_id: &str) -> String {
    format!("product:{project_id}:{repository_id}")
}

pub(crate) fn parse_product_execution_workspace_id(value: &str) -> Option<(&str, &str)> {
    let mut parts = value.split(':');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("product"), Some(project_id), Some(repository_id), None) => {
            Some((project_id, repository_id))
        }
        _ => None,
    }
}
pub(crate) fn product_app_paths(state: &WebAppState) -> ProductAppPaths {
    ProductAppPaths::new(state.workspace_root.join(".aria"))
}

/// 过渡 guard：在 R3/R5 改为请求/issue 所属代码库之前，只有存在逻辑代码库存储
/// 的 project 才能访问旧 logical-codebase 兼容端点。纯单仓 project 不创建任何
/// logical-codebase durable artifact。
pub(crate) fn require_multi_repo_project(
    paths: &ProductAppPaths,
    project_id: &str,
) -> ApiResult<ProjectRecord> {
    let project = ProjectStore::new(paths.clone())
        .get(project_id)
        .map_err(product_store_api_error)?;
    let has_storage = LogicalCodebaseStore::new(paths.clone())
        .has_any_storage(project_id)
        .map_err(product_store_api_error)?;
    if !has_storage {
        return Err(logical_codebase_feature_disabled_api_error(project_id));
    }
    Ok(project)
}

/// Kept as a source-compatible call site during R1. R6 removes the retired
/// project-mode protection entirely because repository mode belongs to a codebase.
pub(crate) fn reject_legacy_repository_endpoint_on_multi_repo(
    _project: &ProjectRecord,
) -> ApiResult<()> {
    Ok(())
}

pub(crate) fn provider_workspace_config(
    author_provider: Option<&str>,
    reviewer_provider: Option<&str>,
    review_rounds: Option<u32>,
    superpowers_enabled: Option<bool>,
    openspec_enabled: Option<bool>,
    provider_availability: &dyn Fn(&ProviderName) -> bool,
) -> ApiResult<ProviderWorkspaceConfig> {
    let review_rounds = review_rounds.unwrap_or(1);
    if !(1..=5).contains(&review_rounds) {
        return Err(ApiError::validation(
            "invalid_review_rounds",
            "review_rounds must be between 1 and 5",
        ));
    }

    let author = match author_provider {
        Some(provider) => resolve_explicit_provider_name(provider, provider_availability)?,
        None => resolve_default_coding_provider("codex", provider_availability)?,
    };
    let reviewer = match reviewer_provider {
        Some(provider) => resolve_explicit_provider_name(provider, provider_availability)?,
        None => resolve_default_coding_provider("claude_code", provider_availability)?,
    };

    Ok(ProviderWorkspaceConfig {
        author_provider: author.provider,
        reviewer_provider: reviewer.provider,
        author_status_code: author.status_code,
        reviewer_status_code: reviewer.status_code,
        review_rounds,
        superpowers_enabled: superpowers_enabled.unwrap_or(true),
        openspec_enabled: openspec_enabled.unwrap_or(true),
    })
}
include!("support_parts/product_store_error.inc.rs");

/// 当 work item group 存在 coding workspace 时拒绝删除，提示先删除 coding workspace。
pub(crate) fn coding_workspace_exists_error(plan_id: &str, attempt_id: &str) -> ApiError {
    ApiError::runtime(
        "coding_workspace_exists",
        "存在 coding workspace，请先删除 coding workspace 再删除 work item group",
        json!({ "plan_id": plan_id, "attempt_id": attempt_id }),
    )
}

/// 当单个 work item 存在 coding workspace 时拒绝删除该 work item。
///
/// 与 `coding_workspace_exists_error` 共用错误码（前端按 `coding_workspace_exists` 统一处理），
/// 但 details 用 `work_item_id` 而非 `plan_id`——work item 级删除入口没有 plan 上下文，
/// 给出真实标识便于定位。同样映射到 409 CONFLICT。
pub(crate) fn coding_workspace_exists_for_work_item_error(
    work_item_id: &str,
    attempt_id: &str,
) -> ApiError {
    ApiError::runtime(
        "coding_workspace_exists",
        "存在 coding workspace，请先删除 coding workspace 再删除 work item",
        json!({ "work_item_id": work_item_id, "attempt_id": attempt_id }),
    )
}

pub(crate) fn node_detail_store_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::NotFound {
            kind: "node_detail",
            ..
        } => ApiError::runtime("node_detail_not_found", "node detail not found", json!({})),
        other => product_store_api_error(other),
    }
}
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
    match &attempt.target_snapshot {
        Some(snapshot) => cleanup_repo_shared_worktree_if_no_active_attempts(
            coding_store,
            app_paths,
            &attempt.project_id,
            &attempt.issue_id,
            snapshot.logical_repository_id,
        )?,
        None => cleanup_issue_shared_worktree_if_no_attempts(
            coding_store,
            app_paths,
            &attempt.project_id,
            &attempt.issue_id,
        )?,
    }
    Ok(())
}

pub(crate) fn git_workspace_api_error(error: GitWorkspaceError) -> ApiError {
    ApiError::runtime(
        "git_workspace_cleanup_failed",
        "git workspace cleanup failed",
        json!({"details": error.to_string()}),
    )
}

pub(crate) fn coding_workspace_engine_with_dummy_events(
    store: CodingAttemptStore,
) -> CodingWorkspaceEngine {
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
}

pub(crate) fn coding_workspace_api_error(error: CodingWorkspaceEngineError) -> ApiError {
    let error_message = error.to_string();
    let fallback = || {
        ApiError::runtime(
            "coding_workspace_engine_failed",
            "coding workspace engine operation failed",
            json!({"details": error_message}),
        )
    };
    match &error {
        CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(_) => ApiError::runtime(
            "shared_worktree_dirty_manual_gate",
            "shared worktree has uncommitted changes; manual cleanup required",
            json!({"details": error_message}),
        ),
        CodingWorkspaceEngineError::LegacySharedWorktreePresent(_) => ApiError::runtime(
            "legacy_shared_worktree_present",
            "legacy issue shared worktree blocks the repository worktree route",
            json!({"details": error_message}),
        ),
        CodingWorkspaceEngineError::CrossTargetDeliveryBlocked(stable_code) => {
            // StableCode 字符串经 CrossTargetDeliveryBlocked(String) 承载：
            // cross_target_violation_detected / cross_target_baseline_missing /
            // cross_target_store_failure 显式透传，其余按未知码兜底 500。
            let code = match stable_code.as_str() {
                "cross_target_violation_detected"
                | "cross_target_baseline_missing"
                | "cross_target_store_failure" => stable_code.as_str(),
                _ => "cross_target_delivery_blocked",
            };
            ApiError::runtime(
                code,
                "cross-target delivery is blocked",
                json!({"details": error_message}),
            )
        }
        CodingWorkspaceEngineError::Store(ProductStoreError::Io(message)) => {
            match message.as_str() {
                "target_snapshot_missing_for_logical" => ApiError::runtime(
                    "target_snapshot_missing_for_logical",
                    "logical coding attempt is missing its target snapshot",
                    json!({}),
                ),
                "target_snapshot_identity_drifted" => ApiError::runtime(
                    "target_snapshot_identity_drifted",
                    "coding attempt target snapshot identity drifted",
                    json!({}),
                ),
                "target_snapshot_policy_drifted" => ApiError::runtime(
                    "target_snapshot_policy_drifted",
                    "coding attempt target snapshot policy drifted",
                    json!({}),
                ),
                "legacy_shared_worktree_inconsistent" => ApiError::runtime(
                    "legacy_shared_worktree_inconsistent",
                    "legacy issue shared worktree migration record is inconsistent",
                    json!({}),
                ),
                _ => fallback(),
            }
        }
        // T11 fix round:gateway 错误(ProviderStream/ProviderAdapter 承载)归一为稳定码;
        // 未命中保持原 fallback。
        CodingWorkspaceEngineError::ProviderStream(_)
        | CodingWorkspaceEngineError::ProviderAdapter(_) => {
            match coding_gateway_api_error(&error) {
                Some(api_error) => api_error,
                None => fallback(),
            }
        }
        _ => fallback(),
    }
}

pub(crate) fn git_workspace_diff_api_error(error: GitWorkspaceError) -> ApiError {
    ApiError::runtime(
        "git_workspace_diff_failed",
        "git workspace diff failed",
        json!({"details": error.to_string()}),
    )
}

pub(crate) fn is_git_repo(path: &StdPath) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn current_git_branch(path: &StdPath) -> Option<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(path)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty()).then_some(branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn availability(provider: &ProviderName) -> bool {
        matches!(provider, ProviderName::ClaudeCode)
    }

    #[test]
    fn provider_workspace_config_rejects_explicit_unavailable_provider() {
        let error = provider_workspace_config(Some("codex"), None, None, None, None, &availability)
            .expect_err("explicit unavailable provider must fail");

        assert_eq!(error.code, "provider_unavailable");
        assert_eq!(error.details["provider"], "codex");
    }

    #[test]
    fn provider_workspace_config_records_default_fallback_status() {
        let config = provider_workspace_config(None, None, None, None, None, &availability)
            .expect("default provider config");

        assert_eq!(config.author_provider, ProviderName::ClaudeCode);
        assert_eq!(config.author_status_code, "provider_fallback");
        assert_eq!(config.reviewer_provider, ProviderName::ClaudeCode);
        assert_eq!(config.reviewer_status_code, "provider_available");
    }

    #[test]
    fn product_store_api_error_maps_registration_codes_to_stable_http_statuses() {
        let cases = [
            (
                ProductStoreError::NotFound {
                    kind: "registration_preflight",
                    id: "preflight_0001".to_string(),
                },
                "registration_preflight_not_found",
                StatusCode::NOT_FOUND,
            ),
            (
                ProductStoreError::Conflict {
                    kind: "registration_batch_candidate_identity_changed",
                    id: "sha256:source".to_string(),
                },
                "registration_batch_conflict",
                StatusCode::CONFLICT,
            ),
            (
                ProductStoreError::Conflict {
                    kind: "aggregate_root_mismatch",
                    id: "project_0001".to_string(),
                },
                "aggregate_root_mismatch",
                StatusCode::CONFLICT,
            ),
            (
                ProductStoreError::Conflict {
                    kind: "aggregate_initialization",
                    id: "aggregate_initialization_0001".to_string(),
                },
                "aggregate_initialization_conflict",
                StatusCode::CONFLICT,
            ),
            (
                ProductStoreError::IdentityMismatch {
                    kind: "registration_batch_member_recovery",
                    id: "sha256:source".to_string(),
                },
                "registration_batch_conflict",
                StatusCode::CONFLICT,
            ),
        ];
        for (store_error, code, status) in cases {
            let error = product_store_api_error(store_error);
            assert_eq!(error.code, code);
            assert_eq!(error.into_response().status(), status);
        }
    }

    #[test]
    fn product_store_api_error_maps_routing_kinds_to_stable_codes() {
        // B3：routing 相关 ProductStoreError → 稳定错误码 + 4xx。
        let error = product_store_api_error(ProductStoreError::Ambiguous {
            kind: "issue_codebase_selection",
            id: "issue_0001".to_string(),
        });

        assert_eq!(error.code, "repository_routing_ambiguous");
        assert_eq!(error.details["kind"], "issue_codebase_selection");
        assert_eq!(error.details["id"], "issue_0001");
        assert_eq!(error.into_response().status(), StatusCode::CONFLICT);
    }

    #[test]
    fn product_store_api_error_maps_routing_store_kinds_with_diagnostic_details() {
        let cases = [
            (
                ProductStoreError::NotFound {
                    kind: "issue_codebase_selection",
                    id: "issue_0001".to_string(),
                },
                "repository_routing_target_missing",
                StatusCode::UNPROCESSABLE_ENTITY,
                "issue_codebase_selection",
                "issue_0001",
            ),
            (
                ProductStoreError::NotFound {
                    kind: "logical_repository",
                    id: "logical_0001".to_string(),
                },
                "repository_routing_target_unknown",
                StatusCode::NOT_FOUND,
                "logical_repository",
                "logical_0001",
            ),
            (
                ProductStoreError::Ambiguous {
                    kind: "logical_repository",
                    id: "logical_0001".to_string(),
                },
                "repository_routing_ambiguous",
                StatusCode::CONFLICT,
                "logical_repository",
                "logical_0001",
            ),
            (
                ProductStoreError::Conflict {
                    kind: "logical_repository",
                    id: "logical_0001".to_string(),
                },
                "repository_routing_inconsistent",
                StatusCode::CONFLICT,
                "logical_repository",
                "logical_0001",
            ),
            (
                ProductStoreError::IdentityMismatch {
                    kind: "logical_repository",
                    id: "logical_0001".to_string(),
                },
                "repository_routing_inconsistent",
                StatusCode::CONFLICT,
                "logical_repository",
                "logical_0001",
            ),
        ];

        for (store_error, expected_code, expected_status, kind, id) in cases {
            let error = product_store_api_error(store_error);
            assert_eq!(error.code, expected_code);
            assert_eq!(error.details["kind"], kind);
            assert_eq!(error.details["id"], id);
            assert_eq!(error.into_response().status(), expected_status);
        }
    }

    #[test]
    fn product_store_api_error_maps_actual_identity_resolution_kinds_to_stable_codes() {
        // RepositoryStore::identity_resolution_error 发出的真实 kind 必须保持 4xx，
        // 不得退回 product_store_error（500）。
        let cases = [
            (
                ProductStoreError::NotFound {
                    kind: "identity_resolution_missing",
                    id: "logical_0001".to_string(),
                },
                "repository_routing_target_unknown",
                StatusCode::NOT_FOUND,
                "identity_resolution_missing",
            ),
            (
                ProductStoreError::Ambiguous {
                    kind: "identity_resolution_ambiguous",
                    id: "logical_0001".to_string(),
                },
                "repository_routing_ambiguous",
                StatusCode::CONFLICT,
                "identity_resolution_ambiguous",
            ),
        ];

        for (store_error, expected_code, expected_status, expected_kind) in cases {
            let error = product_store_api_error(store_error);
            assert_eq!(error.code, expected_code);
            assert_eq!(error.details["kind"], expected_kind);
            assert_eq!(error.details["id"], "logical_0001");
            assert_eq!(error.into_response().status(), expected_status);
        }
    }

    #[test]
    fn product_store_api_error_preserves_routing_kind_and_scopes_invalid_record_mapping() {
        let manifest_error = product_store_api_error(ProductStoreError::NotFound {
            kind: "logical_repository_manifest",
            id: "project_0001".to_string(),
        });
        assert_eq!(manifest_error.code, "repository_routing_inconsistent");
        assert_eq!(
            manifest_error.details["kind"],
            "logical_repository_manifest"
        );

        // 非 routing kind 即使带有 routing 词汇，也不得被重写为 routing 稳定码。
        let unrelated_error = product_store_api_error(ProductStoreError::InvalidRecord {
            kind: "unrelated_record",
            reason: "member_removed: unrelated record repair failed".to_string(),
        });
        assert_eq!(unrelated_error.code, "product_store_error");
        assert_eq!(unrelated_error.details["kind"], "unrelated_record");
        assert_eq!(
            unrelated_error.details["reason"],
            "member_removed: unrelated record repair failed"
        );

        let malformed_routing_error = product_store_api_error(ProductStoreError::InvalidRecord {
            kind: "repository_routing",
            reason: "repository_routing_target_missingness".to_string(),
        });
        assert_eq!(malformed_routing_error.code, "product_store_error");
    }
    #[test]
    fn product_store_api_error_maps_explicit_routing_invalid_record_reasons() {
        let error = product_store_api_error(ProductStoreError::InvalidRecord {
            kind: "repository_routing",
            reason: "repository_routing_target_missing: issue selection is required".to_string(),
        });

        assert_eq!(error.code, "repository_routing_target_missing");
        assert_eq!(error.details["kind"], "repository_routing");
        assert_eq!(
            error.details["reason"],
            "repository_routing_target_missing: issue selection is required"
        );
        assert_eq!(
            error.into_response().status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn active_coding_attempt_conflict_uses_stable_http_contract() {
        let error = product_store_api_error(ProductStoreError::Conflict {
            kind: "active_coding_attempt",
            id: "coding_attempt_winner".to_string(),
        });

        assert_eq!(error.code, "coding_attempt_active");
        assert_eq!(error.details["attempt_id"], "coding_attempt_winner");
        assert_eq!(error.into_response().status(), StatusCode::CONFLICT);
    }

    #[test]
    fn product_store_api_error_fallback_includes_kind_and_id_for_identity_mismatch() {
        // IdentityMismatch 只有 kind=="coding_attempt" 被精确映射；其他 kind 命中兜底，
        // 兜底必须把 kind/id 带进 details 以便定位失败对象。
        let error = product_store_api_error(ProductStoreError::IdentityMismatch {
            kind: "runtime_binding_missing",
            id: "plan_1".to_string(),
        });

        assert_eq!(error.code, "product_store_error");
        assert_eq!(error.message, "product store operation failed");
        assert_eq!(error.details["kind"], "runtime_binding_missing");
        assert_eq!(error.details["id"], "plan_1");
    }

    #[test]
    fn product_store_api_error_fallback_includes_message_for_io() {
        // 未被精确映射的 Io/Json/PathEscape 兜底应带 message 进 details。
        let error =
            product_store_api_error(ProductStoreError::Io("remove tmp: broken pipe".to_string()));

        assert_eq!(error.code, "product_store_error");
        assert_eq!(error.details["message"], "remove tmp: broken pipe");
    }

    #[test]
    fn aggregate_root_api_error_fallback_maps_unknown_code_to_internal_error() {
        // 兜底不得把未知内部码伪装成 409 用户冲突；应显式落入 500 内部错误。
        let error = aggregate_root_api_error(
            crate::product::logical_codebase::AggregateRootPreflightError::new_for_test(
                "future_internal_code",
                "unexpected aggregate-root preflight failure",
            ),
        );

        assert_eq!(error.code, "aggregate_root_internal_error");
        assert_eq!(
            error.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn coding_workspace_exists_error_returns_stable_contract() {
        let error = coding_workspace_exists_error("plan_1", "attempt_1");

        assert_eq!(error.code, "coding_workspace_exists");
        assert_eq!(
            error.message,
            "存在 coding workspace，请先删除 coding workspace 再删除 work item group"
        );
        assert_eq!(error.details["plan_id"], "plan_1");
        assert_eq!(error.details["attempt_id"], "attempt_1");
    }

    #[test]
    fn coding_workspace_exists_for_work_item_error_returns_stable_contract() {
        let error = coding_workspace_exists_for_work_item_error("work_item_1", "attempt_1");

        assert_eq!(error.code, "coding_workspace_exists");
        assert_eq!(
            error.message,
            "存在 coding workspace，请先删除 coding workspace 再删除 work item"
        );
        assert_eq!(error.details["work_item_id"], "work_item_1");
        assert_eq!(error.details["attempt_id"], "attempt_1");
    }

    #[test]
    fn coding_workspace_api_error_maps_legacy_shared_worktree_present_to_409() {
        let error =
            coding_workspace_api_error(CodingWorkspaceEngineError::LegacySharedWorktreePresent(
                "project_0001/issue_0001".to_string(),
            ));

        assert_eq!(error.code, "legacy_shared_worktree_present");
        assert_eq!(error.into_response().status(), StatusCode::CONFLICT);
    }

    #[test]
    fn coding_workspace_api_error_maps_cross_target_blocked_stable_codes() {
        let cases = [
            ("cross_target_violation_detected", StatusCode::CONFLICT),
            (
                "cross_target_baseline_missing",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                "cross_target_store_failure",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];

        for (stable_code, expected_status) in cases {
            let error = coding_workspace_api_error(
                CodingWorkspaceEngineError::CrossTargetDeliveryBlocked(stable_code.to_string()),
            );
            assert_eq!(error.code, stable_code, "{stable_code} code");
            assert_eq!(
                error.into_response().status(),
                expected_status,
                "{stable_code} status mapping"
            );
        }
    }

    #[test]
    fn coding_workspace_api_error_maps_target_snapshot_store_io_stable_codes() {
        let cases = [
            (
                "target_snapshot_missing_for_logical",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            ("target_snapshot_identity_drifted", StatusCode::CONFLICT),
            ("target_snapshot_policy_drifted", StatusCode::CONFLICT),
            ("legacy_shared_worktree_inconsistent", StatusCode::CONFLICT),
        ];

        for (stable_code, expected_status) in cases {
            let error = coding_workspace_api_error(CodingWorkspaceEngineError::Store(
                ProductStoreError::Io(stable_code.to_string()),
            ));
            assert_eq!(error.code, stable_code, "{stable_code} code");
            assert_eq!(
                error.into_response().status(),
                expected_status,
                "{stable_code} status mapping"
            );
        }
    }

    #[test]
    fn coding_workspace_api_error_falls_back_for_unknown_engine_errors() {
        let error = coding_workspace_api_error(CodingWorkspaceEngineError::Aborted);

        assert_eq!(error.code, "coding_workspace_engine_failed");
        assert_eq!(
            error.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    // Task 11 删除调用链改造测试拆分到独立文件（large_file_guard 1200 行红线）。
    include!("support_task11_tests.rs");
}
