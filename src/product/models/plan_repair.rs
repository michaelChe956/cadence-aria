use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
pub struct RepairTarget {
    pub kind: RepairTargetKind,
    pub logical_work_item_ids: Vec<String>,
    pub work_item_revision_ids: Vec<String>,
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
    pub evidence: Vec<serde_json::Value>,
    pub fingerprint: String,
    pub status: PlanRepairRequestStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemRevisionReplacement {
    pub previous_revision_id: String,
    pub next_revision_id: String,
    pub delta_kind: String,
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
    pub dependency_graph_changes: Vec<serde_json::Value>,
    pub contract_deltas: Vec<serde_json::Value>,
    pub unaffected_units: Vec<String>,
    pub revalidation_required_units: Vec<String>,
    pub stale_units: Vec<String>,
    pub replacement_units: BTreeMap<String, Vec<String>>,
    pub resume_target: AmendmentResumeTarget,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanAmendmentPublicationPhase {
    Prepared,
    PlanPublished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmendmentPublicationJournal {
    pub id: String,
    pub plan_id: String,
    pub amendment_id: String,
    pub phase: PlanAmendmentPublicationPhase,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
