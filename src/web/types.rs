use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

use crate::product::coding_models::{
    AnalystDecisionRecord, CodeReviewReport, CodingChoiceGate, CodingGateRequired,
    CodingTimelineNode, InternalPrReview, ReviewRequest, TestingReport, WorkItemExecutionPlan,
    WorkItemHandoff,
};
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

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
pub struct CodingAttemptDiffResponse {
    pub attempt_id: String,
    pub base_branch: String,
    pub worktree_path: PathBuf,
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
    pub work_item_plans: Vec<IssueWorkItemPlanDetailDto>,
    pub work_items: Vec<LifecycleWorkItemDto>,
    pub workspace_sessions: Vec<WorkspaceSessionDto>,
    pub coding_attempts: Vec<CodingAttemptDto>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactVersionDto {
    pub version: u32,
    pub markdown: String,
    pub generated_by: String,
    pub reviewed_by: Option<String>,
    pub review_verdict: Option<String>,
    pub confirmed_by: Option<String>,
    pub created_at: String,
    pub source_node_id: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StorySpecDto {
    pub story_spec_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub title: String,
    pub current_version: Option<u32>,
    pub current_markdown_preview: Option<String>,
    pub confirmation_status: String,
    pub artifact_versions: Vec<ArtifactVersionDto>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DesignSpecDto {
    pub design_spec_id: String,
    pub issue_id: String,
    pub story_spec_ids: Vec<String>,
    pub title: String,
    pub current_version: Option<u32>,
    pub current_markdown_preview: Option<String>,
    pub confirmation_status: String,
    pub artifact_versions: Vec<ArtifactVersionDto>,
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
    pub latest_attempt: Option<CodingAttemptDto>,
    pub artifact_versions: Vec<ArtifactVersionDto>,
    pub work_item_set_id: Option<String>,
    pub kind: String,
    pub sequence_hint: Option<u32>,
    pub depends_on: Vec<String>,
    pub exclusive_write_scopes: Vec<String>,
    pub forbidden_write_scopes: Vec<String>,
    pub context_budget: WorkItemContextBudgetDto,
    pub required_handoff_from: Vec<String>,
    pub verification_plan_ref: Option<String>,
    pub require_execution_plan_confirm: bool,
    pub execution_plan_status: String,
    pub handoff_summary_ref: Option<String>,
    pub completion_commit: Option<String>,
    pub completion_diff_summary_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemContextBudgetDto {
    pub target_context_k: String,
    pub max_summary_chars: usize,
    pub max_handoff_chars: usize,
    pub max_code_context_chars: usize,
    pub max_context_file_refs: usize,
    pub max_traceability_refs: usize,
    pub max_dependency_handoffs: usize,
}

impl Default for WorkItemContextBudgetDto {
    fn default() -> Self {
        Self {
            target_context_k: "30-50".to_string(),
            max_summary_chars: 20_000,
            max_handoff_chars: 12_000,
            max_code_context_chars: 30_000,
            max_context_file_refs: 80,
            max_traceability_refs: 40,
            max_dependency_handoffs: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CodingAttemptDto {
    pub attempt_id: String,
    pub work_item_id: String,
    pub attempt_no: u32,
    pub status: String,
    pub stage: String,
    pub branch_name: String,
    pub base_branch: String,
    pub worktree_path: Option<String>,
    pub rework_count: u32,
    pub head_commit: Option<String>,
    pub push_status: Option<String>,
    pub review_request_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RequestExecutionPlanChangeRequest {
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CodingAttemptSnapshotResponse {
    pub attempt: CodingAttemptDto,
    pub provider_config_snapshot: ProviderConfigSnapshot,
    pub timeline_nodes: Vec<CodingTimelineNode>,
    pub active_node_id: Option<String>,
    pub testing_report: Option<TestingReport>,
    pub code_review_reports: Vec<CodeReviewReport>,
    pub review_request: Option<ReviewRequest>,
    pub internal_pr_review: Option<InternalPrReview>,
    pub pending_gates: Vec<CodingGateRequired>,
    pub pending_choices: Vec<CodingChoiceGate>,
    pub latest_analyst_decision: Option<AnalystDecisionRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_execution_plan: Option<WorkItemExecutionPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_handoff: Option<WorkItemHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateStorySpecsRequest {
    pub title: String,
    pub author_provider: Option<String>,
    pub reviewer_provider: Option<String>,
    pub review_rounds: Option<u32>,
    pub superpowers_enabled: Option<bool>,
    pub openspec_enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateStorySpecsResponse {
    pub story_specs: Vec<StorySpecDto>,
    pub workspace_session: WorkspaceSessionDto,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateDesignSpecsRequest {
    pub title: String,
    pub story_spec_ids: Vec<String>,
    pub author_provider: Option<String>,
    pub reviewer_provider: Option<String>,
    pub review_rounds: Option<u32>,
    pub superpowers_enabled: Option<bool>,
    pub openspec_enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateDesignSpecsResponse {
    pub design_specs: Vec<DesignSpecDto>,
    pub workspace_session: WorkspaceSessionDto,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateWorkItemsRequest {
    pub title: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub include_integration_tests: Option<bool>,
    pub include_e2e_tests: Option<bool>,
    pub force_frontend_backend_split: Option<bool>,
    pub require_execution_plan_confirm: Option<bool>,
    pub author_provider: Option<String>,
    pub reviewer_provider: Option<String>,
    pub review_rounds: Option<u32>,
    pub superpowers_enabled: Option<bool>,
    pub openspec_enabled: Option<bool>,
    /// 重生时注入上一次 validate findings，让 provider 针对问题返修。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_feedback: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemSplitOptions {
    pub include_integration_tests: bool,
    pub include_e2e_tests: bool,
    pub force_frontend_backend_split: bool,
    pub require_execution_plan_confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemSplitFinding {
    pub finding_id: String,
    pub level: String,
    pub message: String,
    pub affected_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PrepareWorkItemPlanRequest {
    pub title: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub author_provider: Option<String>,
    pub reviewer_provider: Option<String>,
    pub review_rounds: Option<u32>,
    pub superpowers_enabled: Option<bool>,
    pub openspec_enabled: Option<bool>,
    pub include_integration_tests: Option<bool>,
    pub include_e2e_tests: Option<bool>,
    pub force_frontend_backend_split: Option<bool>,
    pub require_execution_plan_confirm: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PrepareWorkItemPlanResponse {
    pub work_item_plan: IssueWorkItemPlanDetailDto,
    pub workspace_session: WorkspaceSessionDto,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemPlanDetailDto {
    pub id: String,
    pub issue_id: String,
    pub project_id: String,
    pub status: String,
    pub source_story_spec_ids: Vec<String>,
    pub source_design_spec_ids: Vec<String>,
    pub work_item_ids: Vec<String>,
    pub verification_plan_ids: Vec<String>,
    pub dependency_graph: Vec<IssueWorkItemPlanDependencyEdgeDto>,
    pub repository_profile_ref: Option<String>,
    pub options: WorkItemSplitOptions,
    pub validator_findings: Vec<WorkItemSplitFinding>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemPlanDependencyEdgeDto {
    pub from_work_item_id: String,
    pub to_work_item_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionMessageRequest {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionRunNextRequest {
    pub user_prompt: Option<String>,
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
