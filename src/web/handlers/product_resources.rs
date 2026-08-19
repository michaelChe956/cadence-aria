use super::dto::*;
use super::support::*;
use super::*;
use crate::product::logical_codebase::{
    CodebaseMemberRecord, IssueCodebaseSelection, IssueCodebaseSelectionStore,
    LogicalCodebaseStore, MemberStatus,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub async fn list_workspaces(
    State(state): State<WebAppState>,
) -> ApiResult<Json<WorkspaceListResponse>> {
    let registry = WorkspaceRegistry::new(state.workspace_root.clone());
    let workspaces = registry.ensure_default_workspace()?;
    Ok(Json(WorkspaceListResponse {
        workspaces: workspaces.into_iter().map(workspace_dto).collect(),
    }))
}

pub async fn create_workspace(
    State(state): State<WebAppState>,
    Json(request): Json<CreateWorkspaceRequest>,
) -> ApiResult<Json<WorkspaceDto>> {
    let registry = WorkspaceRegistry::new(state.workspace_root.clone());
    let workspace = registry.create(CreateWorkspaceInput {
        name: request.name,
        path: request.path.into(),
        default_policy_preset: request.default_policy_preset,
        default_provider_mode: request.default_provider_mode,
    })?;
    Ok(Json(workspace_dto(workspace)))
}

pub async fn delete_workspace(
    State(state): State<WebAppState>,
    Path(workspace_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let registry = WorkspaceRegistry::new(state.workspace_root.clone());
    registry.delete(&workspace_id)?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn list_projects(
    State(state): State<WebAppState>,
) -> ApiResult<Json<ProjectListResponse>> {
    let store = ProjectStore::new(product_app_paths(&state));
    let projects = store.list().map_err(product_store_api_error)?;
    Ok(Json(ProjectListResponse {
        projects: projects.into_iter().map(project_dto).collect(),
    }))
}

pub async fn create_project(
    State(state): State<WebAppState>,
    Json(request): Json<CreateProjectRequest>,
) -> ApiResult<Json<ProjectDto>> {
    let store = ProjectStore::new(product_app_paths(&state));
    let project = store
        .create(CreateProjectInput {
            name: request.name,
            description: request.description,
        })
        .map_err(product_store_api_error)?;
    Ok(Json(project_dto(project)))
}

pub async fn get_project(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<ProjectDto>> {
    let store = ProjectStore::new(product_app_paths(&state));
    let project = store.get(&project_id).map_err(product_store_api_error)?;
    Ok(Json(project_dto(project)))
}

pub async fn open_project(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<ProjectDto>> {
    let store = ProjectStore::new(product_app_paths(&state));
    let project = store.open(&project_id).map_err(product_store_api_error)?;
    Ok(Json(project_dto(project)))
}

pub async fn delete_project(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let store = ProjectStore::new(product_app_paths(&state));
    store.delete(&project_id).map_err(product_store_api_error)?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn list_repositories(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<RepositoryListResponse>> {
    let app_paths = product_app_paths(&state);
    let project = ProjectStore::new(app_paths.clone())
        .get(&project_id)
        .map_err(product_store_api_error)?;
    let repositories = if LogicalCodebaseStore::new(app_paths.clone())
        .has_any_storage(&project_id)
        .map_err(product_store_api_error)?
    {
        multi_repo_repository_projection(&app_paths, &project_id)?
    } else {
        RepositoryStore::for_project(app_paths, &project)
            .list(&project_id)
            .map_err(product_store_api_error)?
            .into_iter()
            .map(repository_dto)
            .collect()
    };
    Ok(Json(RepositoryListResponse { repositories }))
}

/// Multi-repo 的 legacy 列表是一个只读兼容投影：成员 authority（manifest + active
/// member record）是唯一输入，绝不读取或暴露 legacy `repos.json` 写通道。
fn multi_repo_repository_projection(
    app_paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
) -> ApiResult<Vec<RepositoryDto>> {
    let authority = LogicalCodebaseStore::new(app_paths.clone());
    let Some(manifest) = authority
        .load_manifest(project_id)
        .map_err(product_store_api_error)?
    else {
        return Ok(Vec::new());
    };
    let members = authority
        .list_members(project_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .map(|member| (member.logical_repository_id, member))
        .collect::<BTreeMap<_, _>>();

    manifest
        .member_ids
        .into_iter()
        .filter_map(|member_id| members.get(&member_id))
        .filter(|member| member.status == MemberStatus::Active)
        .map(|member| repository_member_projection(&authority, project_id, member))
        .collect()
}

fn repository_member_projection(
    authority: &LogicalCodebaseStore,
    project_id: &str,
    member: &CodebaseMemberRecord,
) -> ApiResult<RepositoryDto> {
    let primary_checkout_id = member.checkout_ids.first().copied();
    let path = match primary_checkout_id {
        Some(checkout_id) => {
            authority
                .load_checkout(project_id, checkout_id)
                .map_err(product_store_api_error)?
                .ok_or_else(|| {
                    ApiError::runtime(
                        "repository_routing_inconsistent",
                        "repository routing authority is inconsistent",
                        serde_json::json!({ "checkout_id": checkout_id.0 }),
                    )
                })?
                .canonical_path
        }
        None => repository_member_path(member),
    };
    Ok(RepositoryDto {
        repository_id: member.physical_repository_id.clone(),
        project_id: project_id.to_string(),
        name: member.alias.clone(),
        path: path.to_string_lossy().into_owned(),
        repo_hash: member.source_identity.first_seen_path_hash.clone(),
        runtime_root: path.join(".aria/runtime").to_string_lossy().into_owned(),
        default_policy_preset: "manual-write".to_string(),
        default_provider_mode: "fake".to_string(),
        created_at: member.created_at.clone(),
        updated_at: member.updated_at.clone(),
    })
}

fn repository_member_path(member: &CodebaseMemberRecord) -> PathBuf {
    member
        .source_identity
        .canonical_git_dir
        .parent()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| member.source_identity.canonical_git_dir.clone())
}

pub async fn delete_repository(
    State(state): State<WebAppState>,
    headers: axum::http::HeaderMap,
    Path((project_id, repository_id)): Path<(String, String)>,
) -> ApiResult<Json<RepositoryDeletionReceipt>> {
    let app_paths = product_app_paths(&state);
    let project = ProjectStore::new(app_paths.clone())
        .get(&project_id)
        .map_err(product_store_api_error)?;
    reject_legacy_repository_endpoint_on_multi_repo(&project)?;
    let operation_id = headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::validation("idempotency_key_required", "Idempotency-Key is required")
        })?;
    RepositoryStore::for_project(app_paths, &project)
        .delete(
            &project_id,
            &repository_id,
            DeleteRepositoryCommand {
                operation_id: operation_id.to_string(),
                expected_updated_at: None,
                allow_tombstone_reactivation: false,
            },
        )
        .map(Json)
        .map_err(product_store_api_error)
}

pub async fn list_product_issues(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<ProductIssueListResponse>> {
    let store = IssueStore::new(product_app_paths(&state));
    let issues = store.list(&project_id).map_err(product_store_api_error)?;
    Ok(Json(ProductIssueListResponse {
        issues: issues
            .into_iter()
            .map(|issue| product_issue_dto_with_binding(&product_app_paths(&state), issue))
            .collect::<ApiResult<Vec<_>>>()?,
    }))
}

pub async fn create_product_issue(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<CreateProductIssueRequest>,
) -> ApiResult<Json<ProductIssueDto>> {
    let app_paths = product_app_paths(&state);
    ProjectStore::new(app_paths.clone())
        .get(&project_id)
        .map_err(product_store_api_error)?;

    let repository_id = request
        .repository_id
        .as_deref()
        .ok_or_else(|| ApiError::validation("repository_required", "repository_id is required"))?;

    match request.logical_codebase_id.as_deref() {
        Some(logical_codebase_id) => create_logical_codebase_issue(
            &state,
            &app_paths,
            &project_id,
            logical_codebase_id,
            repository_id,
            &request,
        ),
        None => {
            // 单仓路径保持 legacy create 语义，绝不写 codebase-selection.json，
            // 也绝不触碰 LC store（for_project 过渡语义已移除）。
            let _repository = find_repository(&app_paths, &project_id, repository_id)?;
            let store = IssueStore::new(app_paths);
            let issue = store
                .create_with_repository(CreateProductIssueWithRepositoryInput {
                    project_id,
                    repo_id: repository_id.to_string(),
                    logical_codebase_id: None,
                    title: request.title,
                    description: request.description,
                    change_id: request.change_id,
                })
                .map_err(product_store_api_error)?;
            Ok(Json(product_issue_dto(issue, None)))
        }
    }
}

/// 逻辑代码库 issue（v1.3）：guard LC 存在（404）→ primary 校验（须属该 LC active
/// member）→ 建 issue 并持久化归属 → 写该 LC all_members selection（键含 lc_id）。
/// D4 补偿事务：selection 写失败删除刚建 issue → 422；删除亦失败记 orphan → 500。
fn create_logical_codebase_issue(
    state: &WebAppState,
    app_paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
    repository_id: &str,
    request: &CreateProductIssueRequest,
) -> ApiResult<Json<ProductIssueDto>> {
    require_logical_codebase(app_paths, project_id, logical_codebase_id)?;
    validate_logical_codebase_primary(app_paths, project_id, logical_codebase_id, repository_id)?;

    let store = IssueStore::new(app_paths.clone());
    let issue = store
        .create_with_repository(CreateProductIssueWithRepositoryInput {
            project_id: project_id.to_string(),
            repo_id: repository_id.to_string(),
            logical_codebase_id: Some(logical_codebase_id.to_string()),
            title: request.title.clone(),
            description: request.description.clone(),
            change_id: request.change_id.clone(),
        })
        .map_err(product_store_api_error)?;

    let selection = IssueCodebaseSelection::all_members(project_id, &issue.id, None)
        .for_logical_codebase(logical_codebase_id);
    let selection_result = state.test_controls.save_issue_selection(|| {
        IssueCodebaseSelectionStore::for_lc(app_paths.clone(), logical_codebase_id).save(&selection)
    });
    if let Err(selection_error) = selection_result {
        let delete_result = state
            .test_controls
            .delete_issue(|| store.delete(project_id, &issue.id));
        return match delete_result {
            Ok(()) => Err(issue_selection_write_failed_api_error()),
            Err(delete_error) => {
                tracing::error!(
                    project_id = %project_id,
                    issue_id = %issue.id,
                    original_error = %selection_error,
                    delete_error = %delete_error,
                    "orphaned issue after codebase selection write failure"
                );
                Err(ApiError::runtime(
                    "product_store_error",
                    "product store operation failed",
                    json!({}),
                ))
            }
        };
    }

    Ok(Json(product_issue_dto(issue, None)))
}

/// 逻辑 issue 的 primary 校验：repository_id 必须来自该 LC manifest 的 active member。
/// 预校验发生在 IssueStore::create_with_repository 之前，避免留下没有 selection 的 issue。
fn validate_logical_codebase_primary(
    app_paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
    repository_id: &str,
) -> ApiResult<()> {
    // 与 routing/resolver 同一 scoping 机制（for_lc）：legacy 默认首个 LC 回退
    // project 级路径，非 legacy LC 落在 logical-codebases/{lc_id}/ 子树，保证
    // primary 校验读到的 manifest/member 与后续规划解析完全一致。
    let authority = LogicalCodebaseStore::for_lc(app_paths.clone(), logical_codebase_id);
    let manifest = authority
        .load_manifest(project_id)
        .map_err(product_store_api_error)?
        .ok_or_else(|| {
            product_store_api_error(ProductStoreError::NotFound {
                kind: "logical_codebase_manifest",
                id: logical_codebase_id.to_string(),
            })
        })?;
    let active_members = authority
        .list_members(project_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .filter(|member| {
            manifest.member_ids.contains(&member.logical_repository_id)
                && member.status == MemberStatus::Active
        })
        .collect::<Vec<_>>();
    if active_members.is_empty()
        || !active_members
            .iter()
            .any(|member| member.physical_repository_id == repository_id)
    {
        return Err(product_store_api_error(ProductStoreError::NotFound {
            kind: "repository",
            id: repository_id.to_string(),
        }));
    }
    Ok(())
}

pub async fn delete_product_issue(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let store = IssueStore::new(product_app_paths(&state));
    store
        .delete(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn list_issues(State(state): State<WebAppState>) -> ApiResult<Json<IssueListResponse>> {
    let registry = IssueRegistry::new(state.workspace_root.clone());
    let issues = registry.list()?;
    Ok(Json(IssueListResponse {
        issues: issues.into_iter().map(issue_dto).collect(),
    }))
}

pub async fn create_issue(
    State(state): State<WebAppState>,
    Json(request): Json<CreateIssueRequest>,
) -> ApiResult<Json<IssueDto>> {
    let registry = IssueRegistry::new(state.workspace_root.clone());
    let issue = registry.create(CreateIssueInput {
        title: request.title,
        description: request.description,
        change_id: request.change_id,
    })?;
    Ok(Json(issue_dto(issue)))
}

pub async fn delete_issue(
    State(state): State<WebAppState>,
    Path(issue_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let registry = IssueRegistry::new(state.workspace_root.clone());
    registry.delete(&issue_id)?;
    Ok(Json(json!({"status":"deleted"})))
}

#[cfg(test)]
pub(super) mod create_repository_tests {
    use std::path::PathBuf;

    use crate::product::models::RepositoryRecord;
    use crate::product::repository_store::{
        CadenceSkillsPreparationSummary, RepositoryInitializationCommandSummary,
        RepositoryInitializationSummary, RepositoryRegistrationSuccess,
    };

    pub(crate) fn registration_success() -> RepositoryRegistrationSuccess {
        RepositoryRegistrationSuccess {
            repository: RepositoryRecord {
                id: "repository_0001".to_string(),
                project_id: "project_0001".to_string(),
                name: "Aria".to_string(),
                path: PathBuf::from("/work/aria"),
                repo_hash: "repo-hash".to_string(),
                runtime_root: PathBuf::from("/work/aria/.aria"),
                default_policy_preset: "balanced".to_string(),
                default_provider_mode: "claude_code".to_string(),
                created_at: "2026-07-14T00:00:00Z".to_string(),
                updated_at: "2026-07-14T00:00:00Z".to_string(),
                logical_repository_id: None,
                primary_checkout_id: None,
                identity_schema_version: 0,
            },
            cadence_skills: CadenceSkillsPreparationSummary {
                source_mode: "offline".to_string(),
                source_root: PathBuf::from("/skills/source"),
                skills_root: PathBuf::from("/skills"),
                git_updated: false,
                link_sync_status: "synchronized".to_string(),
                warnings: Vec::new(),
            },
            initialization: RepositoryInitializationSummary {
                provider: "claude_code".to_string(),
                source: PathBuf::from("/skills/source"),
                source_mode: "offline".to_string(),
                skills_root: PathBuf::from("/skills"),
                git_updated: false,
                link_sync_status: "synchronized".to_string(),
                commands: vec![RepositoryInitializationCommandSummary {
                    command_index: 1,
                    command: "/pre-check".to_string(),
                    status: "completed".to_string(),
                    output_summary: None,
                }],
            },
            warnings: Vec::new(),
            changed_paths: Vec::new(),
            git_finalize_warning: None,
            completed_at: "2026-07-14T00:01:00Z".to_string(),
        }
    }
}
