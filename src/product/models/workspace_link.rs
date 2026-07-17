use serde::{Deserialize, Serialize};

use crate::web::workspace_ws_types::TimelineNode;

use super::{PlanAmendmentManifest, PlanProjectionBundle, PlanRepairRequest};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSessionLink {
    pub id: String,
    pub relation: WorkspaceSessionRelation,
    pub parent_session_id: String,
    pub child_session_id: String,
    pub trigger: WorkspaceSessionLinkTrigger,
    pub return_context: WorkspaceReturnContext,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSessionLinkTrigger {
    pub attempt_id: String,
    pub unit_run_id: String,
    pub review_id: Option<String>,
    pub finding_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceReturnContext {
    pub original_attempt_id: String,
    pub original_unit_run_id: String,
    pub timeline_anchor_id: String,
    pub original_route: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSessionRelation {
    PlanRepair,
    StoryAmendment,
    DesignAmendment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRepairSessionStage {
    Triaging,
    AuthoringRevision,
    ValidatingContract,
    GeneratingProjections,
    PlanReview,
    AwaitingConfirmation,
    Published,
    AmendmentConflict,
    ApplyingAmendment,
    AmendmentApplyFailed,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRepairSessionSnapshotDto {
    pub request: PlanRepairRequest,
    pub link: WorkspaceSessionLink,
    pub stage: PlanRepairSessionStage,
    pub projection: Option<PlanProjectionBundle>,
    pub amendment: Option<PlanAmendmentManifest>,
    pub timeline_nodes: Vec<TimelineNode>,
    pub error: Option<String>,
}
