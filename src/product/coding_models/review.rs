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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRequestOwnerKind {
    #[default]
    Attempt,
    PointerPublication,
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
    /// 仓库未配置目标远端（无 origin）：跳过推送，按本地交付收口。
    /// 仅表示「远端未配置」，绝不用于「远端已配置但推送失败」。
    SkippedNotConfigured,
}

impl PushStatus {
    /// 「已交付」统一谓词：远端已推送，或无远端仓库的本地交付。
    /// issue 级与 plan 组级交付聚合必须统一走本谓词，不得再直接比较 `Pushed`。
    pub fn is_delivered(&self) -> bool {
        matches!(self, PushStatus::Pushed | PushStatus::SkippedNotConfigured)
    }
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
    #[serde(default)]
    pub owner_kind: ReviewRequestOwnerKind,
    #[serde(default)]
    pub pointer_publication_id: Option<String>,
    /// revoke 补偿标记：`PointerPublication` 撤回后置 true。存量记录缺省 false，
    /// 不改变 Attempt 来源的既有语义。
    #[serde(default)]
    pub revoked: bool,
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

/// 报告采信投影 wire 值(T18)。与模型 verdict 独立:仅当 reviewer
/// 窗口经三域对账+人工门判定后才可能 `accepted`。缺字段 = 历史报告
/// 未核验(**不是** accepted);权威记录是 reviewer window store 的
/// `report_acceptance`,本字段只是同源投影。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewReportAcceptance {
    PendingAcceptance,
    Accepted,
    Invalidated,
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
    /// T18 采信投影(同源自窗口记录;缺字段 = 历史未核验,不默认
    /// Accepted,不改历史 JSON)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<ReviewReportAcceptance>,
    /// T18 report/window 绑定(产出该报告的 reviewer 窗口)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_window_id: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<ReviewReportAcceptance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_window_id: Option<String>,
}
