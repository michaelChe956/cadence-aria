use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupDependencyGateStatus {
    Ready,
    Waiting,
    FailedClosed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupDependencyGateSnapshot {
    pub attempt_id: String,
    pub status: GroupDependencyGateStatus,
    pub selected_unit_id: Option<String>,
    pub pending_unit_ids: Vec<String>,
    pub reason_code: Option<String>,
    pub message: Option<String>,
    pub plan_revision_id: String,
    pub created_at: String,
}
