use super::dto::*;
use super::support::*;
use super::*;

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
            multi_repo: request.multi_repo,
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
    let store = RepositoryStore::for_project(app_paths, &project);
    let repositories = store.list(&project_id).map_err(product_store_api_error)?;
    Ok(Json(RepositoryListResponse {
        repositories: repositories.into_iter().map(repository_dto).collect(),
    }))
}

pub async fn delete_repository(
    State(state): State<WebAppState>,
    headers: axum::http::HeaderMap,
    Path((project_id, repository_id)): Path<(String, String)>,
) -> ApiResult<Json<RepositoryDeletionReceipt>> {
    let operation_id = headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::validation("idempotency_key_required", "Idempotency-Key is required")
        })?;
    let app_paths = product_app_paths(&state);
    let project = ProjectStore::new(app_paths.clone())
        .get(&project_id)
        .map_err(product_store_api_error)?;
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
    let repository_id = request
        .repository_id
        .ok_or_else(|| ApiError::validation("repository_required", "repository_id is required"))?;
    let app_paths = product_app_paths(&state);
    let _repository = find_repository(&app_paths, &project_id, &repository_id)?;
    let store = IssueStore::new(app_paths);
    let issue = store
        .create_with_repository(CreateProductIssueWithRepositoryInput {
            project_id,
            repo_id: repository_id,
            title: request.title,
            description: request.description,
            change_id: request.change_id,
        })
        .map_err(product_store_api_error)?;
    Ok(Json(product_issue_dto(issue, None)))
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
