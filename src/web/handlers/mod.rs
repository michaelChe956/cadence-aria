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

use crate::cross_cutting::provider_adapter::{FakeProviderAdapter, structured_output_sentinel};
use crate::interactive::models::WebWorkspaceProjection;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CreateCodingAttemptInput, CreateGroupCodingAttemptInput,
    is_group_business_validation_error,
};
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingExecutionUnitStatus, CodingTimelineNode, CodingTimelineNodeStatus,
    GroupReviewArtifactRef, PushStatus, WorkItemDependencyHandoffRef, WorkItemExecutionPlan,
};
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};
use crate::product::gate_store::GateStore;
use crate::product::git_workspace_service::{GitWorkspaceError, GitWorkspaceService};
use crate::product::issue_store::{CreateProductIssueWithRepositoryInput, IssueStore};
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::lifecycle_store::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateStorySpecInput,
    CreateWorkspaceSessionInput, LifecycleStore, UpsertIssueSharedWorktreeInput,
    UpsertRepoSharedWorktreeInput,
};
// 仅测试模块消费（lifecycle_tests.inc.rs 经 `use super::*` 取用）。
#[cfg(test)]
use crate::product::lifecycle_store::CreateIssueWorkItemPlanInput;
use crate::product::models::{
    DesignSpecRecord, GateStatus, IssuePhase as ProductIssuePhase,
    IssueRecord as ProductIssueRecord, IssueRuntimeBindingRecord,
    IssueStatus as ProductIssueStatus, LifecycleConfirmationStatus, LifecycleWorkItemRecord,
    NodeDetail, ProjectRecord, ProviderName, RepositoryRecord, StorySpecRecord,
    WorkItemExecutionPlanStatus, WorkItemKind, WorkItemStatus, WorkspaceMessageRecord,
    WorkspaceRolePermissionModes, WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::models::{
    IssueWorkItemPlan as IssueWorkItemPlanRecord, IssueWorkItemPlanStatus, WorkItemPlanStatus,
};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::provider_workspace_runner::{
    ProviderWorkspaceRunner, WorkspaceProviderRunInput,
};
use crate::product::repository_store::{
    DeleteRepositoryCommand, RepositoryDeletionReceipt, RepositoryStore,
};
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

mod aggregate_index;
pub mod aggregate_initialization;
pub(crate) mod automation_enrollment;
pub(crate) mod automation_enrollment_conflict;
pub(crate) mod automation_enrollment_reenable;
#[cfg(test)]
pub(crate) mod automation_enrollment_test_support;
mod automation_gateway_preflight;
mod automation_target;
mod candidate_attempt_metrics;
mod candidate_attempt_metrics_project;
mod codebases;
mod coding;
mod coding_choice;
pub(crate) mod dto;
mod evidence_error_mapping;
mod evidence_query;
mod gateway_error_mapping;
mod health;
mod identity_repair;
pub mod image_create;
pub mod knowledge_base;
pub(crate) mod lifecycle;
mod logical_codebase_bootstrap;
mod logical_codebase_capabilities;
mod logical_codebase_capability_batch;
mod logical_codebase_members;
mod logical_codebase_registration;
mod pointer_publication;
mod pointer_publish_error_mapping;
mod product_resources;
mod provider_diagnostics;
pub mod provider_installation;
mod providers;
mod repository_registration;
mod runtime;
mod support;
mod workspace_choice;
mod workspace_human_action;
mod workspace_sc_recovery;
mod workspace_session;

#[rustfmt::skip]
pub use coding::{create_coding_attempt, create_group_coding_attempt, delete_plan_coding_workspaces, get_plan_coding_progress, post_plan_coding_final_confirm, post_plan_coding_start};
#[rustfmt::skip]
pub(crate) use coding::{
    abort_coding_attempt, build_group_work_item_progress, coding_attempt_artifact_content, coding_attempt_diff,
    restart_coding_attempt,
    EnrolledPlanDriveIdentity, EnrolledPlanStartStep, enrolled_plan_coding_start,
    upsert_automation_drive_failure_fact,
    confirm_work_item_execution_plan, delete_coding_attempt, get_coding_attempt,
    resolve_reviewer_window_manual_gate,
    post_coding_gate_response,
    get_coding_policy_text, get_verification_command_evidence,
    get_verification_triage_records, post_coding_policy_reauthorization,
    post_rerun_planned_command, post_verification_triage_decision,
    post_verification_triage_enter,
    request_work_item_execution_plan_change, drive_ready_units_for_plan,
};
pub use automation_enrollment::{
    get_automation_enrollment, post_automation_enrollment_binding,
    post_automation_enrollment_continue_chain, post_automation_enrollment_lease_takeover,
    post_automation_enrollment_rebind, post_automation_enrollment_terminate,
    put_automation_enrollment,
};
pub use automation_target::get_automation_target;
pub use candidate_attempt_metrics::get_issue_candidate_attempt_metrics;
pub use candidate_attempt_metrics_project::get_project_candidate_attempt_metrics;
pub use codebases::{
    create_logical_codebase, delete_logical_codebase, get_logical_codebase, list_codebases,
    list_logical_codebases, purge_logical_codebase, restore_logical_codebase,
};
/// P1-a（契约 A2）：plan 级聚合最终确认纯投影（块 4 coding_attention_info
/// 同源调用，不经过 handler）。
pub(crate) use coding::build_plan_final_confirm;
#[cfg(test)]
pub(crate) use coding::scope::CodingAttemptRoutePath;
pub use evidence_query::evidence_query;
pub use health::{health, runtime_info};
pub use identity_repair::{get_lc_identity_repair, post_lc_identity_repair_action};
pub use image_create::{
    create_session as create_image_create_session, delete_session as delete_image_create_session,
    generate_image, get_session as get_image_create_session,
    get_settings as get_image_create_settings, image_create_chat_ws,
    list_sessions as list_image_create_sessions,
    probe_settings_connectivity as probe_image_create_connectivity,
    update_settings as update_image_create_settings,
};
pub use logical_codebase_bootstrap::{
    get_lc_bootstrap_projection, get_logical_codebase_bootstrap, post_lc_bootstrap_action,
    post_lc_bootstrap_advance, post_logical_codebase_bootstrap_action,
};
pub use logical_codebase_capabilities::{get_lc_capabilities, post_lc_capability_revalidate};
pub use logical_codebase_capability_batch::{
    get_lc_capability_revalidate_all_job, post_lc_capability_revalidate_all,
};
pub use logical_codebase_members::{
    list_lc_logical_codebase_members, list_logical_codebase_members,
};
pub use logical_codebase_registration::{
    cancel_lc_registration, cancel_logical_codebase_registration, get_lc_registration_batch,
    get_logical_codebase_registration_batch, preflight_lc_registration,
    preflight_logical_codebase_registration, resume_lc_registration,
    resume_logical_codebase_registration, submit_lc_registration,
    submit_logical_codebase_registration,
};
pub use provider_diagnostics::provider_diagnostics;
pub use provider_installation::{
    get_install_job, get_install_targets, post_provider_expose, post_provider_install,
};
pub use providers::{providers_recheck, providers_status};
#[rustfmt::skip]
pub use lifecycle::{confirm_gate, delete_design_spec, delete_story_spec, delete_work_item, delete_work_item_plan, generate_design_specs, generate_story_specs, issue_lifecycle, prepare_work_item_plan, request_gate_change, terminate_gate};
#[rustfmt::skip]
pub use product_resources::{create_issue, create_product_issue, create_project, create_workspace, delete_issue, delete_product_issue, delete_project, delete_repository, delete_workspace, get_project, list_issues, list_product_issues, list_projects, list_repositories, list_repository_branches, list_workspaces, open_project};
pub use aggregate_index::{
    AggregateIndexActiveResponse, get_active_aggregate_index, get_lc_active_aggregate_index,
    rebuild_aggregate_index, rebuild_lc_aggregate_index,
};
pub use aggregate_initialization::{
    AggregateInitializationDependencies, cancel_aggregate_initialization,
    cancel_lc_aggregate_initialization, create_aggregate_initialization,
    create_lc_aggregate_initialization, create_lc_aggregate_reinitialization,
    get_aggregate_initialization, get_aggregate_initialization_active,
    get_lc_aggregate_initialization, get_lc_aggregate_initialization_active,
};
pub(crate) use coding::map_advance_outcome;
pub(crate) use coding::ws_gate_response_routes_termination;
pub(crate) use coding_choice::{get_coding_choice_response_status, post_coding_choice_response};
pub(crate) use dto::{coding_attempt_scope_text, coding_execution_unit_dto};
pub use pointer_publication::{
    create_lc_pointer_publication, create_pointer_publication, get_lc_pointer_publication,
    get_pointer_publication, list_lc_pointer_publications, list_pointer_publications,
    retry_lc_pointer_publication_repo, retry_pointer_publication_repo,
    revoke_lc_pointer_publication, revoke_pointer_publication,
};
pub use repository_registration::{
    RepositoryRegistrationDependencies, RepositoryRegistrationDependenciesBuilder,
    create_repository, get_project_repository_initialization_waiting_items,
    get_repository_initialization, post_repository_initialization_resume,
};
pub(crate) use workspace_choice::{
    get_workspace_choice_response_status, post_workspace_choice_response,
};
pub(crate) use workspace_human_action::post_workspace_human_action;
pub(crate) use workspace_sc_recovery::{
    get_automation_enrollment_control, post_automation_continue_plan,
    post_workspace_failed_sc_run_retry,
};
#[rustfmt::skip]
pub use runtime::{advance_task, artifact_content, confirm_task, create_task, file_content, file_diff, issue_rollback, issue_rollback_preview, list_tasks, projection, provider_input_content, rollback_preview, rollback_task, stop_task};
#[rustfmt::skip]
pub use support::{EventsQuery, FileContentQuery, FileDiffQuery, GateResolveQuery, ProjectionQuery, WorkspaceQuery, events};
pub(crate) use support::provider_gateway_error_code;
pub(crate) use support::{AutomationCarrierResolution, resolve_automation_carrier};
#[rustfmt::skip]
pub use workspace_session::{workspace_session_artifact_version, workspace_session_confirm, workspace_session_lease_diagnostics, workspace_session_message, workspace_session_run_next, workspace_session_takeover, workspace_session_timeline_event_output, workspace_session_timeline_node_detail, workspace_session_timeline_node_prompt};
