use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdictType {
    Pass,
    Revise,
    NeedsHuman,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFindingSeverity {
    Blocking,
    MustFix,
    StrongRecommendFix,
    Suggestion,
    Minor,
    Optional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub severity: ReviewFindingSeverity,
    pub message: String,
    pub evidence: String,
    pub impact: String,
    pub required_action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewGate {
    RequiresRevision,
    UserConfirmAllowed,
    UserTriageRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanReviewVerdict {
    Pass,
    Revise,
    ReviseBatch,
    NeedsHuman,
    PlanReopenRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanReviewScope {
    Outline,
    Item,
    Batch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanReviewAction {
    Continue,
    ReviseOutline,
    ReviseCurrentItem,
    ReviseBatch,
    HumanTriage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanReviewGate {
    RequiresCurrentItemRevision,
    RequiresBatchRevision,
    RequiresPlanReopen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanReviewAffectedItem {
    pub outline_index: Option<u32>,
    pub target_outline_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanReviewComplete {
    pub verdict: WorkItemPlanReviewVerdict,
    pub review_scope: WorkItemPlanReviewScope,
    pub target_outline_id: Option<String>,
    pub generation_round_id: String,
    pub draft_id: Option<String>,
    pub batch_id: Option<String>,
    pub review_action: WorkItemPlanReviewAction,
    pub gates: Vec<WorkItemPlanReviewGate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affects_items: Vec<WorkItemPlanReviewAffectedItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewVerdict {
    pub verdict: ReviewVerdictType,
    pub comments: String,
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    #[serde(default = "default_review_gate")]
    pub review_gate: ReviewGate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_plan_review: Option<WorkItemPlanReviewComplete>,
}

fn default_review_gate() -> ReviewGate {
    ReviewGate::UserConfirmAllowed
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDecision {
    pub decision: String,
    pub extra_context: Option<String>,
    pub decided_at: String,
}
