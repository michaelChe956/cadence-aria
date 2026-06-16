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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignKind {
    Frontend,
    Backend,
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
    pub design_kind: DesignKind,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationFallbackPolicy {
    ManualGate,
    RepairProviderOutput,
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
