use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingAttemptPlanBinding {
    pub attempt_id: String,
    pub plan_id: String,
    pub bound_plan_revision_id: String,
    pub applied_amendment_ids: Vec<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingUnitRunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Blocked,
    BlockedByPlanDefect,
    AwaitingAmendment,
    NeedsRevalidation,
    Stale,
    Superseded,
}

impl CodingUnitRunStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Pending
                | Self::Running
                | Self::Blocked
                | Self::BlockedByPlanDefect
                | Self::AwaitingAmendment
                | Self::NeedsRevalidation
                | Self::Stale
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingUnitRun {
    pub id: String,
    pub unit_id: String,
    pub execution_no: u32,
    pub work_item_revision_id: String,
    pub resolved_handoff_revision_ids: Vec<String>,
    pub canonical_contract_hash: String,
    pub projection_bundle_id: String,
    pub projection_compiler_version: String,
    pub coder_provider_renderer_version: String,
    pub reviewer_provider_renderer_version: String,
    pub internal_reviewer_provider_renderer_version: Option<String>,
    pub coder_projection_hash: String,
    pub reviewer_projection_hash: String,
    pub coder_execution_context_hash: Option<String>,
    pub reviewer_execution_context_hash: Option<String>,
    pub internal_reviewer_execution_context_hash: Option<String>,
    pub status: CodingUnitRunStatus,
    pub unit_rework_count: u32,
    pub verification_retry_count: u32,
    pub operational_retry_count: u32,
    pub plan_repair_count: u32,
    pub start_commit: Option<String>,
    pub completion_commit: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAmendmentApplicationPhase {
    Started,
    PlanBindingWritten,
    UnitRunsWritten,
    ResumeTargetWritten,
    Completed,
}

impl CodingAmendmentApplicationPhase {
    pub fn order(&self) -> u8 {
        match self {
            Self::Started => 0,
            Self::PlanBindingWritten => 1,
            Self::UnitRunsWritten => 2,
            Self::ResumeTargetWritten => 3,
            Self::Completed => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingAmendmentApplicationJournal {
    pub id: String,
    pub attempt_id: String,
    pub amendment_id: String,
    pub materialization_head_commit: Option<String>,
    pub phase: CodingAmendmentApplicationPhase,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingPlanAmendmentDeliveryStatus {
    Pending,
    Delivered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingPlanAmendmentDelivery {
    pub id: String,
    pub event_id: String,
    pub attempt_id: String,
    pub amendment_id: String,
    pub status: CodingPlanAmendmentDeliveryStatus,
    pub delivered_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
