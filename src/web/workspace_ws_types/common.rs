use serde::{Deserialize, Serialize};

use crate::product::models::ProviderName;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsPermissionRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsProviderStatus {
    Starting,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsExecutionEventKind {
    Provider,
    Turn,
    Command,
    Output,
    Artifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsExecutionEventStatus {
    Started,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsExecutionEvent {
    pub event_id: String,
    pub node_id: Option<String>,
    pub agent: Option<ProviderName>,
    pub kind: WsExecutionEventKind,
    pub status: WsExecutionEventStatus,
    pub title: String,
    pub detail: Option<String>,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub output: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDefaults {
    pub reviewer: ProviderName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsMessageDto {
    pub id: String,
    pub role: String,
    pub content: String,
    pub checkpoint_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsCheckpointDto {
    pub id: String,
    pub message_index: u32,
    pub stage: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsProviderConfig {
    pub author: ProviderName,
    pub reviewer: Option<ProviderName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfigSnapshot {
    pub author: ProviderName,
    pub reviewer: Option<ProviderName>,
    pub review_rounds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredFeedback {
    pub feedback_types: Vec<String>,
    pub description: String,
    pub target_artifact_version: Option<u32>,
}
