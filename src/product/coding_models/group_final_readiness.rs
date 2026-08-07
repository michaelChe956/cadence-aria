use serde::{Deserialize, Serialize};

use super::{ReviewFinding, ReviewVerdict};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GroupFinalReadinessStatus {
    Complete,
    #[default]
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupFinalReadinessDiagnosticKind {
    UnitRunMissing,
    CompletionCommitMissing,
    CodeReviewMissing,
    HandoffMissing,
    PlanBindingMismatch,
    IdentityMismatch,
}

impl Default for GroupFinalReadinessDiagnosticKind {
    fn default() -> Self {
        Self::UnitRunMissing
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GroupFinalReadinessDiagnostic {
    pub kind: GroupFinalReadinessDiagnosticKind,
    pub unit_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GroupFinalReadinessUnit {
    pub unit_id: String,
    pub logical_work_item_id: String,
    pub unit_run_id: String,
    pub start_commit: String,
    pub completion_commit: String,
    pub commit_shas: Vec<String>,
    pub diff_ref: String,
    pub code_review_report_id: Option<String>,
    pub review_verdict: Option<ReviewVerdict>,
    pub review_summary: Option<String>,
    pub review_findings: Option<Vec<ReviewFinding>>,
    pub review_raw_provider_output_ref: Option<String>,
    pub handoff_revision_id: Option<String>,
    pub plan_revision_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GroupFinalReadinessSnapshot {
    pub attempt_id: String,
    pub status: GroupFinalReadinessStatus,
    pub units: Vec<GroupFinalReadinessUnit>,
    pub diagnostics: Vec<GroupFinalReadinessDiagnostic>,
    pub created_at: String,
}
