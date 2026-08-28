use serde::{Deserialize, Serialize};

use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::product::work_item_plan_policy::{
    HumanGateSnapshot, PolicyDiagnostic, ProviderStartLedgerEntry, RepairReservation,
    ReviewInvocationScope, RunHistory, RunPolicy, WorkItemPlanFlowKind,
};
use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

use super::provider::{ProviderConversationRef, ProviderName};
use super::work_item_revision::WorkItemRuntimeBinding;

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
    StoppedNeedsHuman,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceRolePermissionModes {
    pub author: ProviderPermissionMode,
    pub reviewer: ProviderPermissionMode,
}

impl Default for WorkspaceRolePermissionModes {
    fn default() -> Self {
        Self {
            author: ProviderPermissionMode::Auto,
            reviewer: ProviderPermissionMode::Auto,
        }
    }
}

fn default_flow_kind() -> WorkItemPlanFlowKind {
    WorkItemPlanFlowKind::Legacy
}

fn default_run_policy() -> RunPolicy {
    RunPolicy::Interactive
}

/// 单候选路径的 durable 阶段。旧 session 缺失该字段时维持 legacy 语义。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SingleCandidatePhase {
    Generate,
    Evaluate,
    Approval,
    Compile,
}

/// Approval 成功后确定性派生的编译三元组；同一 session 只能持久化同一值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingleCandidateCompileReservation {
    pub compile_id: String,
    pub now: String,
    pub publication_provenance_ref: String,
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
    #[serde(default)]
    pub permission_modes: WorkspaceRolePermissionModes,
    #[serde(default)]
    pub provisional_reviewer_provider: Option<ProviderName>,
    #[serde(default)]
    pub reviewer_enabled_at_start: Option<bool>,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    #[serde(default = "default_flow_kind")]
    pub flow_kind: WorkItemPlanFlowKind,
    #[serde(default = "default_run_policy")]
    pub run_policy: RunPolicy,
    #[serde(default)]
    pub run_history: RunHistory,
    #[serde(default)]
    pub review_invocation_scope: Option<ReviewInvocationScope>,
    #[serde(default)]
    pub human_gate_snapshot: Option<HumanGateSnapshot>,
    #[serde(default)]
    pub repair_reservation: Option<RepairReservation>,
    #[serde(default)]
    pub policy_diagnostics: Vec<PolicyDiagnostic>,
    #[serde(default)]
    pub provider_start_ledger: Vec<ProviderStartLedgerEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub single_candidate_phase: Option<SingleCandidatePhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_plan_source_revision_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_candidate_ir_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mechanical_report_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile_reservation: Option<SingleCandidateCompileReservation>,
    #[serde(default)]
    pub work_item_runtime_binding: Option<WorkItemRuntimeBinding>,
    #[serde(default)]
    pub provider_conversations: Vec<ProviderConversationRef>,
    pub messages: Vec<WorkspaceMessageRecord>,
    pub created_at: String,
    pub updated_at: String,
}

/// Durable audit event that associates an immutable stopped auto run with the
/// interactive session created by an explicit human takeover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanGateTakeoverEvent {
    pub id: String,
    pub parent_session_id: String,
    pub child_session_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionSummaryRecord {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_feedback: Option<String>,
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

    #[test]
    fn workspace_role_permission_modes_default_is_auto() {
        let modes = WorkspaceRolePermissionModes::default();
        assert_eq!(modes.author, ProviderPermissionMode::Auto);
        assert_eq!(modes.reviewer, ProviderPermissionMode::Auto);
    }

    #[test]
    fn old_workspace_session_record_without_permission_modes_deserializes_to_auto() {
        let json = serde_json::json!({
            "id": "s1", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
            "workspace_type": "story", "status": "open",
            "author_provider": "claude_code", "reviewer_provider": "codex",
            "review_rounds": 1, "superpowers_enabled": false, "openspec_enabled": false,
            "messages": [], "created_at": "", "updated_at": ""
        });
        let record: WorkspaceSessionRecord = serde_json::from_value(json).unwrap();
        assert_eq!(record.permission_modes.author, ProviderPermissionMode::Auto);
        assert_eq!(
            record.permission_modes.reviewer,
            ProviderPermissionMode::Auto
        );
    }

    #[test]
    fn old_workspace_session_record_defaults_work_item_plan_policy_fields() {
        let json = serde_json::json!({
            "id": "s1", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
            "workspace_type": "work_item_plan", "status": "open",
            "author_provider": "claude_code", "reviewer_provider": "codex",
            "review_rounds": 1, "superpowers_enabled": false, "openspec_enabled": false,
            "messages": [], "created_at": "", "updated_at": ""
        });

        let record: WorkspaceSessionRecord = serde_json::from_value(json).unwrap();

        assert_eq!(
            record.flow_kind,
            crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy
        );
        assert_eq!(
            record.run_policy,
            crate::product::work_item_plan_policy::RunPolicy::Interactive
        );
        assert_eq!(
            record.run_history,
            crate::product::work_item_plan_policy::RunHistory::default()
        );
    }

    #[test]
    fn workspace_session_status_serializes_new_terminal_values() {
        assert_eq!(
            serde_json::to_value(WorkspaceSessionStatus::StoppedNeedsHuman).unwrap(),
            "stopped_needs_human"
        );
        assert_eq!(
            serde_json::to_value(WorkspaceSessionStatus::Failed).unwrap(),
            "failed"
        );
    }
}
