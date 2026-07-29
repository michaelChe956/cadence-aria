use super::*;

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
    RepositoryStore::new(app_paths.clone())
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
pub(crate) fn product_store_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::NotFound {
            kind: "project", ..
        } => ApiError::runtime("project_not_found", "project not found", json!({})),
        ProductStoreError::NotFound {
            kind: "repository", ..
        } => ApiError::runtime("repository_not_found", "repository not found", json!({})),
        ProductStoreError::NotFound {
            kind: "repository_initialization_operation",
            ..
        } => ApiError::runtime(
            "repository_initialization_operation_not_found",
            "repository initialization operation not found",
            json!({}),
        ),
        ProductStoreError::NotFound { kind: "issue", .. } => {
            ApiError::runtime("issue_not_found", "issue not found", json!({}))
        }
        ProductStoreError::NotFound {
            kind: "work_item", ..
        } => ApiError::runtime("work_item_not_found", "work item not found", json!({})),
        ProductStoreError::NotFound {
            kind: "coding_attempt",
            ..
        } => ApiError::runtime(
            "coding_attempt_not_found",
            "coding attempt not found",
            json!({}),
        ),
        ProductStoreError::NotFound {
            kind: "workspace_session",
            ..
        } => ApiError::runtime(
            "workspace_session_not_found",
            "workspace session not found",
            json!({}),
        ),
        ProductStoreError::NotFound { kind: "gate", .. } => {
            ApiError::runtime("gate_not_found", "gate not found", json!({}))
        }
        ProductStoreError::Io(message) if message == "workspace_session_ambiguous" => {
            ApiError::runtime(
                "workspace_session_ambiguous",
                "workspace session matches multiple files",
                json!({}),
            )
        }
        ProductStoreError::Io(message) if message == "gate_ambiguous" => ApiError::runtime(
            "gate_ambiguous",
            "gate matches multiple projects",
            json!({}),
        ),
        ProductStoreError::Ambiguous {
            kind: "coding_attempt",
            id,
        } => ApiError::runtime(
            "coding_attempt_ambiguous",
            "coding attempt matches multiple issues",
            json!({"attempt_id": id}),
        ),
        ProductStoreError::Conflict {
            kind: "active_coding_attempt",
            id,
        } => ApiError::runtime(
            "coding_attempt_active",
            "an active coding attempt already exists for this work item",
            json!({"attempt_id": id}),
        ),
        ProductStoreError::IdentityMismatch {
            kind: "coding_attempt",
            id,
        } => ApiError::runtime(
            "coding_attempt_scope_mismatch",
            "coding attempt does not belong to the requested project and issue",
            json!({"attempt_id": id}),
        ),
        ProductStoreError::PathEscape(_) => {
            ApiError::validation("invalid_project_id", "invalid project id")
        }
        other => {
            let details = match &other {
                ProductStoreError::NotFound { kind, id }
                | ProductStoreError::Ambiguous { kind, id }
                | ProductStoreError::Conflict { kind, id }
                | ProductStoreError::IdentityMismatch { kind, id } => {
                    json!({ "kind": kind, "id": id })
                }
                ProductStoreError::Io(message)
                | ProductStoreError::Json(message)
                | ProductStoreError::PathEscape(message) => json!({ "message": message }),
            };
            ApiError::runtime(
                "product_store_error",
                "product store operation failed",
                details,
            )
        }
    }
}

/// 当 work item group 存在 coding workspace 时拒绝删除，提示先删除 coding workspace。
#[allow(dead_code)] // 由 Task 3 起的 deletion.rs 删除门禁消费
pub(crate) fn coding_workspace_exists_error(plan_id: &str, attempt_id: &str) -> ApiError {
    ApiError::runtime(
        "coding_workspace_exists",
        "存在 coding workspace，请先删除 coding workspace 再删除 work item group",
        json!({ "plan_id": plan_id, "attempt_id": attempt_id }),
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
    if error_message.contains("shared_worktree_dirty_manual_gate") {
        return ApiError::runtime(
            "shared_worktree_dirty_manual_gate",
            "shared worktree has uncommitted changes; manual cleanup required",
            json!({"details": error_message}),
        );
    }
    ApiError::runtime(
        "coding_workspace_engine_failed",
        "coding workspace engine operation failed",
        json!({"details": error_message}),
    )
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
}
