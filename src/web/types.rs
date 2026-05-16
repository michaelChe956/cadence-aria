use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateTaskRequest {
    pub request_text: String,
    pub change_id: String,
    pub policy_preset: String,
    pub provider_mode: String,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateTaskResponse {
    pub task_id: String,
    pub session_id: String,
    pub change_id: String,
    pub phase: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PendingProviderStepDto {
    pub node_id: String,
    pub provider_type: String,
    pub runtime_role: String,
    pub adapter_role: String,
    pub prompt: String,
    pub input_summary: Value,
    pub canonical_input_refs: Vec<String>,
    pub context_files: Vec<String>,
    pub output_schema: String,
    pub allowed_write_scope: Vec<String>,
    pub forbidden_actions: Vec<String>,
    pub verification_commands: Vec<String>,
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AdvanceTaskResponse {
    Advanced {
        projection_version: u64,
    },
    PausedForApproval {
        pending_step: Box<PendingProviderStepDto>,
    },
    Completed {
        projection_version: u64,
    },
}

impl AdvanceTaskResponse {
    pub fn expect_pending_step(self) -> Option<PendingProviderStepDto> {
        match self {
            AdvanceTaskResponse::PausedForApproval { pending_step } => Some(*pending_step),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConfirmTaskRequest {
    pub checkpoint_id: String,
    pub prompt: String,
    pub policy_override: Option<String>,
    pub provider_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConfirmTaskResponse {
    pub status: String,
    pub node_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackPreviewRequest {
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackPreviewResponse {
    pub checkpoint_id: String,
    pub git_head: Option<String>,
    pub dirty: bool,
    pub turns_to_drop: usize,
    pub node_runs_to_drop: usize,
    pub provider_runs_to_drop: usize,
    pub artifacts_to_drop: usize,
    pub files_may_change: Vec<String>,
}

impl From<crate::interactive::checkpoint::RollbackPreview> for RollbackPreviewResponse {
    fn from(preview: crate::interactive::checkpoint::RollbackPreview) -> Self {
        Self {
            checkpoint_id: preview.checkpoint_id,
            git_head: preview.git_head,
            dirty: preview.dirty,
            turns_to_drop: preview.turns_to_drop,
            node_runs_to_drop: preview.node_runs_to_drop,
            provider_runs_to_drop: preview.provider_runs_to_drop,
            artifacts_to_drop: preview.artifacts_to_drop,
            files_may_change: preview.files_may_change,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackRequest {
    pub checkpoint_id: String,
    pub force_when_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueRollbackPreviewRequest {
    pub execution_record_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueRollbackRequest {
    pub execution_record_id: String,
    pub force_when_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackResponse {
    pub status: String,
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WebEvent {
    pub cursor: u64,
    pub event_type: String,
    pub task_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskListResponse {
    pub tasks: Vec<TaskListItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskListItem {
    pub task_id: String,
    pub change_id: Option<String>,
    pub phase: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactContentResponse {
    pub artifact_ref: String,
    pub artifact_kind: String,
    pub producer_node: Option<String>,
    pub path: String,
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FileContentResponse {
    pub path: String,
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FileDiffResponse {
    pub base_checkpoint: String,
    pub path: String,
    pub diff: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderInputContentResponse {
    pub input_ref: String,
    pub content_type: String,
    pub content: String,
    pub redaction_applied: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderInputPrepared {
    pub node_id: String,
    pub input_ref: String,
    pub input_summary: Value,
    pub redaction_applied: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderOutputChunk {
    pub node_id: String,
    pub provider_run_id: String,
    pub stream: String,
    pub text: String,
    pub structured_output: Option<Value>,
    pub manual_gate: Option<String>,
    pub retry_attempt: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StopTaskResponse {
    pub status: String,
    pub task_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceListResponse {
    pub workspaces: Vec<WorkspaceDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceDto {
    pub workspace_id: String,
    pub name: String,
    pub path: String,
    pub default_policy_preset: String,
    pub default_provider_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateWorkspaceRequest {
    pub name: String,
    pub path: String,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectDto {
    pub project_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectListResponse {
    pub projects: Vec<ProjectDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateProjectRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryDto {
    pub repository_id: String,
    pub project_id: String,
    pub name: String,
    pub path: String,
    pub repo_hash: String,
    pub runtime_root: String,
    pub default_policy_preset: String,
    pub default_provider_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryListResponse {
    pub repositories: Vec<RepositoryDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateRepositoryRequest {
    pub name: String,
    pub path: String,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProductIssueDto {
    pub issue_id: String,
    pub project_id: String,
    pub repo_id: Option<String>,
    pub workspace_id: Option<String>,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub change_id: String,
    pub phase: String,
    pub status: String,
    pub active_binding_id: Option<String>,
    pub artifacts: Vec<ProductIssueArtifactDto>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProductIssueArtifactDto {
    pub artifact_ref: String,
    pub artifact_kind: String,
    pub producer_node: Option<String>,
    pub path: String,
    pub summary: String,
    pub stage: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProductIssueListResponse {
    pub issues: Vec<ProductIssueDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateProductIssueRequest {
    pub title: String,
    pub description: Option<String>,
    pub change_id: Option<String>,
    pub repository_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueLifecycleResponse {
    pub issue: ProductIssueDto,
    pub story_specs: Vec<StorySpecDto>,
    pub design_specs: Vec<DesignSpecDto>,
    pub work_items: Vec<LifecycleWorkItemDto>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StorySpecDto {
    pub story_spec_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub title: String,
    pub current_version: Option<u32>,
    pub confirmation_status: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DesignSpecDto {
    pub design_spec_id: String,
    pub issue_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_kind: String,
    pub title: String,
    pub current_version: Option<u32>,
    pub confirmation_status: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LifecycleWorkItemDto {
    pub work_item_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub title: String,
    pub plan_status: String,
    pub execution_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateStorySpecsRequest {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateStorySpecsResponse {
    pub story_specs: Vec<StorySpecDto>,
    pub workspace_session: WorkspaceSessionDto,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionMessageRequest {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionConfirmRequest {
    pub confirmed_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceMessageDto {
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionDto {
    pub workspace_session_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: String,
    pub status: String,
    pub author_provider: String,
    pub reviewer_provider: String,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    pub messages: Vec<WorkspaceMessageDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StartProductIssueRequest {
    pub workspace_id: Option<String>,
    pub repository_id: Option<String>,
    pub policy_preset: Option<String>,
    pub provider_mode: Option<String>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StartProductIssueResponse {
    pub issue_id: String,
    pub project_id: String,
    pub repository_id: String,
    pub workspace_id: String,
    pub task_id: String,
    pub session_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueListResponse {
    pub issues: Vec<IssueDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueDto {
    pub issue_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub workspace_id: Option<String>,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub change_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateIssueRequest {
    pub title: String,
    pub description: Option<String>,
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StartIssueRequest {
    pub workspace_id: String,
    pub policy_preset: Option<String>,
    pub provider_mode: Option<String>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StartIssueResponse {
    pub issue_id: String,
    pub workspace_id: String,
    pub task_id: String,
    pub session_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResolveGateRequest {
    pub comment: Option<String>,
    pub requested_change: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResolveGateResponse {
    pub issue_id: String,
    pub gate_id: String,
    pub node_id: String,
    pub decision: String,
    pub next_node: Option<String>,
}
