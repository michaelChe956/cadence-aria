use serde::{Deserialize, Serialize};

use super::{ReviewFinding, ReviewVerdict};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitReviewConclusionSnapshot {
    pub attempt_id: String,
    pub unit_id: String,
    pub unit_run_id: String,
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub code_review_report_id: String,
    pub verdict: ReviewVerdict,
    pub finding_digest: Vec<CompactFindingDigest>,
    pub evidence_refs: Vec<String>,
    pub diff_refs: Vec<String>,
    pub raw_report_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactFindingDigest {
    pub defect_class: Option<String>,
    pub reason_code: Option<String>,
    pub severity: String,
    pub message_digest: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotRebuildError {
    #[error("report_missing_unit_run_id: {0}")]
    MissingUnitRunId(String),
    #[error("store_error: {0}")]
    Store(#[from] crate::product::json_store::ProductStoreError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupReviewArtifactRef {
    pub id: String,
    pub raw_provider_output_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupReviewShardReport {
    pub id: String,
    pub attempt_id: String,
    pub snapshot_hash: String,
    pub shard_id: String,
    pub ordered_unit_run_ids: Vec<String>,
    pub partition_rationale: Vec<String>,
    pub verdict: ReviewVerdict,
    pub findings: Vec<ReviewFinding>,
    pub unresolved_obligations: Vec<GroupReviewObligation>,
    pub selected_diff_refs: Vec<String>,
    pub raw_provider_output_refs: Vec<String>,
    pub role_run_ids: Vec<String>,
    pub run_failure_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupReviewReductionReport {
    pub id: String,
    pub attempt_id: String,
    pub snapshot_hash: String,
    pub shard_report_ids: Vec<String>,
    pub verdict: ReviewVerdict,
    pub findings: Vec<ReviewFinding>,
    pub impact_scope: Vec<String>,
    pub pr_description: String,
    pub commit_message_suggestion: String,
    pub provenance: Vec<ReviewProvenance>,
    pub raw_provider_output_refs: Vec<String>,
    pub role_run_ids: Vec<String>,
    pub run_failure_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupReviewObligation {
    pub obligation_id: String,
    pub kind: String,
    pub related_unit_run_ids: Vec<String>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewProvenance {
    pub source_kind: String,
    pub source_id: String,
    pub finding_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CasOutcome {
    Written,
    StoredStale,
}
