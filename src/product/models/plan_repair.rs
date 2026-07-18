use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::product::work_item_contract::DependencyContractEdge;

use super::{
    DependencyGraphRevision, LogicalWorkItem, PlanProjectionBundle, PlanValidationReportArtifact,
    VerificationPlanRevision, WorkItemDraftRevision, WorkItemPlanLineage, WorkItemPlanRevision,
    WorkItemProjectionBundle, WorkItemRevision,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDefectClass {
    ImplementationDefect,
    VerificationIncomplete,
    CurrentWorkItemInvalid,
    UpstreamContractInvalid,
    DependencyGraphInvalid,
    DesignAmendmentRequired,
    StoryAmendmentRequired,
    OperationalBlocker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDefectRoute {
    CoderRework,
    VerificationRetry,
    PlanRepair,
    StoryAmendment,
    DesignAmendment,
    OperationalGate,
    HumanTriage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairTargetKind {
    CurrentWorkItem,
    UpstreamWorkItem,
    Subgraph,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairTarget {
    pub kind: RepairTargetKind,
    pub logical_work_item_ids: Vec<String>,
    pub work_item_revision_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefectEvidence {
    pub kind: String,
    pub source_ref: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRepairRequestStatus {
    Open,
    InProgress,
    AwaitingConfirmation,
    Published,
    Applied,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRepairRequest {
    pub id: String,
    pub plan_id: String,
    pub base_plan_revision_id: String,
    pub trigger_attempt_id: String,
    pub trigger_unit_run_id: String,
    pub trigger_review_id: Option<String>,
    pub trigger_finding_id: String,
    pub amendment_id: Option<String>,
    pub defect_class: PlanDefectClass,
    pub reason_code: String,
    pub repair_target: RepairTarget,
    pub contract_refs: Vec<String>,
    pub capability_refs: Vec<String>,
    pub evidence: Vec<PlanDefectEvidence>,
    pub fingerprint: String,
    pub status: PlanRepairRequestStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractDeltaKind {
    InformativeOnly,
    ImplementationGuidance,
    CompatibleContractExtension,
    BreakingContractChange,
    TopologyChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemRevisionReplacement {
    pub previous_revision_id: String,
    pub next_revision_id: String,
    pub delta_kind: ContractDeltaKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyGraphChangeKind {
    EdgeAdded,
    EdgeRemoved,
    EdgeReplaced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyGraphChange {
    pub kind: DependencyGraphChangeKind,
    pub previous: Option<DependencyContractEdge>,
    pub next: Option<DependencyContractEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmendmentResumeMode {
    Reexecute,
    Revalidate,
    AwaitHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmendmentResumeTarget {
    pub logical_work_item_id: String,
    pub mode: AmendmentResumeMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmendmentManifest {
    pub id: String,
    pub repair_request_id: String,
    pub previous_plan_revision_id: String,
    pub new_plan_revision_id: String,
    pub revised_work_items: BTreeMap<String, WorkItemRevisionReplacement>,
    pub superseded_revisions: Vec<String>,
    pub dependency_graph_changes: Vec<DependencyGraphChange>,
    pub contract_deltas: Vec<crate::product::plan_repair::ContractDelta>,
    pub unaffected_units: Vec<String>,
    pub revalidation_required_units: Vec<String>,
    pub stale_units: Vec<String>,
    pub replacement_units: BTreeMap<String, Vec<String>>,
    pub resume_target: AmendmentResumeTarget,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmendmentConfirmation {
    pub amendment_id: String,
    pub base_plan_revision_id: String,
    pub accepted_impact_scope: Vec<String>,
    pub risk_acceptance_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_attestation_id: Option<String>,
    pub confirmed_by: String,
    pub confirmed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanAmendmentPublicationPhase {
    Preparing,
    Prepared,
    PlanPublished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmendmentWorkItemArtifacts {
    pub logical_work_item: LogicalWorkItem,
    pub draft_revision: WorkItemDraftRevision,
    pub work_item_revision: WorkItemRevision,
    pub verification_plan_revision: VerificationPlanRevision,
    pub projection_bundle: WorkItemProjectionBundle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmendmentPublicationSnapshot {
    pub lineage: WorkItemPlanLineage,
    pub plan_revision: WorkItemPlanRevision,
    pub dependency_graph_revision: DependencyGraphRevision,
    pub validation_report: PlanValidationReportArtifact,
    pub plan_projection_bundle: PlanProjectionBundle,
    pub work_items: Vec<PlanAmendmentWorkItemArtifacts>,
    pub manifest: PlanAmendmentManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmendmentPublicationJournal {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub amendment_id: String,
    pub request_id: String,
    pub base_plan_revision_id: String,
    pub new_plan_revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<PlanAmendmentConfirmation>,
    pub artifact_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<PlanAmendmentPublicationSnapshot>,
    pub phase: PlanAmendmentPublicationPhase,
    pub error: Option<String>,
    pub recovery: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
