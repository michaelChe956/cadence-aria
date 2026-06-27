use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{
    IntoResponse, Response,
    sse::{Event, KeepAlive, Sse},
};
use futures_util::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use std::fs;
use std::path::{Path as StdPath, PathBuf};
use std::process::{Command, Stdio};
use tokio_stream::wrappers::BroadcastStream;

use crate::cross_cutting::provider_adapter::{
    FakeProviderAdapter, STRUCTURED_OUTPUT_END, STRUCTURED_OUTPUT_START,
};
use crate::interactive::models::WebWorkspaceProjection;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CreateCodingAttemptInput, CreateCodingExecutionUnitInput,
    CreateGroupCodingAttemptInput,
};
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingExecutionUnitStatus, CodingTimelineNode, CodingTimelineNodeStatus, PushStatus,
    WorkItemDependencyHandoffRef, WorkItemExecutionPlan,
};
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};
use crate::product::gate_store::GateStore;
use crate::product::git_workspace_service::{GitWorkspaceError, GitWorkspaceService};
use crate::product::issue_store::{CreateProductIssueWithRepositoryInput, IssueStore};
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::lifecycle_store::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateIssueWorkItemPlanInput,
    CreateStorySpecInput, CreateWorkspaceSessionInput, LifecycleStore,
    UpsertIssueSharedWorktreeInput,
};
use crate::product::models::{
    DesignSpecRecord, GateStatus, IssuePhase as ProductIssuePhase,
    IssueRecord as ProductIssueRecord, IssueRuntimeBindingRecord,
    IssueStatus as ProductIssueStatus, LifecycleConfirmationStatus, LifecycleWorkItemRecord,
    NodeDetail, ProjectRecord, ProviderName, RepositoryRecord, StorySpecRecord,
    WorkItemExecutionPlanStatus, WorkItemKind, WorkItemStatus, WorkspaceMessageRecord,
    WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::models::{
    IssueWorkItemPlan as IssueWorkItemPlanRecord, IssueWorkItemPlanStatus, WorkItemPlanStatus,
};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::provider_workspace_runner::{
    ProviderWorkspaceRunner, WorkspaceProviderRunInput,
};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::product::runtime_binding_store::RuntimeBindingStore;

use crate::web::error::{ApiError, ApiResult};
use crate::web::events::WebEventType;
use crate::web::issue_registry::{CreateIssueInput, IssueRecord, IssueRegistry, IssueStatus};
use crate::web::provider_availability::{
    provider_name_key, resolve_default_coding_provider, resolve_explicit_provider_name,
};
use crate::web::redaction::redact_sensitive_lines;
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;
use crate::web::types::*;
use crate::web::workspace_context::ensure_workspace_context_message;
use crate::web::workspace_registry::{CreateWorkspaceInput, WorkspaceRecord, WorkspaceRegistry};
use crate::web::workspace_ws_types::{ArtifactVersion, ProviderConfigSnapshot, ReviewVerdictType};

mod coding;
mod dto;
mod health;
mod lifecycle;
mod product_resources;
mod runtime;
mod support;
mod workspace_session;

#[rustfmt::skip]
pub use coding::{abort_coding_attempt, coding_attempt_artifact_content, coding_attempt_diff, confirm_work_item_execution_plan, create_coding_attempt, create_group_coding_attempt, delete_coding_attempt, get_coding_attempt, request_work_item_execution_plan_change};
pub use health::{health, runtime_info};
#[rustfmt::skip]
pub use lifecycle::{confirm_gate, delete_design_spec, delete_story_spec, delete_work_item, delete_work_item_plan, generate_design_specs, generate_story_specs, issue_lifecycle, prepare_work_item_plan, request_gate_change, terminate_gate};
#[rustfmt::skip]
pub use product_resources::{create_issue, create_product_issue, create_project, create_repository, create_workspace, delete_issue, delete_product_issue, delete_project, delete_repository, delete_workspace, get_project, list_issues, list_product_issues, list_projects, list_repositories, list_workspaces, open_project};
#[rustfmt::skip]
pub use runtime::{advance_task, artifact_content, confirm_task, create_task, file_content, file_diff, issue_rollback, issue_rollback_preview, list_tasks, projection, provider_input_content, rollback_preview, rollback_task, stop_task};
#[rustfmt::skip]
pub use support::{EventsQuery, FileContentQuery, FileDiffQuery, GateResolveQuery, ProjectionQuery, WorkspaceQuery, events};
#[rustfmt::skip]
pub use workspace_session::{workspace_session_artifact_version, workspace_session_confirm, workspace_session_message, workspace_session_run_next, workspace_session_timeline_event_output, workspace_session_timeline_node_detail, workspace_session_timeline_node_prompt};
