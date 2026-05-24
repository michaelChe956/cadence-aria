use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::web::workspace_ws_types::ProviderConfigSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingExecutionStage {
    PrepareContext,
    WorktreePrepare,
    Coding,
    Testing,
    CodeReview,
    Rework,
    ReviewRequest,
    InternalPrReview,
    FinalConfirm,
}

impl CodingExecutionStage {
    pub fn order(&self) -> u8 {
        match self {
            Self::PrepareContext => 0,
            Self::WorktreePrepare => 1,
            Self::Coding => 2,
            Self::Testing => 3,
            Self::CodeReview => 4,
            Self::Rework => 5,
            Self::ReviewRequest => 6,
            Self::InternalPrReview => 7,
            Self::FinalConfirm => 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAttemptStatus {
    Created,
    Running,
    WaitingForHuman,
    Blocked,
    Completed,
    Failed,
    Aborted,
}

impl CodingAttemptStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Created | Self::Running | Self::WaitingForHuman | Self::Blocked
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingExecutionAttempt {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub attempt_no: u32,
    pub status: CodingAttemptStatus,
    pub stage: CodingExecutionStage,
    pub base_branch: String,
    pub branch_name: String,
    pub worktree_path: Option<PathBuf>,
    pub provider_config_snapshot: ProviderConfigSnapshot,
    pub rework_count: u32,
    pub max_auto_rework: u32,
    pub head_commit: Option<String>,
    pub pushed_remote: Option<String>,
    pub review_request_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestCommandStatus {
    Passed,
    Failed,
    TimedOut,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCommand {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout_ref: String,
    pub stderr_ref: String,
    pub status: TestCommandStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestingOverallStatus {
    Passed,
    Failed,
    SkippedByUserDecision,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestingReport {
    pub id: String,
    pub attempt_id: String,
    pub commands: Vec<TestCommand>,
    pub overall_status: TestingOverallStatus,
    pub provider_claim: Option<serde_json::Value>,
    pub backend_verified: bool,
    pub started_at: String,
    pub completed_at: Option<String>,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub severity: FindingSeverity,
    pub file_path: Option<String>,
    pub line: Option<u32>,
    pub message: String,
    pub required_action: Option<String>,
    pub source_stage: CodingExecutionStage,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternalPrReview {
    pub id: String,
    pub attempt_id: String,
    pub review_request_id: String,
    pub verdict: ReviewVerdict,
    pub findings: Vec<ReviewFinding>,
    pub tested_evidence_refs: Vec<String>,
    pub diff_refs: Vec<String>,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingTimelineNodeStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAgentRole {
    Author,
    Tester,
    Reviewer,
    Git,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingTimelineNode {
    pub id: String,
    pub attempt_id: String,
    pub stage: CodingExecutionStage,
    pub title: String,
    pub status: CodingTimelineNodeStatus,
    pub agent_role: Option<CodingAgentRole>,
    pub summary: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub artifact_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingGateAction {
    pub action_id: String,
    pub label: String,
    pub action_type: CodingGateActionType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingGateActionType {
    ContinueRework,
    AcceptRisk,
    Abort,
    RetryPush,
    ManualFix,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingGateKind {
    Permission,
    Blocked,
    FinalConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingGateRequired {
    pub gate_id: String,
    pub kind: CodingGateKind,
    pub title: String,
    pub description: String,
    pub available_actions: Vec<CodingGateAction>,
}
