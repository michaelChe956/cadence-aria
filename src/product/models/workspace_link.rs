use serde::{Deserialize, Serialize};

use crate::product::plan_repair::ContractImpactReport;
use crate::web::workspace_ws_types::TimelineNode;
use crate::web::workspace_ws_types::WorkItemPlanReviewComplete;

use super::{
    PlanAmendmentManifest, PlanProjectionBundle, PlanRepairRequest, PlanValidationReportArtifact,
};

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
    #[serde(default)]
    pub repair_request_id: String,
    #[serde(default)]
    pub amendment_id: String,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub base_plan_revision_id: String,
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
pub struct PlanRepairPackageIdentity {
    pub request_id: String,
    pub amendment_id: String,
    pub plan_id: String,
    pub base_plan_revision_id: String,
    pub next_plan_revision_id: String,
    pub projection_bundle_id: String,
    pub validation_report_id: String,
    pub reviewed_plan_revision_id: String,
    pub review_generation_round_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRepairAwaitingConfirmationPackage {
    pub package_identity: PlanRepairPackageIdentity,
    pub projection: PlanProjectionBundle,
    pub amendment: PlanAmendmentManifest,
    pub validation: PlanValidationReportArtifact,
    pub impact: ContractImpactReport,
    pub plan_review: WorkItemPlanReviewComplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRepairSessionSnapshotDto {
    pub request: PlanRepairRequest,
    pub link: WorkspaceSessionLink,
    pub stage: PlanRepairSessionStage,
    pub projection: Option<PlanProjectionBundle>,
    pub amendment: Option<PlanAmendmentManifest>,
    #[serde(default)]
    pub validation: Option<PlanValidationReportArtifact>,
    #[serde(default)]
    pub impact: Option<ContractImpactReport>,
    #[serde(default)]
    pub plan_review: Option<WorkItemPlanReviewComplete>,
    #[serde(default)]
    pub package_identity: Option<PlanRepairPackageIdentity>,
    pub timeline_nodes: Vec<TimelineNode>,
    pub error: Option<String>,
}
