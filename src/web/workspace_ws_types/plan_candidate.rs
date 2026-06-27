use serde::{Deserialize, Serialize};

/// Complete candidate produced by the work item plan author flow.
///
/// Carries the draft plan, proposed work items, verification plans, repository
/// profile and validator findings. WP2b generates this DTO; WP2a embeds it into
/// `ArtifactPayload::WorkItemPlanCandidate` and the workspace session artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanCandidateDto {
    pub plan: WorkItemPlanDto,
    pub work_items: Vec<WorkItemCandidateDto>,
    pub verification_plans: Vec<VerificationPlanDto>,
    pub repository_profile: Option<RepositoryProfileDto>,
    pub validator_findings: Vec<ValidatorFindingDto>,
}

/// Core work item plan metadata sent over the websocket.
///
/// Mirrors the HTTP `IssueWorkItemPlan`/`IssueWorkItemPlanDetailDto` shape but
/// omits issue/project IDs that are already present in the session context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanDto {
    pub id: String,
    pub status: String,
    pub options: WorkItemSplitOptionsDto,
    pub dependency_graph: Vec<WorkItemDependencyEdgeDto>,
}

/// Split options controlling how work items are generated from a plan.
///
/// Consumed by WP2b to decide whether to include integration/e2e tests, force
/// a frontend/backend split, or require explicit confirmation of the execution
/// plan before coding begins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemSplitOptionsDto {
    pub include_integration_tests: bool,
    pub include_e2e_tests: bool,
    pub force_frontend_backend_split: bool,
    pub require_execution_plan_confirm: bool,
}

/// Directed edge in the work item dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemDependencyEdgeDto {
    pub from_work_item_id: String,
    pub to_work_item_id: String,
}

/// A single proposed work item inside a `WorkItemPlanCandidateDto`.
///
/// WP2b produces these candidates; WP3 review and WP4 revision/revert may
/// update `meta.reverted`/`meta.revert_feedback` before WP5 confirms the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemCandidateDto {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub depends_on: Vec<String>,
    pub exclusive_write_scopes: Vec<String>,
    pub verification_plan_ref: Option<String>,
    pub meta: WorkItemCandidateMetaDto,
}

/// Mutable metadata attached to a `WorkItemCandidateDto`.
///
/// Tracks revert state and feedback generated during WP3/WP4 review cycles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemCandidateMetaDto {
    #[serde(default)]
    pub reverted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revert_feedback: Option<String>,
}

/// Validation finding produced when the candidate plan is checked for
/// consistency, scope conflicts, or missing verification coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ValidatorFindingDto {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub work_item_ids: Vec<String>,
}

/// Verification plan for one or more work items in the candidate.
///
/// WP2b generates these plans; WP5/execution uses them to gate completion of
/// the associated work items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerificationPlanDto {
    pub plan_ref: String,
    pub scope: String,
    pub commands: Vec<VerificationCommandDto>,
    pub manual_checks: Vec<VerificationManualCheckDto>,
    pub required_gates: Vec<String>,
    pub risk_notes: Vec<String>,
    pub confidence: String,
    pub fallback_policy: String,
}

/// Automated command that must pass to satisfy a `VerificationPlanDto`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerificationCommandDto {
    pub label: String,
    pub command: String,
    pub cwd: String,
    pub purpose: String,
    pub required: bool,
    pub timeout_seconds: u64,
    pub safety: String,
}

/// Manual check that must be performed to satisfy a `VerificationPlanDto`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerificationManualCheckDto {
    pub label: String,
    pub instructions: String,
    pub required: bool,
}

/// Repository profile used by the work item plan generator.
///
/// Contains detected languages, frameworks, layers and split recommendations.
/// The WS shape carries more detail than the HTTP `RepositoryProfile` so the
/// frontend can render the profile card without an extra REST round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryProfileDto {
    pub profile_id: String,
    pub repository_id: String,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub package_managers: Vec<String>,
    pub test_frameworks: Vec<String>,
    pub build_systems: Vec<String>,
    pub detected_layers: Vec<String>,
    pub split_recommendation: String,
    pub confidence: String,
}
