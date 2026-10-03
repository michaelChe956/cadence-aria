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
    /// P3（REQ-WIGA-07）：RFC3339；不传则不请求近期完成目录。
    pub recent_since: Option<String>,
    /// P3（REQ-WIGA-07）：默认 32；仅允许 1..=32，孤立 limit（缺 since）报 422。
    pub recent_limit: Option<usize>,
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
    // v1.3：单仓代码库路径不再按 project 判定 feature（for_project 过渡语义已移除），
    // 直接以禁用逻辑身份的 RepositoryStore 读 repos.json，绝不触碰 LC store。
    let _project = ProjectStore::new(app_paths.clone())
        .get(project_id)
        .map_err(product_store_api_error)?;
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

/// C5 Task 2（REQ-WIGA-01/REQ-ROUTE-C5-TARGET）：自动化载体判定结果。
/// GET `/automation-target` 投影与 PUT Enable 共用同一判定入口，防止
/// 两处内联分叉（Review Focus 5）。
pub(crate) enum AutomationCarrierResolution {
    /// 单仓 issue：target 携带真实 `RepositoryRecord.id`，绝不伪造
    /// logical 替身身份。
    SingleRepository {
        target: crate::product::logical_codebase::EnrollmentTarget,
    },
    /// LC issue：C4 authority resolver 的完整解析结果（manifest/selection
    /// 由调用方按既有 LC 约束消费）。
    LogicalCodebase {
        resolution: Box<crate::product::logical_codebase::RepositoryAuthorityResolution>,
    },
}

/// C5 Task 2：唯一载体判定入口。先经 C4 `resolve_for_issue`——
/// `Ok(Some)` → LC 分支透传 resolution；`Ok(None)` → 单仓分支：
/// `issue.repo_id` 缺失→422（文案指明缺仓库身份）；经 `find_repository`
/// 取真实 `RepositoryRecord` 后，再走 C4 显式 single-repo authority
/// 检查（canonical git root 别名冲突、legacy active member 来源冲突均
/// 在 `resolve_single_repo` 内 fail-closed）；`Err` 沿既有
/// `repository_routing_*` HTTP 409/4xx 映射透传。只读，不写任何 store。
pub(crate) fn resolve_automation_carrier(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue: &crate::product::models::IssueRecord,
) -> ApiResult<AutomationCarrierResolution> {
    let resolver =
        crate::product::logical_codebase::RepositoryAuthorityResolver::new(app_paths.clone());
    match resolver
        .resolve_for_issue(project_id, &issue.id)
        .map_err(product_store_api_error)?
    {
        Some(resolution) => Ok(AutomationCarrierResolution::LogicalCodebase {
            resolution: Box::new(resolution),
        }),
        None => {
            let repository_id = issue.repo_id.clone().ok_or_else(|| {
                super::automation_enrollment::invalid_scope(
                    "automation carrier requires a repository identity: the issue has no repo_id; \
                     assign the issue to a registered repository first",
                )
            })?;
            // 未登记仓：422 指明缺失仓库身份（不猜、不回退）。
            let record = match find_repository(app_paths, project_id, &repository_id) {
                Ok(record) => record,
                Err(error) => {
                    return Err(super::automation_enrollment::invalid_scope(format!(
                        "automation carrier requires a registered repository: repo_id \
                         {repository_id} has no repository record ({})",
                        error.message
                    )));
                }
            };
            // C4 显式 single-repo authority/conflict 检查：同 git 根别名与
            // legacy active member 来源冲突均在此 fail-closed（409 透传）。
            resolver
                .resolve(crate::product::logical_codebase::RepositoryRoutingRequest {
                    project_id: project_id.to_string(),
                    issue_id: Some(issue.id.clone()),
                    kind: crate::product::logical_codebase::RepositoryTargetKind::SingleRepo,
                    repository_id: Some(repository_id),
                    logical_codebase_id: None,
                    logical_repository_id: None,
                    checkout_id: None,
                })
                .map_err(product_store_api_error)?;
            Ok(AutomationCarrierResolution::SingleRepository {
                target: crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
                    repository_id: record.id,
                },
            })
        }
    }
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

/// v1.3 guard: the request path names a logical codebase; an unknown (or
/// single-repo-only project) id is a plain 404 `logical_codebase_not_found`.
pub(crate) fn require_logical_codebase(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
) -> ApiResult<crate::product::logical_codebase::LogicalCodebaseRecord> {
    ProjectStore::new(paths.clone())
        .get(project_id)
        .map_err(product_store_api_error)?;
    LogicalCodebaseStore::new(paths.clone())
        .get(project_id, logical_codebase_id)
        .map_err(product_store_api_error)?
        .ok_or_else(|| {
            ApiError::runtime(
                "logical_codebase_not_found",
                "logical codebase not found",
                json!({
                    "project_id": project_id,
                    "logical_codebase_id": logical_codebase_id,
                }),
            )
        })
}

/// Legacy `/logical-codebase/*` endpoints alias the project's default first
/// logical codebase. The alias is pinned to the v1.2 migration logical
/// codebase whenever its record exists (its id is deterministic), so newly
/// created logical codebases whose uuid ids sort lexicographically earlier
/// cannot steal the alias from old clients. Otherwise it falls back to the
/// deterministic first record by id order. With no logical codebase at all
/// the alias has no referent and returns the same 404.
pub(crate) fn default_logical_codebase_id(
    paths: &ProductAppPaths,
    project_id: &str,
) -> ApiResult<String> {
    ProjectStore::new(paths.clone())
        .get(project_id)
        .map_err(product_store_api_error)?;
    let store = LogicalCodebaseStore::new(paths.clone());
    let legacy_id = crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id);
    if store
        .get(project_id, &legacy_id)
        .map_err(product_store_api_error)?
        .is_some()
    {
        return Ok(legacy_id);
    }
    store
        .list(project_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .next()
        .map(|record| record.id)
        .ok_or_else(|| {
            ApiError::runtime(
                "logical_codebase_not_found",
                "logical codebase not found",
                json!({ "project_id": project_id }),
            )
        })
}

/// C4 Task 2：canonical LC 路由的统一身份校验——经唯一 authority resolver
/// 冻结 LC authority（kind/重复来源/legacy 布局冲突 fail-closed），未知 LC
/// 保持与 `require_logical_codebase` 相同的 `logical_codebase_not_found` 404。
pub(crate) fn resolve_lc_authority(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
) -> ApiResult<crate::product::logical_codebase::RepositoryAuthorityResolution> {
    crate::product::logical_codebase::RepositoryAuthorityResolver::new(paths.clone())
        .resolve(crate::product::logical_codebase::RepositoryRoutingRequest {
            project_id: project_id.to_string(),
            issue_id: None,
            kind: crate::product::logical_codebase::RepositoryTargetKind::LogicalCodebase,
            repository_id: None,
            logical_codebase_id: Some(logical_codebase_id.to_string()),
            logical_repository_id: None,
            checkout_id: None,
        })
        .map_err(|error| match error {
            crate::product::json_store::ProductStoreError::NotFound {
                kind: "logical_codebase",
                ..
            } => ApiError::runtime(
                "logical_codebase_not_found",
                "logical codebase not found",
                json!({
                    "project_id": project_id,
                    "logical_codebase_id": logical_codebase_id,
                }),
            ),
            other => product_store_api_error(other),
        })
}

pub(crate) fn provider_workspace_config(
    author_provider: Option<&str>,
    reviewer_provider: Option<&str>,
    review_rounds: Option<u32>,
    superpowers_enabled: Option<bool>,
    openspec_enabled: Option<bool>,
    test_provider_enabled: bool,
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
        None => {
            resolve_default_coding_provider("codex", test_provider_enabled, provider_availability)?
        }
    };
    let reviewer = match reviewer_provider {
        Some(provider) => resolve_explicit_provider_name(provider, provider_availability)?,
        None => resolve_default_coding_provider(
            "claude_code",
            test_provider_enabled,
            provider_availability,
        )?,
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
// coding attempt 删除/清理链（abort→cleanup→purge→finalize）拆分到
// support_parts/coding_cleanup.inc.rs（large_file_guard 1200 行红线，纯移动）。
include!("support_parts/coding_cleanup.inc.rs");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn availability(provider: &ProviderName) -> bool {
        matches!(provider, ProviderName::ClaudeCode)
    }

    #[test]
    fn provider_workspace_config_rejects_explicit_unavailable_provider() {
        let error =
            provider_workspace_config(Some("codex"), None, None, None, None, false, &availability)
                .expect_err("explicit unavailable provider must fail");

        assert_eq!(error.code, "provider_unavailable");
        assert_eq!(error.details["provider"], "codex");
    }

    #[test]
    fn provider_workspace_config_records_default_fallback_status() {
        let config = provider_workspace_config(None, None, None, None, None, false, &availability)
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
    fn finalize_deletion_cleans_miswritten_legacy_issue_layout_for_snapshot_attempts() {
        use crate::product::coding_attempt_store::CreateCodingAttemptInput;
        use crate::product::coding_models::AttemptTargetSnapshot;
        use crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput;
        use crate::product::logical_codebase::RepositoryCheckoutId;
        use crate::web::workspace_ws_types::ProviderConfigSnapshot;

        // 缺陷 #13 层2 存量出口：组入口路由分流修复前，Logical 路由的 group
        // create 误写 issue 维 legacy 布局（与 preflight 契约相反）。删除该
        // issue 最后一个 attempt 时必须条件清掉该文件，恢复迁移契约一致性。
        let root = tempfile::tempdir().expect("root");
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let coding_store = CodingAttemptStore::new(app_paths.clone());
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let attempt = coding_store
            .create_attempt(CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/issues/issue_0001".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 2,
            })
            .expect("attempt");
        // 落盘带 target_snapshot 的形态（Logical 路由 attempt；照
        // group_review_identity_snapshot 测试的 with_target_snapshot 模式直写）。
        let mut logical = attempt.clone();
        logical.target_snapshot = Some(AttemptTargetSnapshot {
            logical_repository_id: LogicalRepositoryId(uuid::Uuid::nil()),
            checkout_id: RepositoryCheckoutId(uuid::Uuid::nil()),
            physical_repository_id: "repository_0001".to_string(),
            canonical_path: root.path().join("repo"),
            git_dir_identity: "git-dir-identity".to_string(),
            revision: None,
            policy_digest: String::new(),
            membership_revision: 1,
            captured_at: "2026-10-02T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        });
        let attempt_path = app_paths
            .issue_lifecycle_root(&logical.project_id, &logical.issue_id)
            .join("coding-attempts")
            .join(format!("{}.json", logical.id));
        crate::product::json_store::write_json(&attempt_path, &logical)
            .expect("write attempt with target snapshot");
        // 模拟分流修复前误写的 issue 维 legacy 布局。
        lifecycle
            .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
                project_id: logical.project_id.clone(),
                issue_id: logical.issue_id.clone(),
                repository_id: "repository_0001".to_string(),
                branch_name: "aria/issues/issue_0001".to_string(),
                worktree_path: root.path().join("wt"),
                base_branch: "main".to_string(),
            })
            .expect("seed miswritten legacy layout");

        finalize_coding_attempt_deletion(&coding_store, &app_paths, &logical)
            .expect("finalize deletion");

        assert!(
            !app_paths
                .issue_root(&logical.project_id, &logical.issue_id)
                .join("issue-shared-worktree.json")
                .exists(),
            "miswritten legacy issue layout must be cleaned once the last attempt is deleted"
        );
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
