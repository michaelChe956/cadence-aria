use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;

use crate::interactive::models::WebWorkspaceProjection;
use crate::product::app_paths::ProductAppPaths;
use crate::product::gate_store::GateStore;
use crate::product::json_store::ProductStoreError;
use crate::product::models::{GateStatus, ProjectRecord};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::web::error::{ApiError, ApiResult};
use crate::web::events::WebEventType;
use crate::web::issue_registry::{CreateIssueInput, IssueRecord, IssueRegistry, IssueStatus};
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;
use crate::web::types::{
    AdvanceTaskResponse, ArtifactContentResponse, ConfirmTaskRequest, ConfirmTaskResponse,
    CreateIssueRequest, CreateProjectRequest, CreateTaskRequest, CreateTaskResponse,
    CreateWorkspaceRequest, FileContentResponse, FileDiffResponse, IssueDto, IssueListResponse,
    ProjectDto, ProjectListResponse, ResolveGateRequest, ResolveGateResponse,
    RollbackPreviewRequest, RollbackPreviewResponse, RollbackRequest, RollbackResponse,
    StartIssueRequest, StartIssueResponse, StopTaskResponse, TaskListResponse, WebEvent,
    WorkspaceDto, WorkspaceListResponse,
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

pub async fn events(
    State(state): State<WebAppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let replay_stream = stream::iter(state.events.replay_after(0));
    let live_stream = BroadcastStream::new(state.events.subscribe())
        .filter_map(|event| async move { event.ok() });
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
        return Ok(workspace_registry.get(workspace_id)?.path);
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

fn product_store_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::NotFound {
            kind: "project", ..
        } => ApiError::runtime("project_not_found", "project not found", json!({})),
        ProductStoreError::NotFound { kind: "gate", .. } => {
            ApiError::runtime("gate_not_found", "gate not found", json!({}))
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
