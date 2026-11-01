use serde::{Deserialize, Serialize};

use crate::product::models::AmendmentResumeTarget;

/// Durable linkage between a group coding attempt paused by a plan defect and
/// the original SC plan session that hosts the sole human gate for the
/// amendment conversation (REQ-GCE-03).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmendmentContext {
    pub id: String,
    pub plan_session_id: String,
    pub group_attempt_id: String,
    pub trigger_unit_id: String,
    pub trigger_finding_id: String,
    pub previous_plan_revision_id: String,
    pub new_plan_revision_id: Option<String>,
    pub resume_target: AmendmentResumeTarget,
    pub status: PlanAmendmentContextStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanAmendmentContextStatus {
    Open,
    Applying,
    Applied,
    FailedClosed,
}
