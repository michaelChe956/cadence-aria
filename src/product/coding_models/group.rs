use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingExecutionUnitStatus {
    Pending,
    Running,
    WaitingForHuman,
    Completed,
    Failed,
    Blocked,
    BlockedByPlanDefect,
    AwaitingAmendment,
    NeedsRevalidation,
    Stale,
    Superseded,
    Skipped,
}

impl CodingExecutionUnitStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Running
                | Self::WaitingForHuman
                | Self::Blocked
                | Self::BlockedByPlanDefect
                | Self::AwaitingAmendment
                | Self::NeedsRevalidation
                | Self::Stale
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingExecutionUnit {
    pub id: String,
    pub attempt_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub dependency_logical_work_item_ids: Vec<String>,
    pub order_index: u32,
    pub status: CodingExecutionUnitStatus,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub latest_handoff_revision_id: Option<String>,
    pub completion_commit: Option<String>,
    pub summary: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
