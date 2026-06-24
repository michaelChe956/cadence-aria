use serde::{Deserialize, Serialize};

use super::execution::{CodingExecutionStage, CodingProviderRole};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingRoleRunStatus {
    Running,
    Completed,
    Failed,
    Blocked,
    Superseded,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingRoleRunTrigger {
    Initial,
    RetryTestPlan,
    RerunMissingSteps,
    RetryReview,
    RetryAnalyst,
    RetryInternalReview,
    ManualRerun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingRoleRun {
    pub id: String,
    pub attempt_id: String,
    pub stage: CodingExecutionStage,
    pub role: CodingProviderRole,
    pub run_no: u32,
    pub status: CodingRoleRunStatus,
    pub trigger: CodingRoleRunTrigger,
    pub node_id: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    #[serde(default)]
    pub supersedes_run_id: Option<String>,
    #[serde(default)]
    pub superseded_by_run_id: Option<String>,
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub raw_provider_output_refs: Vec<String>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingRoleRunEventType {
    ProviderPrompt,
    ProviderStart,
    TextDelta,
    ExecutionEvent,
    ToolCall,
    ToolResult,
    StatusChanged,
    PermissionRequest,
    ChoiceRequest,
    MessageComplete,
    ProviderFailed,
    Timeout,
    Aborted,
    PersistenceWarning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingRoleRunEvent {
    pub attempt_id: String,
    pub role_run_id: String,
    pub node_id: Option<String>,
    pub stage: CodingExecutionStage,
    pub role: CodingProviderRole,
    pub sequence: u64,
    pub event_type: CodingRoleRunEventType,
    pub created_at: String,
    pub payload: serde_json::Value,
    pub truncated: bool,
    pub artifact_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingRoleRunEventSummary {
    pub event_count: usize,
    pub last_event_at: Option<String>,
    pub last_event_type: Option<CodingRoleRunEventType>,
    pub last_event_title: Option<String>,
    pub last_event_status: Option<String>,
    pub terminal_event_type: Option<CodingRoleRunEventType>,
    pub terminal_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingRoleRunEventPreview {
    pub sequence: u64,
    pub event_type: CodingRoleRunEventType,
    pub created_at: String,
    pub title: Option<String>,
    pub status: Option<String>,
    pub detail: Option<String>,
    pub truncated: bool,
    pub artifact_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingRoleRunSnapshot {
    #[serde(flatten)]
    pub run: CodingRoleRun,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_summary: Option<CodingRoleRunEventSummary>,
    #[serde(default)]
    pub recent_events: Vec<CodingRoleRunEventPreview>,
}

impl std::ops::Deref for CodingRoleRunSnapshot {
    type Target = CodingRoleRun;

    fn deref(&self) -> &Self::Target {
        &self.run
    }
}
