use serde::{Deserialize, Deserializer, Serialize};

use super::execution::CodingExecutionStage;
use crate::product::models::{PlanDefectClass, PlanDefectRoute, RepairTarget};
use crate::product::plan_repair::PlanDefectConfidence;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRequestKind {
    GitBranchOnly,
    GitlabMergeRequest,
    GithubPullRequest,
    ManualExternalRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteKind {
    Github,
    Gitlab,
    GenericGit,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushStatus {
    NotPushed,
    Pushed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRequest {
    pub id: String,
    pub attempt_id: String,
    pub kind: ReviewRequestKind,
    pub remote_kind: RemoteKind,
    pub remote: String,
    pub base_branch: String,
    pub branch_name: String,
    pub commit_sha: String,
    pub push_status: PushStatus,
    pub external_url: Option<String>,
    pub manual_instructions: Vec<String>,
    #[serde(default)]
    pub push_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Approve,
    RequestChanges,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Error,
    Warning,
    Info,
}

impl<'de> Deserialize<'de> for FindingSeverity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.trim().to_ascii_lowercase().as_str() {
            "error" | "blocked" | "blocker" | "blocking" | "critical" | "high" | "must_fix" => {
                Ok(Self::Error)
            }
            "warning" | "medium" | "strong_recommend_fix" | "suggestion" => Ok(Self::Warning),
            "info" | "low" | "minor" | "optional" => Ok(Self::Info),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &[
                    "error",
                    "blocked",
                    "warning",
                    "info",
                    "blocker",
                    "blocking",
                    "critical",
                    "high",
                    "medium",
                    "low",
                    "must_fix",
                    "strong_recommend_fix",
                    "suggestion",
                    "minor",
                    "optional",
                ],
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub severity: FindingSeverity,
    pub file_path: Option<String>,
    pub line: Option<u32>,
    pub message: String,
    pub required_action: Option<String>,
    #[serde(default = "default_review_finding_source_stage")]
    pub source_stage: CodingExecutionStage,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_defect_evidence: Vec<crate::product::models::PlanDefectEvidence>,
    #[serde(default)]
    pub related_requirements: Vec<String>,
    #[serde(default)]
    pub related_design_constraints: Vec<String>,
    #[serde(default)]
    pub related_work_item_tasks: Vec<String>,
    #[serde(default = "default_review_finding_defect_class")]
    pub defect_class: PlanDefectClass,
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub contract_refs: Vec<String>,
    #[serde(default)]
    pub capability_refs: Vec<String>,
    #[serde(default)]
    pub repair_target: Option<RepairTarget>,
    #[serde(default = "default_review_finding_route")]
    pub recommended_route: PlanDefectRoute,
    #[serde(default)]
    pub confidence: Option<PlanDefectConfidence>,
}

fn default_review_finding_source_stage() -> CodingExecutionStage {
    CodingExecutionStage::CodeReview
}

fn default_review_finding_defect_class() -> PlanDefectClass {
    PlanDefectClass::ImplementationDefect
}

fn default_review_finding_route() -> PlanDefectRoute {
    PlanDefectRoute::CoderRework
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeReviewReport {
    pub id: String,
    pub attempt_id: String,
    pub round: u32,
    pub verdict: ReviewVerdict,
    pub findings: Vec<ReviewFinding>,
    pub tested_evidence_refs: Vec<String>,
    pub diff_refs: Vec<String>,
    pub summary: String,
    pub created_at: String,
    #[serde(default)]
    pub raw_provider_output_ref: Option<String>,
    #[serde(default)]
    pub role_run_id: Option<String>,
    #[serde(default)]
    pub run_no: Option<u32>,
    #[serde(default)]
    pub unit_run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternalPrReview {
    pub id: String,
    pub attempt_id: String,
    pub review_request_id: String,
    pub verdict: ReviewVerdict,
    pub findings: Vec<ReviewFinding>,
    pub impact_scope: Vec<String>,
    pub pr_description: String,
    pub commit_message_suggestion: String,
    pub tested_evidence_refs: Vec<String>,
    pub diff_refs: Vec<String>,
    pub summary: String,
    pub created_at: String,
    #[serde(default)]
    pub raw_provider_output_ref: Option<String>,
    #[serde(default)]
    pub role_run_id: Option<String>,
    #[serde(default)]
    pub run_no: Option<u32>,
}
