use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssuePhase {
    Clarification,
    Development,
    Acceptance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Draft,
    InProgress,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBindingStatus {
    Created,
    Running,
    Completed,
    Blocked,
    Detached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Open,
    Confirmed,
    ChangeRequested,
    Terminated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateType {
    PolicyControlled,
    HardGate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Dropped,
    NeedsHuman,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    ClaudeCode,
    Codex,
    Fake,
}

// TODO(P1): WorkItemRecord 删除后该枚举暂无字段使用者，但它是 pub enum 不触发 dead_code lint。
// 需与 protocol::projections::ExecutionMode 区分，后续评估清理。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Agent,
    NeedsInfo,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemStatus {
    Pending,
    Planning,
    Coding,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSharedWorktreeStatus {
    NotCreated,
    Ready,
    Running,
    Blocked,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryRecord {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub path: PathBuf,
    pub repo_hash: String,
    pub runtime_root: PathBuf,
    pub default_policy_preset: String,
    pub default_provider_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueRecord {
    pub id: String,
    pub project_id: String,
    pub repo_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub change_id: String,
    pub phase: IssuePhase,
    pub status: IssueStatus,
    pub active_binding_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueRuntimeBindingRecord {
    pub id: String,
    pub issue_id: String,
    pub repo_id: String,
    pub change_id: String,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub runtime_root: PathBuf,
    pub task_root: Option<PathBuf>,
    pub status: RuntimeBindingStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GateRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub binding_id: String,
    pub node_id: String,
    pub gate_type: GateType,
    pub status: GateStatus,
    pub artifact_refs: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub resolved_at: Option<String>,
    pub comment: Option<String>,
    pub requested_change: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExecutionRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub binding_id: String,
    pub node_id: String,
    pub status: ExecutionStatus,
    pub event_type: String,
    pub artifact_refs: Vec<String>,
    pub message: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleConfirmationStatus {
    Draft,
    InReview,
    Confirmed,
    ChangeRequested,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderName {
    ClaudeCode,
    Codex,
    Fake,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderConversationRole {
    Author,
    Reviewer,
    Coder,
    Tester,
    Analyst,
    CodeReviewer,
    InternalReviewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderConversationRef {
    pub role: ProviderConversationRole,
    pub provider: ProviderName,
    pub provider_session_id: String,
    pub updated_at: String,
    pub last_node_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceType {
    Story,
    Design,
    WorkItem,
    WorkItemPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSessionStatus {
    Open,
    Running,
    WaitingForHuman,
    Confirmed,
    ChangeRequested,
    BlockedProviderUnavailable,
    Terminated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanStatus {
    NotStarted,
    Draft,
    Confirmed,
    ChangeRequested,
}

impl WorkItemPlanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkItemPlanStatus::NotStarted => "not_started",
            WorkItemPlanStatus::Draft => "draft",
            WorkItemPlanStatus::Confirmed => "confirmed",
            WorkItemPlanStatus::ChangeRequested => "change_requested",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemKind {
    Backend,
    Frontend,
    Integration,
    E2e,
    Docs,
    Infra,
    #[default]
    Other,
}

impl WorkItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Frontend => "frontend",
            Self::Integration => "integration",
            Self::E2e => "e2e",
            Self::Docs => "docs",
            Self::Infra => "infra",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemExecutionPlanStatus {
    #[default]
    NotStarted,
    Draft,
    Confirmed,
    ChangeRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemContextBudget {
    pub target_context_k: String,
    pub max_summary_chars: usize,
    pub max_handoff_chars: usize,
    pub max_code_context_chars: usize,
    pub max_context_file_refs: usize,
    pub max_traceability_refs: usize,
    pub max_dependency_handoffs: usize,
}

impl Default for WorkItemContextBudget {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StorySpecRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub title: String,
    pub current_version: Option<u32>,
    pub confirmation_status: LifecycleConfirmationStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DesignSpecRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub story_spec_ids: Vec<String>,
    pub title: String,
    pub current_version: Option<u32>,
    pub confirmation_status: LifecycleConfirmationStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LifecycleWorkItemRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub title: String,
    pub plan_status: WorkItemPlanStatus,
    pub execution_status: WorkItemStatus,
    pub worktree_path: Option<PathBuf>,
    #[serde(default)]
    pub work_item_set_id: Option<String>,
    #[serde(default)]
    pub kind: WorkItemKind,
    #[serde(default)]
    pub sequence_hint: Option<u32>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub exclusive_write_scopes: Vec<String>,
    #[serde(default)]
    pub forbidden_write_scopes: Vec<String>,
    #[serde(default)]
    pub context_budget: WorkItemContextBudget,
    #[serde(default)]
    pub required_handoff_from: Vec<String>,
    #[serde(default)]
    pub verification_plan_ref: Option<String>,
    #[serde(default)]
    pub require_execution_plan_confirm: bool,
    #[serde(default)]
    pub execution_plan_status: WorkItemExecutionPlanStatus,
    #[serde(default)]
    pub handoff_summary_ref: Option<String>,
    #[serde(default)]
    pub completion_commit: Option<String>,
    #[serde(default)]
    pub completion_diff_summary_ref: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueWorkItemPlanStatus {
    Draft,
    Confirmed,
    ChangeRequested,
}

impl IssueWorkItemPlanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Confirmed => "confirmed",
            Self::ChangeRequested => "change_requested",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueSharedWorktree {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub base_branch: String,
    pub status: IssueSharedWorktreeStatus,
    pub current_active_work_item_id: Option<String>,
    pub last_completed_work_item_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemPlanOptions {
    pub include_integration_tests: bool,
    pub include_e2e_tests: bool,
    pub force_frontend_backend_split: bool,
    pub require_execution_plan_confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemDependencyEdge {
    pub from_work_item_id: String,
    pub to_work_item_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemSplitFindingSeverity {
    Error,
    Warning,
}

impl WorkItemSplitFindingSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemSplitFinding {
    pub severity: WorkItemSplitFindingSeverity,
    pub code: String,
    pub message: String,
    pub work_item_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryProfileConfidence {
    Low,
    Medium,
    High,
}

impl RepositoryProfileConfidence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryProfile {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub provider_run_ref: Option<String>,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub package_managers: Vec<String>,
    pub test_frameworks: Vec<String>,
    pub build_systems: Vec<String>,
    pub verification_capabilities: Vec<String>,
    pub detected_layers: Vec<String>,
    pub split_recommendation: String,
    pub confidence: RepositoryProfileConfidence,
    pub uncertainties: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationScope {
    Unit,
    Integration,
    E2e,
    Build,
    Lint,
    Manual,
    Custom,
}

impl VerificationScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Integration => "integration",
            Self::E2e => "e2e",
            Self::Build => "build",
            Self::Lint => "lint",
            Self::Manual => "manual",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationCommandSource {
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationCommandSafety {
    Approved,
    NeedsManualReview,
}

impl VerificationCommandSafety {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::NeedsManualReview => "needs_manual_review",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationFallbackPolicy {
    ManualGate,
    RepairProviderOutput,
}

impl VerificationFallbackPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ManualGate => "manual_gate",
            Self::RepairProviderOutput => "repair_provider_output",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerificationCommand {
    pub id: String,
    pub label: String,
    pub command: String,
    pub cwd: String,
    pub purpose: String,
    pub required: bool,
    pub timeout_seconds: u64,
    pub source: VerificationCommandSource,
    pub safety: VerificationCommandSafety,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerificationManualCheck {
    pub id: String,
    pub label: String,
    pub instructions: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerificationPlan {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub repository_profile_ref: Option<String>,
    pub provider_run_ref: Option<String>,
    pub scope: VerificationScope,
    pub commands: Vec<VerificationCommand>,
    pub manual_checks: Vec<VerificationManualCheck>,
    pub required_gates: Vec<String>,
    pub risk_notes: Vec<String>,
    pub confidence: RepositoryProfileConfidence,
    pub fallback_policy: VerificationFallbackPolicy,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemPlan {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub source_story_spec_ids: Vec<String>,
    pub source_design_spec_ids: Vec<String>,
    pub options: IssueWorkItemPlanOptions,
    pub status: IssueWorkItemPlanStatus,
    pub work_item_ids: Vec<String>,
    pub repository_profile_ref: Option<String>,
    pub verification_plan_ids: Vec<String>,
    pub dependency_graph: Vec<IssueWorkItemDependencyEdge>,
    pub created_from_provider_run: Option<String>,
    pub validator_findings: Vec<WorkItemSplitFinding>,
    pub review_summary: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanOutline {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub source_story_spec_ids: Vec<String>,
    pub source_design_spec_ids: Vec<String>,
    pub strategy_summary: String,
    pub work_item_outlines: Vec<WorkItemOutline>,
    pub dependency_graph: Vec<WorkItemOutlineDependencyEdge>,
    pub risks: Vec<String>,
    pub handoff_strategy: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemOutline {
    pub outline_id: String,
    pub title: String,
    pub kind: WorkItemKind,
    pub goal: String,
    pub scope: Vec<String>,
    pub non_goals: Vec<String>,
    pub source_story_spec_ids: Vec<String>,
    pub source_design_spec_ids: Vec<String>,
    pub exclusive_write_scopes: Vec<String>,
    pub forbidden_write_scopes: Vec<String>,
    pub depends_on: Vec<String>,
    pub verification_intent: Vec<String>,
    pub handoff_notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemOutlineDependencyEdge {
    pub from_outline_id: String,
    pub to_outline_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemDraftCandidate {
    pub outline_id: String,
    pub title: String,
    pub kind: WorkItemKind,
    pub goal: String,
    pub implementation_context: String,
    pub exclusive_write_scopes: Vec<String>,
    pub forbidden_write_scopes: Vec<String>,
    pub depends_on_outline_ids: Vec<String>,
    pub required_handoff_from_outline_ids: Vec<String>,
    pub handoff_summary: String,
    pub verification_plan: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemDraftRecord {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub draft_id: String,
    pub outline_id: String,
    pub generation_round_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    pub attempt_index: u32,
    pub outline_version_ref: String,
    pub generation_mode: WorkItemGenerationMode,
    pub candidate: WorkItemDraftCandidate,
    pub status: WorkItemDraftStatus,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by_draft_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersede_reason: Option<WorkItemDraftSupersedeReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copied_from_draft_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_verdict_ref: Option<String>,
    pub generated_from_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemDraftStatus {
    Draft,
    Accepted,
    Superseded,
    ValidationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemGenerationMode {
    Serial,
    Batch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemDraftSupersedeReason {
    DirectRewrite,
    AncestorRewritten,
    OutlineRevised,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanDraftActiveIndex {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub current_generation_round_id: String,
    pub outline_state: String,
    pub outline_to_current_draft_id: BTreeMap<String, String>,
    pub draft_statuses: BTreeMap<String, WorkItemDraftStatus>,
    pub batches: Vec<WorkItemBatchRecord>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemBatchRecord {
    pub batch_id: String,
    pub generation_round_id: String,
    pub mode: WorkItemGenerationMode,
    pub item_draft_ids: Vec<String>,
    pub status: WorkItemBatchStatus,
    pub validation_failed_ids: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemBatchStatus {
    Generating,
    Completed,
    ReviewPending,
    ReviewDone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanCompileTransaction {
    pub compile_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub generation_round_id: String,
    pub outline_version_ref: String,
    pub active_draft_ids: Vec<String>,
    pub status: WorkItemPlanCompileStatus,
    pub plan_commit_state: WorkItemPlanCommitState,
    pub step_cursor: String,
    pub outline_to_work_item_id: BTreeMap<String, String>,
    pub outline_to_verification_plan_id: BTreeMap<String, String>,
    pub created_work_item_ids: Vec<String>,
    pub created_verification_plan_ids: Vec<String>,
    pub child_session_ids: Vec<String>,
    pub validator_findings: Vec<WorkItemSplitFinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abort_requested_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub previous_plan_snapshot: IssueWorkItemPlan,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanCompileStatus {
    Preparing,
    Validating,
    Committing,
    Committed,
    Failed,
    RecoveryRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanCommitState {
    NotStarted,
    Committed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OutlineContextIndex {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub generation_round_id: String,
    pub blocker_resolutions: Vec<OutlineContextBlockerResolution>,
    pub design_context_gaps: Vec<String>,
    pub design_context_capabilities: DesignContextCapabilities,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DesignContextCapabilities {
    pub has_architecture: bool,
    pub has_module_breakdown: bool,
    pub has_tech_stack: bool,
    pub has_test_strategy: bool,
    pub has_key_paths: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OutlineContextBlockerResolution {
    pub blocker_node_id: String,
    pub resolution_node_id: String,
    pub resolution_artifact_ref: String,
    pub estimated_tokens: u32,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SpecVersionRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub version: u32,
    pub markdown: String,
    pub provider_run_refs: Vec<String>,
    pub review_refs: Vec<String>,
    pub confirmed_by: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: WorkspaceType,
    pub status: WorkspaceSessionStatus,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    #[serde(default)]
    pub provider_conversations: Vec<ProviderConversationRef>,
    pub messages: Vec<WorkspaceMessageRecord>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceMessageRecord {
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub name: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub artifact_id: String,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Author,
    Reviewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionEvent {
    pub request_id: String,
    pub request: serde_json::Value,
    pub response: Option<serde_json::Value>,
    pub ts: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDetail {
    pub node_id: String,
    pub session_id: String,
    pub node_type: TimelineNodeType,
    pub status: TimelineNodeStatus,
    pub agent_role: Option<AgentRole>,
    pub provider: Option<ProviderSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    pub messages: Vec<serde_json::Value>,
    pub streaming_content: String,
    pub execution_events: Vec<serde_json::Value>,
    pub permission_events: Vec<PermissionEvent>,
    pub verdict: Option<serde_json::Value>,
    pub artifact_ref: Option<ArtifactRef>,
    pub is_revision: bool,
    pub base_artifact_ref: Option<ArtifactRef>,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderReviewRoundRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub session_id: String,
    pub round_index: u32,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_result: String,
    pub revision_result: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectProviderDefaultsRecord {
    pub project_id: String,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

    #[test]
    fn node_detail_roundtrip() {
        let detail = NodeDetail {
            node_id: "node-1".to_string(),
            session_id: "sess-1".to_string(),
            node_type: TimelineNodeType::AuthorRun,
            status: TimelineNodeStatus::Completed,
            agent_role: Some(AgentRole::Author),
            provider: Some(ProviderSnapshot {
                name: "claude_code".to_string(),
                model: "claude-opus-4-7".to_string(),
            }),
            prompt: Some("Workspace 类型: Story Spec".to_string()),
            messages: vec![],
            streaming_content: "输出内容".to_string(),
            execution_events: vec![],
            permission_events: vec![PermissionEvent {
                request_id: "perm-1".to_string(),
                request: serde_json::json!({"tool": "shell"}),
                response: Some(serde_json::json!({"approved": true})),
                ts: "2026-05-20T14:35:00Z".to_string(),
            }],
            verdict: None,
            artifact_ref: Some(ArtifactRef {
                artifact_id: "art-1".to_string(),
                version: 2,
            }),
            is_revision: false,
            base_artifact_ref: None,
            started_at: "2026-05-20T14:30:00Z".to_string(),
            ended_at: Some("2026-05-20T14:35:00Z".to_string()),
        };

        let json = serde_json::to_value(&detail).unwrap();
        let back: NodeDetail = serde_json::from_value(json).unwrap();

        assert_eq!(back.node_id, detail.node_id);
        assert_eq!(back.prompt, detail.prompt);
        assert_eq!(back.permission_events.len(), 1);
    }
}
