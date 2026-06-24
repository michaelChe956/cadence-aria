use serde::{Deserialize, Serialize};

use crate::product::models::ProviderName;

use super::common::ProviderConfigSnapshot;
use super::stage::WorkspaceStage;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineNodeType {
    PrepareContext,
    ContextNote,
    StartGeneration,
    AuthorConfirm,
    #[serde(alias = "generation")]
    AuthorRun,
    #[serde(alias = "review")]
    ReviewerRun,
    ReviewDecision,
    Revision,
    HumanConfirm,
    WorkItemPlanOutlineRun,
    WorkItemPlanOutlineConfirm,
    WorkItemPlanOutlineReview,
    WorkItemPlanContextBlocker,
    WorkItemGenerationMode,
    WorkItemDraftRun,
    WorkItemDraftConfirm,
    WorkItemDraftReview,
    WorkItemBatchRun,
    WorkItemBatchConfirm,
    WorkItemBatchReview,
    WorkItemPlanCompile,
    WorkItemPlanCompileRecovery,
    AbortedByDisconnect,
    ProtocolError,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineNodeStatus {
    Active,
    Paused,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineNodeRetryError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineNodeRetry {
    pub retry_of_node_id: String,
    pub retry_attempt: u32,
    pub retry_reason: String,
    pub retry_error: TimelineNodeRetryError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineNode {
    pub node_id: String,
    pub node_type: TimelineNodeType,
    pub agent: Option<ProviderName>,
    pub stage: WorkspaceStage,
    pub round: Option<u32>,
    pub status: TimelineNodeStatus,
    pub title: String,
    pub summary: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub artifact_ref: Option<String>,
    pub provider_config_snapshot: ProviderConfigSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<TimelineNodeRetry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDetailSummary {
    pub node_id: String,
    pub node_type: String,
    pub status: String,
    pub agent_role: Option<String>,
    pub provider_name: Option<String>,
    pub prompt_size: usize,
    pub prompt_preview: Option<String>,
    pub stream_size: usize,
    pub stream_preview: Option<String>,
    pub execution_event_count: usize,
    pub has_large_outputs: bool,
    pub artifact_ref: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
}
