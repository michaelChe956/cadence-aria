use serde::{Deserialize, Serialize};

use super::execution::{CodingAgentRole, CodingExecutionStage};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingTimelineNodeStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingTimelineNode {
    pub id: String,
    pub attempt_id: String,
    pub stage: CodingExecutionStage,
    pub title: String,
    pub status: CodingTimelineNodeStatus,
    pub agent_role: Option<CodingAgentRole>,
    pub summary: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub artifact_refs: Vec<String>,
}
