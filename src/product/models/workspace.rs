use serde::{Deserialize, Serialize};

use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
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
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    #[serde(default)]
    pub work_item_runtime_binding: Option<WorkItemRuntimeBinding>,
    #[serde(default)]
    pub provider_conversations: Vec<ProviderConversationRef>,
    pub messages: Vec<WorkspaceMessageRecord>,
    pub created_at: String,
    pub updated_at: String,
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
}
