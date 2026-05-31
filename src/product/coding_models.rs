use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

use crate::product::models::ProviderName;
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
#[serde(rename_all = "snake_case")]
pub enum CodingProviderRole {
    Coder,
    Tester,
    Analyst,
    CodeReviewer,
    InternalReviewer,
}

impl fmt::Display for CodingProviderRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Coder => "Coder",
            Self::Tester => "Tester",
            Self::Analyst => "Analyst",
            Self::CodeReviewer => "Code Reviewer",
            Self::InternalReviewer => "Internal Reviewer",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingRoleProviderConfigSnapshot {
    pub coder: ProviderName,
    pub tester: ProviderName,
    pub analyst: ProviderName,
    pub code_reviewer: ProviderName,
    pub internal_reviewer: ProviderName,
    pub review_rounds: u32,
}

impl From<ProviderConfigSnapshot> for CodingRoleProviderConfigSnapshot {
    fn from(snapshot: ProviderConfigSnapshot) -> Self {
        Self::from(&snapshot)
    }
}

impl From<&ProviderConfigSnapshot> for CodingRoleProviderConfigSnapshot {
    fn from(snapshot: &ProviderConfigSnapshot) -> Self {
        let reviewer = snapshot
            .reviewer
            .clone()
            .unwrap_or_else(|| snapshot.author.clone());
        Self {
            coder: snapshot.author.clone(),
            tester: snapshot.author.clone(),
            analyst: snapshot.author.clone(),
            code_reviewer: reviewer.clone(),
            internal_reviewer: reviewer,
            review_rounds: snapshot.review_rounds,
        }
    }
}

impl CodingRoleProviderConfigSnapshot {
    pub fn provider_for_role(&self, role: &CodingProviderRole) -> &ProviderName {
        match role {
            CodingProviderRole::Coder => &self.coder,
            CodingProviderRole::Tester => &self.tester,
            CodingProviderRole::Analyst => &self.analyst,
            CodingProviderRole::CodeReviewer => &self.code_reviewer,
            CodingProviderRole::InternalReviewer => &self.internal_reviewer,
        }
    }

    pub fn set_provider_for_role(&mut self, role: &CodingProviderRole, provider: ProviderName) {
        match role {
            CodingProviderRole::Coder => self.coder = provider,
            CodingProviderRole::Tester => self.tester = provider,
            CodingProviderRole::Analyst => self.analyst = provider,
            CodingProviderRole::CodeReviewer => self.code_reviewer = provider,
            CodingProviderRole::InternalReviewer => self.internal_reviewer = provider,
        }
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
pub enum AnalystVerdict {
    NeedsFix,
    NeedsHumanInput,
    NoIssue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodingEntryType {
    UserMessage,
    AssistantMessage,
    ToolCall {
        tool_name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        output: String,
        is_error: bool,
    },
    StageGate {
        stage: CodingExecutionStage,
        countdown_seconds: u8,
    },
    AnalystVerdict {
        verdict: AnalystVerdict,
    },
    StageSummary {
        stage: CodingExecutionStage,
        summary: String,
    },
    SystemEvent {
        event_type: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingChatEntry {
    pub id: String,
    pub attempt_id: String,
    pub node_id: Option<String>,
    pub role: CodingAgentRole,
    pub entry_type: CodingEntryType,
    pub content: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingContextNote {
    pub id: String,
    pub attempt_id: String,
    pub content: String,
    pub created_at: String,
    pub consumed_by_rework_round: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingReworkInstruction {
    pub id: String,
    pub attempt_id: String,
    pub source_stage: CodingExecutionStage,
    pub rework_round: u32,
    pub summary: String,
    pub fix_hints: Vec<String>,
    pub questions: Vec<String>,
    pub created_at: String,
    pub consumed_by_node_id: Option<String>,
    pub consumed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingStageGateStatus {
    Open,
    Confirmed,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingStageGateState {
    pub gate_id: String,
    pub attempt_id: String,
    pub stage: CodingExecutionStage,
    pub role: CodingProviderRole,
    pub expires_at: String,
    pub provider_snapshot: CodingRoleProviderConfigSnapshot,
    pub status: CodingStageGateStatus,
    pub created_at: String,
    pub updated_at: String,
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
            "error" | "blocking" | "critical" | "high" => Ok(Self::Error),
            "warning" | "medium" => Ok(Self::Warning),
            "info" | "low" => Ok(Self::Info),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &[
                    "error", "warning", "info", "blocking", "critical", "high", "medium", "low",
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
    pub impact_scope: Vec<String>,
    pub pr_description: String,
    pub commit_message_suggestion: String,
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
    ConfirmStage,
    AcceptRisk,
    Abort,
    RetryPush,
    ManualFix,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingGateKind {
    Permission,
    StageGate,
    Blocked,
    FinalConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingGateRequired {
    pub gate_id: String,
    pub kind: CodingGateKind,
    pub title: String,
    pub description: String,
    pub stage: Option<CodingExecutionStage>,
    pub role: Option<CodingProviderRole>,
    pub expires_at: Option<String>,
    pub provider_snapshot: Option<CodingRoleProviderConfigSnapshot>,
    pub available_actions: Vec<CodingGateAction>,
}
