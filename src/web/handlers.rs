use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use std::fs;
use std::path::{Path as StdPath, PathBuf};
use tokio_stream::wrappers::BroadcastStream;

use crate::cross_cutting::provider_adapter::{
    FakeProviderAdapter, STRUCTURED_OUTPUT_END, STRUCTURED_OUTPUT_START,
};
use crate::interactive::models::WebWorkspaceProjection;
use crate::product::app_paths::ProductAppPaths;
use crate::product::gate_store::GateStore;
use crate::product::issue_store::{
    CreateProductIssueWithRepositoryInput, IssueStore, StartProductIssueInput,
};
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::lifecycle_store::{
    CreateDesignSpecInput, CreateStorySpecInput, CreateWorkItemInput, CreateWorkspaceSessionInput,
    LifecycleStore,
};
use crate::product::models::{
    DesignKind, DesignSpecRecord, GateStatus, IssuePhase as ProductIssuePhase,
    IssueRecord as ProductIssueRecord, IssueRuntimeBindingRecord,
    IssueStatus as ProductIssueStatus, LifecycleConfirmationStatus, LifecycleWorkItemRecord,
    ProjectRecord, ProviderName, RepositoryRecord, StorySpecRecord, WorkItemPlanStatus,
    WorkItemStatus, WorkspaceMessageRecord, WorkspaceSessionRecord, WorkspaceSessionStatus,
    WorkspaceType,
};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::provider_workspace_runner::{
    ProviderWorkspaceRunner, WorkspaceProviderRunInput,
};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::product::runtime_binding_store::{CreateRuntimeBindingInput, RuntimeBindingStore};
use crate::web::error::{ApiError, ApiResult};
use crate::web::events::WebEventType;
use crate::web::issue_registry::{CreateIssueInput, IssueRecord, IssueRegistry, IssueStatus};
use crate::web::redaction::redact_sensitive_lines;
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;
use crate::web::types::{
    AdvanceTaskResponse, ArtifactContentResponse, ConfirmTaskRequest, ConfirmTaskResponse,
    CreateIssueRequest, CreateProductIssueRequest, CreateProjectRequest, CreateRepositoryRequest,
    CreateTaskRequest, CreateTaskResponse, CreateWorkspaceRequest, DesignSpecDto,
    FileContentResponse, FileDiffResponse, GenerateDesignSpecsRequest, GenerateDesignSpecsResponse,
    GenerateStorySpecsRequest, GenerateStorySpecsResponse, GenerateWorkItemsRequest,
    GenerateWorkItemsResponse, IssueDto, IssueLifecycleResponse, IssueListResponse,
    IssueRollbackPreviewRequest, IssueRollbackRequest, LifecycleWorkItemDto,
    ProductIssueArtifactDto, ProductIssueDto, ProductIssueListResponse, ProjectDto,
    ProjectListResponse, ProviderInputContentResponse, RepositoryDto, RepositoryListResponse,
    ResolveGateRequest, ResolveGateResponse, RollbackPreviewRequest, RollbackPreviewResponse,
    RollbackRequest, RollbackResponse, StartIssueRequest, StartIssueResponse,
    StartProductIssueRequest, StartProductIssueResponse, StopTaskResponse, StorySpecDto,
    TaskListResponse, WebEvent, WorkspaceDto, WorkspaceListResponse, WorkspaceMessageDto,
    WorkspaceSessionConfirmRequest, WorkspaceSessionDto, WorkspaceSessionMessageRequest,
    WorkspaceSessionRunNextRequest,
};
use crate::web::workspace_registry::{CreateWorkspaceInput, WorkspaceRecord, WorkspaceRegistry};

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

#[derive(Debug, Clone)]
struct ProviderWorkspaceConfig {
    author_provider: ProviderName,
    reviewer_provider: ProviderName,
    review_rounds: u32,
    superpowers_enabled: bool,
    openspec_enabled: bool,
}

pub async fn health() -> Json<serde_json::Value> {
    Json(json!({"status":"ok"}))
}

pub async fn create_task(
    State(state): State<WebAppState>,
    Json(request): Json<CreateTaskRequest>,
) -> ApiResult<Json<CreateTaskResponse>> {
    let mut runtime = state.runtime.lock().expect("runtime lock");
    let response = runtime.create_task(request)?;
    state.events.publish(
        WebEventType::ProjectionUpdated.as_str(),
        Some(&response.task_id),
        json!({"phase": response.phase}),
    );
    Ok(Json(response))
}

pub async fn list_tasks(State(state): State<WebAppState>) -> ApiResult<Json<TaskListResponse>> {
    let runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.list_tasks()?))
}

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

pub async fn issue_lifecycle(
    State(state): State<WebAppState>,
    Path(issue_id): Path<String>,
    Query(query): Query<GateResolveQuery>,
) -> ApiResult<Json<IssueLifecycleResponse>> {
    let project_id = query
        .project_id
        .ok_or_else(|| ApiError::validation("project_required", "project_id is required"))?;
    let app_paths = product_app_paths(&state);
    let issue = IssueStore::new(app_paths.clone())
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story_specs = lifecycle
        .list_story_specs(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .map(story_spec_dto)
        .collect();
    let design_specs = lifecycle
        .list_design_specs(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .map(design_spec_dto)
        .collect();
    let work_items = lifecycle
        .list_work_items(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .map(lifecycle_work_item_dto)
        .collect();
    let workspace_sessions = lifecycle
        .list_workspace_sessions(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .map(workspace_session_dto)
        .collect();

    Ok(Json(IssueLifecycleResponse {
        issue: product_issue_dto_with_binding(&app_paths, issue)?,
        story_specs,
        design_specs,
        work_items,
        workspace_sessions,
    }))
}

pub async fn generate_story_specs(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<GenerateStorySpecsRequest>,
) -> ApiResult<Json<GenerateStorySpecsResponse>> {
    let workspace_config = provider_workspace_config(
        request.author_provider.as_deref(),
        request.reviewer_provider.as_deref(),
        request.review_rounds,
        request.superpowers_enabled,
        request.openspec_enabled,
    )?;
    let app_paths = product_app_paths(&state);
    let issue = IssueStore::new(app_paths.clone())
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let repository_id = issue
        .repo_id
        .clone()
        .ok_or_else(|| ApiError::validation("repository_required", "repository_id is required"))?;
    find_repository(&app_paths, &project_id, &repository_id)?;
    let lifecycle = LifecycleStore::new(app_paths);
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            repository_id,
            title: request.title,
        })
        .map_err(product_store_api_error)?;
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id,
            issue_id,
            entity_id: story.id.clone(),
            workspace_type: WorkspaceType::Story,
            author_provider: workspace_config.author_provider,
            reviewer_provider: workspace_config.reviewer_provider,
            review_rounds: workspace_config.review_rounds,
            superpowers_enabled: workspace_config.superpowers_enabled,
            openspec_enabled: workspace_config.openspec_enabled,
        })
        .map_err(product_store_api_error)?;

    Ok(Json(GenerateStorySpecsResponse {
        story_specs: vec![story_spec_dto(story)],
        workspace_session: workspace_session_dto(session),
    }))
}

pub async fn generate_design_specs(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<GenerateDesignSpecsRequest>,
) -> ApiResult<Json<GenerateDesignSpecsResponse>> {
    let workspace_config = provider_workspace_config(
        request.author_provider.as_deref(),
        request.reviewer_provider.as_deref(),
        request.review_rounds,
        request.superpowers_enabled,
        request.openspec_enabled,
    )?;
    let app_paths = product_app_paths(&state);
    IssueStore::new(app_paths.clone())
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let lifecycle = LifecycleStore::new(app_paths);
    validate_confirmed_story_specs(&lifecycle, &project_id, &issue_id, &request.story_spec_ids)?;
    let design_kind = parse_design_kind(&request.design_kind)?;
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            story_spec_ids: request.story_spec_ids,
            design_kind,
            title: request.title,
        })
        .map_err(product_store_api_error)?;
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id,
            issue_id,
            entity_id: design.id.clone(),
            workspace_type: WorkspaceType::Design,
            author_provider: workspace_config.author_provider,
            reviewer_provider: workspace_config.reviewer_provider,
            review_rounds: workspace_config.review_rounds,
            superpowers_enabled: workspace_config.superpowers_enabled,
            openspec_enabled: workspace_config.openspec_enabled,
        })
        .map_err(product_store_api_error)?;

    Ok(Json(GenerateDesignSpecsResponse {
        design_specs: vec![design_spec_dto(design)],
        workspace_session: workspace_session_dto(session),
    }))
}

pub async fn generate_work_items(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<GenerateWorkItemsRequest>,
) -> ApiResult<Json<GenerateWorkItemsResponse>> {
    let workspace_config = provider_workspace_config(
        request.author_provider.as_deref(),
        request.reviewer_provider.as_deref(),
        request.review_rounds,
        request.superpowers_enabled,
        request.openspec_enabled,
    )?;
    let app_paths = product_app_paths(&state);
    let issue = IssueStore::new(app_paths.clone())
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let repository_id = issue
        .repo_id
        .clone()
        .ok_or_else(|| ApiError::validation("repository_required", "repository_id is required"))?;
    find_repository(&app_paths, &project_id, &repository_id)?;
    let lifecycle = LifecycleStore::new(app_paths);
    validate_confirmed_story_specs(&lifecycle, &project_id, &issue_id, &request.story_spec_ids)?;
    validate_confirmed_design_specs(&lifecycle, &project_id, &issue_id, &request.design_spec_ids)?;
    let work_item = lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            repository_id,
            story_spec_ids: request.story_spec_ids,
            design_spec_ids: request.design_spec_ids,
            title: request.title,
        })
        .map_err(product_store_api_error)?;
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id,
            issue_id,
            entity_id: work_item.id.clone(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: workspace_config.author_provider,
            reviewer_provider: workspace_config.reviewer_provider,
            review_rounds: workspace_config.review_rounds,
            superpowers_enabled: workspace_config.superpowers_enabled,
            openspec_enabled: workspace_config.openspec_enabled,
        })
        .map_err(product_store_api_error)?;

    Ok(Json(GenerateWorkItemsResponse {
        work_items: vec![lifecycle_work_item_dto(work_item)],
        workspace_session: workspace_session_dto(session),
    }))
}

pub async fn workspace_session_message(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WorkspaceSessionMessageRequest>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    validate_workspace_message(&request)?;
    let session = LifecycleStore::new(product_app_paths(&state))
        .append_workspace_message(&session_id, request.role, request.content)
        .map_err(product_store_api_error)?;
    Ok(Json(workspace_session_dto(session)))
}

pub async fn workspace_session_run_next(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WorkspaceSessionRunNextRequest>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    let paths = product_app_paths(&state);
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .get_workspace_session(&session_id)
        .map_err(product_store_api_error)?;
    let user_prompt = workspace_user_prompt(request.user_prompt);
    lifecycle
        .append_workspace_message(&session_id, "user".to_string(), user_prompt.clone())
        .map_err(product_store_api_error)?;

    let runner = ProviderWorkspaceRunner::new(paths);
    let output = runner
        .run_next(
            WorkspaceProviderRunInput {
                session_id,
                user_prompt: provider_workspace_prompt(user_prompt),
            },
            &FakeProviderAdapter,
        )
        .map_err(|error| {
            ApiError::runtime(
                "provider_workspace_run_failed",
                "provider workspace run failed",
                json!({"details": error.details}),
            )
        })?;
    Ok(Json(workspace_session_dto(output.session)))
}

pub async fn workspace_session_confirm(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WorkspaceSessionConfirmRequest>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    let lifecycle = LifecycleStore::new(product_app_paths(&state));
    let session = lifecycle
        .update_workspace_session_status(&session_id, WorkspaceSessionStatus::Confirmed)
        .map_err(product_store_api_error)?;
    confirm_workspace_entity(&lifecycle, &session)?;
    let session = lifecycle
        .append_workspace_message(
            &session_id,
            "system".to_string(),
            format!("已由 {} 确认当前 Workspace 产物。", request.confirmed_by),
        )
        .map_err(product_store_api_error)?;
    Ok(Json(workspace_session_dto(session)))
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

pub async fn start_product_issue(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<StartProductIssueRequest>,
) -> ApiResult<Json<StartProductIssueResponse>> {
    let app_paths = product_app_paths(&state);
    let issue_store = IssueStore::new(app_paths.clone());
    let issue = issue_store
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    if let Some(binding) = active_binding_for_issue(&app_paths, &project_id, &issue)? {
        let workspace_id = product_execution_workspace_id(&project_id, &binding.repo_id);
        return Ok(Json(StartProductIssueResponse {
            issue_id: issue.id,
            project_id,
            repository_id: binding.repo_id,
            workspace_id,
            task_id: binding.task_id.unwrap_or_default(),
            session_id: binding.session_id.unwrap_or_default(),
            status: product_issue_status_text(&issue.status).to_string(),
        }));
    }
    let workspace_repository_id = request
        .workspace_id
        .as_deref()
        .or(request.repository_id.as_deref())
        .ok_or_else(|| ApiError::validation("workspace_required", "workspace_id is required"))?;
    let repository = find_repository(&app_paths, &project_id, workspace_repository_id)?;

    let mut runtime = WebRuntime::new_fake(repository.path.clone());
    let created = runtime.create_task(CreateTaskRequest {
        request_text: issue
            .description
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| issue.title.clone()),
        change_id: issue.change_id.clone(),
        policy_preset: request
            .policy_preset
            .unwrap_or_else(|| repository.default_policy_preset.clone()),
        provider_mode: request
            .provider_mode
            .unwrap_or_else(|| repository.default_provider_mode.clone()),
        timeout_secs: request.timeout_secs.unwrap_or(2400),
    })?;

    let binding = RuntimeBindingStore::new(app_paths.clone())
        .create(CreateRuntimeBindingInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            repo_id: repository.id.clone(),
            change_id: issue.change_id,
            task_id: Some(created.task_id.clone()),
            session_id: Some(created.session_id.clone()),
            runtime_root: repository.runtime_root.clone(),
        })
        .map_err(product_store_api_error)?;
    let started = issue_store
        .start(StartProductIssueInput {
            project_id: project_id.clone(),
            issue_id,
            repo_id: repository.id.clone(),
            active_binding_id: binding.id,
        })
        .map_err(product_store_api_error)?;
    let workspace_id = product_execution_workspace_id(&project_id, &repository.id);
    state.events.publish(
        WebEventType::ProjectionUpdated.as_str(),
        Some(&created.task_id),
        json!({
            "project_id": project_id,
            "issue_id": started.id,
            "repository_id": repository.id,
            "workspace_id": workspace_id,
            "phase": created.phase
        }),
    );
    Ok(Json(StartProductIssueResponse {
        issue_id: started.id,
        project_id,
        repository_id: repository.id,
        workspace_id,
        task_id: created.task_id,
        session_id: created.session_id,
        status: product_issue_status_text(&started.status).to_string(),
    }))
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

pub async fn start_issue(
    State(state): State<WebAppState>,
    Path(issue_id): Path<String>,
    Json(request): Json<StartIssueRequest>,
) -> ApiResult<Json<StartIssueResponse>> {
    let issue_registry = IssueRegistry::new(state.workspace_root.clone());
    let workspace_registry = WorkspaceRegistry::new(state.workspace_root.clone());
    let issue = issue_registry.get(&issue_id)?;
    if let (Some(workspace_id), Some(task_id), Some(session_id)) = (
        issue.workspace_id.clone(),
        issue.task_id.clone(),
        issue.session_id.clone(),
    ) {
        return Ok(Json(StartIssueResponse {
            issue_id,
            workspace_id,
            task_id,
            session_id,
            status: "started".to_string(),
        }));
    }
    let workspace = workspace_registry.get(&request.workspace_id)?;
    let mut runtime = WebRuntime::new_fake(workspace.path.clone());
    let created = runtime.create_task(CreateTaskRequest {
        request_text: issue
            .description
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| issue.title.clone()),
        change_id: issue.change_id.clone(),
        policy_preset: request
            .policy_preset
            .unwrap_or_else(|| workspace.default_policy_preset.clone()),
        provider_mode: request
            .provider_mode
            .unwrap_or_else(|| workspace.default_provider_mode.clone()),
        timeout_secs: request.timeout_secs.unwrap_or(2400),
    })?;
    let started = issue_registry.mark_started(
        &issue_id,
        &workspace.workspace_id,
        &created.task_id,
        &created.session_id,
    )?;
    state.events.publish(
        WebEventType::ProjectionUpdated.as_str(),
        Some(&created.task_id),
        json!({
            "issue_id": started.issue_id,
            "workspace_id": workspace.workspace_id,
            "phase": created.phase
        }),
    );
    Ok(Json(StartIssueResponse {
        issue_id: started.issue_id,
        workspace_id: workspace.workspace_id,
        task_id: created.task_id,
        session_id: created.session_id,
        status: issue_status_text(&started.status).to_string(),
    }))
}

pub async fn confirm_gate(
    State(state): State<WebAppState>,
    Path((issue_id, gate_id)): Path<(String, String)>,
    Query(query): Query<GateResolveQuery>,
    Json(request): Json<ResolveGateRequest>,
) -> ApiResult<Json<ResolveGateResponse>> {
    resolve_gate(
        &state,
        issue_id,
        gate_id,
        query.project_id,
        GateStatus::Confirmed,
        "confirmed",
        request,
    )
}

pub async fn request_gate_change(
    State(state): State<WebAppState>,
    Path((issue_id, gate_id)): Path<(String, String)>,
    Query(query): Query<GateResolveQuery>,
    Json(request): Json<ResolveGateRequest>,
) -> ApiResult<Json<ResolveGateResponse>> {
    resolve_gate(
        &state,
        issue_id,
        gate_id,
        query.project_id,
        GateStatus::ChangeRequested,
        "change_requested",
        request,
    )
}

pub async fn terminate_gate(
    State(state): State<WebAppState>,
    Path((issue_id, gate_id)): Path<(String, String)>,
    Query(query): Query<GateResolveQuery>,
    Json(request): Json<ResolveGateRequest>,
) -> ApiResult<Json<ResolveGateResponse>> {
    resolve_gate(
        &state,
        issue_id,
        gate_id,
        query.project_id,
        GateStatus::Terminated,
        "terminated",
        request,
    )
}

pub async fn advance_task(
    State(state): State<WebAppState>,
    Path(task_id): Path<String>,
    Query(query): Query<WorkspaceQuery>,
) -> ApiResult<Json<AdvanceTaskResponse>> {
    if let Some(workspace_id) = query.workspace_id.as_deref() {
        let workspace_root =
            resolve_workspace_root(&state.workspace_root, Some(workspace_id), Some(&task_id))?;
        let mut runtime = WebRuntime::new_fake(workspace_root);
        let response = runtime.advance_task(&task_id)?;
        if let AdvanceTaskResponse::PausedForApproval { pending_step } = &response {
            state.events.publish(
                WebEventType::CheckpointCreated.as_str(),
                Some(&task_id),
                json!({"checkpoint_id": pending_step.checkpoint_id, "workspace_id": workspace_id}),
            );
            state.events.publish(
                WebEventType::PausedForApproval.as_str(),
                Some(&task_id),
                json!({"node_id": pending_step.node_id, "workspace_id": workspace_id}),
            );
        }
        state.events.publish(
            WebEventType::ProjectionUpdated.as_str(),
            Some(&task_id),
            json!({"workspace_id": workspace_id}),
        );
        return Ok(Json(response));
    }
    let mut runtime = state.runtime.lock().expect("runtime lock");
    let response = runtime.advance_task(&task_id)?;
    if let AdvanceTaskResponse::PausedForApproval { pending_step } = &response {
        state.events.publish(
            WebEventType::CheckpointCreated.as_str(),
            Some(&task_id),
            json!({"checkpoint_id": pending_step.checkpoint_id}),
        );
        state.events.publish(
            WebEventType::PausedForApproval.as_str(),
            Some(&task_id),
            json!({"node_id": pending_step.node_id}),
        );
    }
    state.events.publish(
        WebEventType::ProjectionUpdated.as_str(),
        Some(&task_id),
        json!({}),
    );
    Ok(Json(response))
}

pub async fn confirm_task(
    State(state): State<WebAppState>,
    Path(task_id): Path<String>,
    Query(query): Query<WorkspaceQuery>,
    Json(request): Json<ConfirmTaskRequest>,
) -> ApiResult<Json<ConfirmTaskResponse>> {
    if let Some(workspace_id) = query.workspace_id.as_deref() {
        let workspace_root =
            resolve_workspace_root(&state.workspace_root, Some(workspace_id), Some(&task_id))?;
        let mut runtime = WebRuntime::new_fake(workspace_root);
        let prepared = runtime.prepare_provider_input(&task_id, &request.prompt)?;
        state.events.publish(
            WebEventType::ProviderInputPrepared.as_str(),
            Some(&task_id),
            json!({
                "node_id": prepared.node_id,
                "input_ref": prepared.input_ref,
                "input_summary": prepared.input_summary,
                "redaction_applied": prepared.redaction_applied,
                "workspace_id": workspace_id,
            }),
        );
        let response = runtime.confirm_task(&task_id, request)?;
        state.events.publish(
            WebEventType::NodeStarted.as_str(),
            Some(&task_id),
            json!({"node_id": response.node_id, "workspace_id": workspace_id}),
        );
        state.events.publish(
            WebEventType::ProjectionUpdated.as_str(),
            Some(&task_id),
            json!({"workspace_id": workspace_id}),
        );
        return Ok(Json(response));
    }
    let mut runtime = state.runtime.lock().expect("runtime lock");
    let prepared = runtime.prepare_provider_input(&task_id, &request.prompt)?;
    state.events.publish(
        WebEventType::ProviderInputPrepared.as_str(),
        Some(&task_id),
        json!({
            "node_id": prepared.node_id,
            "input_ref": prepared.input_ref,
            "input_summary": prepared.input_summary,
            "redaction_applied": prepared.redaction_applied,
        }),
    );
    let response = runtime.confirm_task(&task_id, request)?;
    state.events.publish(
        WebEventType::NodeStarted.as_str(),
        Some(&task_id),
        json!({"node_id": response.node_id}),
    );
    state.events.publish(
        WebEventType::ProviderOutput.as_str(),
        Some(&task_id),
        json!({"node_id": response.node_id, "stream": "stdout"}),
    );
    state.events.publish(
        WebEventType::ArtifactWritten.as_str(),
        Some(&task_id),
        json!({"node_id": response.node_id, "artifact_ref": "coding_report_work_wt_001_0001"}),
    );
    state.events.publish(
        WebEventType::NodeCompleted.as_str(),
        Some(&task_id),
        json!({"node_id": response.node_id}),
    );
    state.events.publish(
        WebEventType::ProjectionUpdated.as_str(),
        Some(&task_id),
        json!({}),
    );
    Ok(Json(response))
}

pub async fn stop_task(
    State(state): State<WebAppState>,
    Path(task_id): Path<String>,
    Query(query): Query<WorkspaceQuery>,
) -> ApiResult<Json<StopTaskResponse>> {
    if let Some(workspace_id) = query.workspace_id.as_deref() {
        let workspace_root =
            resolve_workspace_root(&state.workspace_root, Some(workspace_id), Some(&task_id))?;
        let mut runtime = WebRuntime::new_fake(workspace_root);
        let response = runtime.stop_task(&task_id)?;
        state.events.publish(
            WebEventType::ProjectionUpdated.as_str(),
            Some(&task_id),
            json!({ "reason": "stop_requested", "task_id": task_id, "workspace_id": workspace_id }),
        );
        return Ok(Json(response));
    }
    let mut runtime = state.runtime.lock().expect("runtime lock");
    let response = runtime.stop_task(&task_id)?;
    state.events.publish(
        WebEventType::ProjectionUpdated.as_str(),
        Some(&task_id),
        json!({ "reason": "stop_requested", "task_id": task_id }),
    );
    Ok(Json(response))
}

pub async fn rollback_preview(
    State(state): State<WebAppState>,
    Path(task_id): Path<String>,
    Query(query): Query<WorkspaceQuery>,
    Json(request): Json<RollbackPreviewRequest>,
) -> ApiResult<Json<RollbackPreviewResponse>> {
    if let Some(workspace_id) = query.workspace_id.as_deref() {
        let workspace_root =
            resolve_workspace_root(&state.workspace_root, Some(workspace_id), Some(&task_id))?;
        let runtime = WebRuntime::new_fake(workspace_root);
        let response = runtime.rollback_preview(&task_id, &request.checkpoint_id)?;
        state.events.publish(
            WebEventType::RollbackPreviewed.as_str(),
            Some(&task_id),
            json!({ "checkpoint_id": response.checkpoint_id, "workspace_id": workspace_id }),
        );
        return Ok(Json(response));
    }
    let runtime = state.runtime.lock().expect("runtime lock");
    let response = runtime.rollback_preview(&task_id, &request.checkpoint_id)?;
    state.events.publish(
        WebEventType::RollbackPreviewed.as_str(),
        Some(&task_id),
        json!({ "checkpoint_id": response.checkpoint_id }),
    );
    Ok(Json(response))
}

pub async fn rollback_task(
    State(state): State<WebAppState>,
    Path(task_id): Path<String>,
    Query(query): Query<WorkspaceQuery>,
    Json(request): Json<RollbackRequest>,
) -> ApiResult<Json<RollbackResponse>> {
    if let Some(workspace_id) = query.workspace_id.as_deref() {
        let workspace_root =
            resolve_workspace_root(&state.workspace_root, Some(workspace_id), Some(&task_id))?;
        let mut runtime = WebRuntime::new_fake(workspace_root);
        let response =
            runtime.rollback(&task_id, &request.checkpoint_id, request.force_when_dirty)?;
        state.events.publish(
            WebEventType::RollbackCompleted.as_str(),
            Some(&task_id),
            json!({ "checkpoint_id": response.checkpoint_id, "workspace_id": workspace_id }),
        );
        state.events.publish(
            WebEventType::ProjectionUpdated.as_str(),
            Some(&task_id),
            json!({ "reason": "rollback_completed", "workspace_id": workspace_id }),
        );
        return Ok(Json(response));
    }
    let mut runtime = state.runtime.lock().expect("runtime lock");
    let response = runtime.rollback(&task_id, &request.checkpoint_id, request.force_when_dirty)?;
    state.events.publish(
        WebEventType::RollbackCompleted.as_str(),
        Some(&task_id),
        json!({ "checkpoint_id": response.checkpoint_id }),
    );
    state.events.publish(
        WebEventType::ProjectionUpdated.as_str(),
        Some(&task_id),
        json!({ "reason": "rollback_completed" }),
    );
    Ok(Json(response))
}

pub async fn issue_rollback_preview(
    State(_state): State<WebAppState>,
    Path(issue_id): Path<String>,
    Json(request): Json<IssueRollbackPreviewRequest>,
) -> ApiResult<Json<RollbackPreviewResponse>> {
    validate_issue_rollback_ids(&issue_id, &request.execution_record_id)?;
    Err(issue_rollback_missing_worktree())
}

pub async fn issue_rollback(
    State(_state): State<WebAppState>,
    Path(issue_id): Path<String>,
    Json(request): Json<IssueRollbackRequest>,
) -> ApiResult<Json<RollbackResponse>> {
    validate_issue_rollback_ids(&issue_id, &request.execution_record_id)?;
    let _force_when_dirty = request.force_when_dirty;
    Err(issue_rollback_missing_worktree())
}

pub async fn projection(
    State(state): State<WebAppState>,
    Query(query): Query<ProjectionQuery>,
) -> ApiResult<Json<WebWorkspaceProjection>> {
    let workspace_root = resolve_workspace_root(
        &state.workspace_root,
        query.workspace_id.as_deref(),
        query.task_id.as_deref(),
    )?;
    Ok(Json(WebRuntime::projection_for_workspace(
        &workspace_root,
        query.task_id.as_deref(),
        query.node_id.as_deref(),
    )?))
}

pub async fn artifact_content(
    State(state): State<WebAppState>,
    Path(artifact_ref): Path<String>,
    Query(query): Query<WorkspaceQuery>,
) -> ApiResult<Json<ArtifactContentResponse>> {
    if let Some(workspace_id) = query.workspace_id.as_deref() {
        let workspace_root =
            resolve_workspace_root(&state.workspace_root, Some(workspace_id), None)?;
        let runtime = WebRuntime::new_fake(workspace_root);
        return Ok(Json(runtime.artifact_content(&artifact_ref)?));
    }
    let runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.artifact_content(&artifact_ref)?))
}

pub async fn file_content(
    State(state): State<WebAppState>,
    Query(query): Query<FileContentQuery>,
) -> ApiResult<Json<FileContentResponse>> {
    if let Some(workspace_id) = query.workspace_id.as_deref() {
        let workspace_root =
            resolve_workspace_root(&state.workspace_root, Some(workspace_id), None)?;
        let runtime = WebRuntime::new_fake(workspace_root);
        return Ok(Json(runtime.file_content(&query.path)?));
    }
    let runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.file_content(&query.path)?))
}

pub async fn file_diff(
    State(state): State<WebAppState>,
    Query(query): Query<FileDiffQuery>,
) -> ApiResult<Json<FileDiffResponse>> {
    if let Some(workspace_id) = query.workspace_id.as_deref() {
        let workspace_root =
            resolve_workspace_root(&state.workspace_root, Some(workspace_id), None)?;
        let runtime = WebRuntime::new_fake(workspace_root);
        return Ok(Json(
            runtime.file_diff(&query.base_checkpoint, &query.path)?,
        ));
    }
    let runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(
        runtime.file_diff(&query.base_checkpoint, &query.path)?,
    ))
}

pub async fn provider_input_content(
    State(state): State<WebAppState>,
    Path((issue_id, input_ref)): Path<(String, String)>,
) -> ApiResult<Json<ProviderInputContentResponse>> {
    let file_name = provider_input_file_name(&input_ref)?;
    let issue = IssueRegistry::new(state.workspace_root.clone()).get(&issue_id)?;
    let workspace_id = issue.workspace_id.as_deref().ok_or_else(|| {
        ApiError::runtime(
            "task_workspace_not_found",
            "issue has no active task binding",
            json!({}),
        )
    })?;
    let task_id = issue.task_id.as_deref().ok_or_else(|| {
        ApiError::runtime(
            "task_workspace_not_found",
            "issue has no active task binding",
            json!({}),
        )
    })?;
    validate_relative_id(task_id)
        .map_err(|_| ApiError::validation("invalid_task_id", "invalid task id"))?;
    let workspace = WorkspaceRegistry::new(state.workspace_root.clone()).get(workspace_id)?;
    let workspace_root = canonical_provider_input_component(&workspace.path)?;
    let runtime_tasks_root = workspace_root.join(".aria/runtime/tasks");
    let task_root = runtime_tasks_root.join(task_id);
    let path = canonical_provider_input_path(
        &workspace_root,
        &runtime_tasks_root,
        &task_root,
        &file_name,
    )?;
    let content = fs::read_to_string(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => {
            ApiError::runtime("artifact_not_found", "provider input not found", json!({}))
        }
        _ => ApiError::runtime(
            "provider_input_read_failed",
            "provider input read failed",
            json!({}),
        ),
    })?;
    let redacted = redact_sensitive_lines(&content);
    Ok(Json(ProviderInputContentResponse {
        input_ref,
        content_type: "application/json".to_string(),
        redaction_applied: redacted != content,
        content: redacted,
    }))
}

fn canonical_provider_input_path(
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

fn canonical_provider_input_component(path: &StdPath) -> ApiResult<PathBuf> {
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

fn provider_input_path_escape() -> ApiError {
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

fn sse_event(event: WebEvent) -> Event {
    Event::default()
        .id(event.cursor.to_string())
        .event(event.event_type.clone())
        .json_data(event)
        .expect("serialize web event")
}

fn resolve_gate(
    state: &WebAppState,
    issue_id: String,
    gate_id: String,
    project_id: Option<String>,
    status: GateStatus,
    decision: &str,
    request: ResolveGateRequest,
) -> ApiResult<Json<ResolveGateResponse>> {
    let store = GateStore::new(product_app_paths(state));
    let ResolveGateRequest {
        comment,
        requested_change,
    } = request;
    let gate = match project_id {
        Some(project_id) => store
            .resolve(
                &project_id,
                &issue_id,
                &gate_id,
                status,
                comment,
                requested_change,
            )
            .map_err(product_store_api_error)?,
        None => {
            let project_ids = store
                .project_ids_for_gate(&issue_id, &gate_id)
                .map_err(product_store_api_error)?;
            match project_ids.as_slice() {
                [project_id] => store
                    .resolve(
                        project_id,
                        &issue_id,
                        &gate_id,
                        status,
                        comment,
                        requested_change,
                    )
                    .map_err(product_store_api_error)?,
                [] => {
                    return Err(product_store_api_error(ProductStoreError::NotFound {
                        kind: "gate",
                        id: gate_id,
                    }));
                }
                _ => {
                    return Err(ApiError::runtime(
                        "gate_ambiguous",
                        "gate matches multiple projects",
                        json!({}),
                    ));
                }
            }
        }
    };
    Ok(Json(ResolveGateResponse {
        issue_id: gate.issue_id,
        gate_id: gate.id,
        node_id: gate.node_id,
        decision: decision.to_string(),
        next_node: None,
    }))
}

fn resolve_workspace_root(
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

fn provider_input_file_name(input_ref: &str) -> ApiResult<String> {
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

fn workspace_dto(record: WorkspaceRecord) -> WorkspaceDto {
    WorkspaceDto {
        workspace_id: record.workspace_id,
        name: record.name,
        path: record.path.to_string_lossy().to_string(),
        default_policy_preset: record.default_policy_preset,
        default_provider_mode: record.default_provider_mode,
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

fn project_dto(record: ProjectRecord) -> ProjectDto {
    ProjectDto {
        project_id: record.id,
        name: record.name,
        description: record.description,
        created_at: record.created_at,
        updated_at: record.updated_at,
        last_opened_at: record.last_opened_at,
    }
}

fn repository_dto(record: RepositoryRecord) -> RepositoryDto {
    RepositoryDto {
        repository_id: record.id,
        project_id: record.project_id,
        name: record.name,
        path: record.path.to_string_lossy().to_string(),
        repo_hash: record.repo_hash,
        runtime_root: record.runtime_root.to_string_lossy().to_string(),
        default_policy_preset: record.default_policy_preset,
        default_provider_mode: record.default_provider_mode,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

fn product_issue_dto_with_binding(
    app_paths: &ProductAppPaths,
    record: ProductIssueRecord,
) -> ApiResult<ProductIssueDto> {
    let active_binding = active_binding_for_issue(app_paths, &record.project_id, &record)?;
    Ok(product_issue_dto(record, active_binding))
}

fn product_issue_dto(
    record: ProductIssueRecord,
    active_binding: Option<IssueRuntimeBindingRecord>,
) -> ProductIssueDto {
    let workspace_id = active_binding
        .as_ref()
        .map(|binding| product_execution_workspace_id(&record.project_id, &binding.repo_id));
    let task_id = active_binding
        .as_ref()
        .and_then(|binding| binding.task_id.clone());
    let session_id = active_binding
        .as_ref()
        .and_then(|binding| binding.session_id.clone());
    let artifacts = active_binding
        .as_ref()
        .map(product_issue_artifacts)
        .unwrap_or_default();
    ProductIssueDto {
        issue_id: record.id,
        project_id: record.project_id,
        repo_id: record.repo_id,
        workspace_id,
        task_id,
        session_id,
        title: record.title,
        description: record.description,
        change_id: record.change_id,
        phase: product_issue_phase_text(&record.phase).to_string(),
        status: product_issue_status_text(&record.status).to_string(),
        active_binding_id: record.active_binding_id,
        artifacts,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

fn story_spec_dto(record: StorySpecRecord) -> StorySpecDto {
    StorySpecDto {
        story_spec_id: record.id,
        issue_id: record.issue_id,
        repository_id: record.repository_id,
        title: record.title,
        current_version: record.current_version,
        confirmation_status: lifecycle_confirmation_status_text(&record.confirmation_status)
            .to_string(),
    }
}

fn design_spec_dto(record: DesignSpecRecord) -> DesignSpecDto {
    DesignSpecDto {
        design_spec_id: record.id,
        issue_id: record.issue_id,
        story_spec_ids: record.story_spec_ids,
        design_kind: design_kind_text(&record.design_kind).to_string(),
        title: record.title,
        current_version: record.current_version,
        confirmation_status: lifecycle_confirmation_status_text(&record.confirmation_status)
            .to_string(),
    }
}

fn lifecycle_work_item_dto(record: LifecycleWorkItemRecord) -> LifecycleWorkItemDto {
    LifecycleWorkItemDto {
        work_item_id: record.id,
        issue_id: record.issue_id,
        repository_id: record.repository_id,
        story_spec_ids: record.story_spec_ids,
        design_spec_ids: record.design_spec_ids,
        title: record.title,
        plan_status: work_item_plan_status_text(&record.plan_status).to_string(),
        execution_status: work_item_status_text(&record.execution_status).to_string(),
    }
}

fn workspace_session_dto(record: WorkspaceSessionRecord) -> WorkspaceSessionDto {
    WorkspaceSessionDto {
        workspace_session_id: record.id,
        issue_id: record.issue_id,
        entity_id: record.entity_id,
        workspace_type: workspace_type_text(&record.workspace_type).to_string(),
        status: workspace_session_status_text(&record.status).to_string(),
        author_provider: provider_name_text(&record.author_provider).to_string(),
        reviewer_provider: provider_name_text(&record.reviewer_provider).to_string(),
        review_rounds: record.review_rounds,
        superpowers_enabled: record.superpowers_enabled,
        openspec_enabled: record.openspec_enabled,
        messages: record
            .messages
            .into_iter()
            .map(workspace_message_dto)
            .collect(),
    }
}

fn workspace_message_dto(record: WorkspaceMessageRecord) -> WorkspaceMessageDto {
    WorkspaceMessageDto {
        role: record.role,
        content: record.content,
        created_at: record.created_at,
    }
}

fn product_issue_artifacts(binding: &IssueRuntimeBindingRecord) -> Vec<ProductIssueArtifactDto> {
    let Some(task_id) = binding.task_id.as_deref() else {
        return Vec::new();
    };
    let Some(workspace_root) = workspace_root_for_binding(binding) else {
        return Vec::new();
    };
    WebRuntime::projection_for_workspace(&workspace_root, Some(task_id), None)
        .map(|projection| {
            projection
                .artifact_index
                .into_iter()
                .map(|artifact| ProductIssueArtifactDto {
                    stage: artifact_stage(
                        &artifact.artifact_kind,
                        artifact.producer_node.as_deref(),
                    )
                    .to_string(),
                    artifact_ref: artifact.artifact_ref,
                    artifact_kind: artifact.artifact_kind,
                    producer_node: artifact.producer_node,
                    path: artifact.path,
                    summary: artifact.summary,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn workspace_root_for_binding(binding: &IssueRuntimeBindingRecord) -> Option<PathBuf> {
    binding.runtime_root.parent()?.parent().map(PathBuf::from)
}

fn artifact_stage(artifact_kind: &str, producer_node: Option<&str>) -> &'static str {
    match producer_node {
        Some("N04" | "N05" | "N06" | "N07") => return "story_spec",
        Some("N08" | "N09" | "N10" | "N11" | "N12") => return "design_spec",
        Some("N27") => return "done",
        Some(_) => return "work_item",
        None => {}
    }
    if artifact_kind.contains("clarification")
        || artifact_kind == "spec"
        || artifact_kind == "openspec_spec"
        || artifact_kind == "openspec_proposal"
    {
        "story_spec"
    } else if artifact_kind.contains("design") {
        "design_spec"
    } else if artifact_kind.contains("final") {
        "done"
    } else {
        "work_item"
    }
}

fn active_binding_for_issue(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue: &ProductIssueRecord,
) -> ApiResult<Option<IssueRuntimeBindingRecord>> {
    let Some(active_binding_id) = issue.active_binding_id.as_deref() else {
        return Ok(None);
    };
    Ok(RuntimeBindingStore::new(app_paths.clone())
        .list(project_id, &issue.id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|binding| binding.id == active_binding_id))
}

fn issue_dto(record: IssueRecord) -> IssueDto {
    IssueDto {
        issue_id: record.issue_id,
        title: record.title,
        description: record.description,
        status: issue_status_text(&record.status).to_string(),
        workspace_id: record.workspace_id,
        task_id: record.task_id,
        session_id: record.session_id,
        change_id: record.change_id,
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

fn find_repository(
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

fn product_execution_workspace_id(project_id: &str, repository_id: &str) -> String {
    format!("product:{project_id}:{repository_id}")
}

fn parse_product_execution_workspace_id(value: &str) -> Option<(&str, &str)> {
    let mut parts = value.split(':');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("product"), Some(project_id), Some(repository_id), None) => {
            Some((project_id, repository_id))
        }
        _ => None,
    }
}

fn product_issue_phase_text(phase: &ProductIssuePhase) -> &'static str {
    match phase {
        ProductIssuePhase::Clarification => "clarification",
        ProductIssuePhase::Development => "development",
        ProductIssuePhase::Acceptance => "acceptance",
    }
}

fn product_issue_status_text(status: &ProductIssueStatus) -> &'static str {
    match status {
        ProductIssueStatus::Draft => "draft",
        ProductIssueStatus::InProgress => "in_progress",
        ProductIssueStatus::Completed => "completed",
        ProductIssueStatus::Blocked => "blocked",
    }
}

fn lifecycle_confirmation_status_text(status: &LifecycleConfirmationStatus) -> &'static str {
    match status {
        LifecycleConfirmationStatus::Draft => "draft",
        LifecycleConfirmationStatus::InReview => "in_review",
        LifecycleConfirmationStatus::Confirmed => "confirmed",
        LifecycleConfirmationStatus::ChangeRequested => "change_requested",
        LifecycleConfirmationStatus::Blocked => "blocked",
    }
}

fn design_kind_text(kind: &DesignKind) -> &'static str {
    match kind {
        DesignKind::Frontend => "frontend",
        DesignKind::Backend => "backend",
    }
}

fn work_item_plan_status_text(status: &WorkItemPlanStatus) -> &'static str {
    match status {
        WorkItemPlanStatus::NotStarted => "not_started",
        WorkItemPlanStatus::Draft => "draft",
        WorkItemPlanStatus::Confirmed => "confirmed",
        WorkItemPlanStatus::ChangeRequested => "change_requested",
    }
}

fn work_item_status_text(status: &WorkItemStatus) -> &'static str {
    match status {
        WorkItemStatus::Pending => "pending",
        WorkItemStatus::Planning => "planning",
        WorkItemStatus::Coding => "coding",
        WorkItemStatus::Completed => "completed",
        WorkItemStatus::Blocked => "blocked",
    }
}

fn workspace_type_text(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "story",
        WorkspaceType::Design => "design",
        WorkspaceType::WorkItem => "work_item",
    }
}

fn workspace_session_status_text(status: &WorkspaceSessionStatus) -> &'static str {
    match status {
        WorkspaceSessionStatus::Open => "open",
        WorkspaceSessionStatus::Running => "running",
        WorkspaceSessionStatus::WaitingForHuman => "waiting_for_human",
        WorkspaceSessionStatus::Confirmed => "confirmed",
        WorkspaceSessionStatus::ChangeRequested => "change_requested",
        WorkspaceSessionStatus::BlockedProviderUnavailable => "blocked_provider_unavailable",
        WorkspaceSessionStatus::Terminated => "terminated",
    }
}

fn provider_name_text(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::ClaudeCode => "claude_code",
        ProviderName::Codex => "codex",
        ProviderName::Fake => "fake",
    }
}

fn issue_status_text(status: &IssueStatus) -> &'static str {
    match status {
        IssueStatus::Draft => "draft",
        IssueStatus::Started => "started",
        IssueStatus::Running => "running",
        IssueStatus::Completed => "completed",
        IssueStatus::Blocked => "blocked",
    }
}

fn product_app_paths(state: &WebAppState) -> ProductAppPaths {
    ProductAppPaths::new(state.workspace_root.join(".aria"))
}

fn provider_workspace_config(
    author_provider: Option<&str>,
    reviewer_provider: Option<&str>,
    review_rounds: Option<u32>,
    superpowers_enabled: Option<bool>,
    openspec_enabled: Option<bool>,
) -> ApiResult<ProviderWorkspaceConfig> {
    let review_rounds = review_rounds.unwrap_or(1);
    if !(1..=5).contains(&review_rounds) {
        return Err(ApiError::validation(
            "invalid_review_rounds",
            "review_rounds must be between 1 and 5",
        ));
    }

    Ok(ProviderWorkspaceConfig {
        author_provider: parse_provider_name(author_provider.unwrap_or("codex"))?,
        reviewer_provider: parse_provider_name(reviewer_provider.unwrap_or("claude_code"))?,
        review_rounds,
        superpowers_enabled: superpowers_enabled.unwrap_or(true),
        openspec_enabled: openspec_enabled.unwrap_or(true),
    })
}

fn parse_provider_name(value: &str) -> ApiResult<ProviderName> {
    match value {
        "claude_code" => Ok(ProviderName::ClaudeCode),
        "codex" => Ok(ProviderName::Codex),
        "fake" => Ok(ProviderName::Fake),
        _ => Err(ApiError::validation(
            "invalid_provider",
            "provider must be claude_code, codex, or fake",
        )),
    }
}

fn parse_design_kind(value: &str) -> ApiResult<DesignKind> {
    match value {
        "frontend" => Ok(DesignKind::Frontend),
        "backend" => Ok(DesignKind::Backend),
        _ => Err(ApiError::validation(
            "invalid_design_kind",
            "design_kind must be frontend or backend",
        )),
    }
}

fn validate_confirmed_story_specs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    story_spec_ids: &[String],
) -> ApiResult<()> {
    if story_spec_ids.is_empty() {
        return Err(ApiError::validation(
            "story_spec_required",
            "story_spec_ids is required",
        ));
    }

    let stories = lifecycle
        .list_story_specs(project_id, issue_id)
        .map_err(product_store_api_error)?;
    for story_id in story_spec_ids {
        let Some(story) = stories.iter().find(|story| story.id == *story_id) else {
            return Err(ApiError::runtime(
                "story_spec_not_found",
                "story spec not found",
                json!({}),
            ));
        };
        if story.confirmation_status != LifecycleConfirmationStatus::Confirmed {
            return Err(ApiError::validation(
                "story_spec_not_confirmed",
                "story spec must be confirmed before generating downstream artifacts",
            ));
        }
    }
    Ok(())
}

fn validate_confirmed_design_specs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    design_spec_ids: &[String],
) -> ApiResult<()> {
    if design_spec_ids.is_empty() {
        return Err(ApiError::validation(
            "design_spec_required",
            "design_spec_ids is required",
        ));
    }

    let designs = lifecycle
        .list_design_specs(project_id, issue_id)
        .map_err(product_store_api_error)?;
    for design_id in design_spec_ids {
        let Some(design) = designs.iter().find(|design| design.id == *design_id) else {
            return Err(ApiError::runtime(
                "design_spec_not_found",
                "design spec not found",
                json!({}),
            ));
        };
        if design.confirmation_status != LifecycleConfirmationStatus::Confirmed {
            return Err(ApiError::validation(
                "design_spec_not_confirmed",
                "design spec must be confirmed before generating work items",
            ));
        }
    }
    Ok(())
}

fn confirm_workspace_entity(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> ApiResult<()> {
    match session.workspace_type {
        WorkspaceType::Story | WorkspaceType::Design => lifecycle
            .update_spec_confirmation_status(
                &session.project_id,
                &session.issue_id,
                &session.entity_id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .map_err(product_store_api_error),
        WorkspaceType::WorkItem => lifecycle
            .update_work_item_plan_status(
                &session.project_id,
                &session.issue_id,
                &session.entity_id,
                WorkItemPlanStatus::Confirmed,
            )
            .map(|_| ())
            .map_err(product_store_api_error),
    }
}

fn validate_workspace_message(request: &WorkspaceSessionMessageRequest) -> ApiResult<()> {
    if !matches!(
        request.role.as_str(),
        "user" | "assistant" | "system" | "provider" | "reviewer"
    ) || request.content.trim().is_empty()
    {
        return Err(ApiError::validation(
            "invalid_workspace_message",
            "workspace message role/content is invalid",
        ));
    }
    Ok(())
}

fn workspace_user_prompt(user_prompt: Option<String>) -> String {
    user_prompt
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "请基于当前 Issue 上下文生成或修订 Workspace 产物。".to_string())
}

fn provider_workspace_prompt(prompt: String) -> String {
    let structured = json!({
        "markdown": format!("# Provider Workspace\n\n{prompt}"),
        "review_result": "review completed",
        "revision_result": "revision completed"
    });
    format!("{prompt}\n\n{STRUCTURED_OUTPUT_START}\n{structured}\n{STRUCTURED_OUTPUT_END}")
}

fn validate_issue_rollback_ids(issue_id: &str, execution_record_id: &str) -> ApiResult<()> {
    validate_relative_id(issue_id)
        .map_err(|_| ApiError::validation("invalid_issue_id", "invalid issue id"))?;
    validate_relative_id(execution_record_id).map_err(|_| {
        ApiError::validation("invalid_execution_record_id", "invalid execution record id")
    })?;
    Ok(())
}

fn issue_rollback_missing_worktree() -> ApiError {
    ApiError::validation(
        "issue_rollback_missing_worktree",
        "issue rollback requires a work item worktree",
    )
}

fn product_store_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::NotFound {
            kind: "project", ..
        } => ApiError::runtime("project_not_found", "project not found", json!({})),
        ProductStoreError::NotFound {
            kind: "repository", ..
        } => ApiError::runtime("repository_not_found", "repository not found", json!({})),
        ProductStoreError::NotFound { kind: "issue", .. } => {
            ApiError::runtime("issue_not_found", "issue not found", json!({}))
        }
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
        ProductStoreError::PathEscape(_) => {
            ApiError::validation("invalid_project_id", "invalid project id")
        }
        _ => ApiError::runtime(
            "product_store_error",
            "product store operation failed",
            json!({}),
        ),
    }
}
