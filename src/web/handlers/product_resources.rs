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
    let store = RepositoryStore::new(product_app_paths(&state));
    let repositories = store.list(&project_id).map_err(product_store_api_error)?;
    Ok(Json(RepositoryListResponse {
        repositories: repositories.into_iter().map(repository_dto).collect(),
    }))
}

pub async fn create_repository(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<CreateRepositoryRequest>,
) -> ApiResult<Json<RepositoryDto>> {
    let store = RepositoryStore::new(product_app_paths(&state));
    let repository = store
        .create(CreateRepositoryInput {
            project_id,
            name: request.name,
            path: request.path.into(),
            default_policy_preset: request.default_policy_preset,
            default_provider_mode: request.default_provider_mode,
        })
        .map_err(product_store_api_error)?;
    Ok(Json(repository_dto(repository)))
}

pub async fn delete_repository(
    State(state): State<WebAppState>,
    Path((project_id, repository_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let store = RepositoryStore::new(product_app_paths(&state));
    store
        .delete(&project_id, &repository_id)
        .map_err(product_store_api_error)?;
    Ok(Json(json!({"status":"deleted"})))
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
