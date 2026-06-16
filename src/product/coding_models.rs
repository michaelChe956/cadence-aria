use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

use crate::product::models::{ProviderConversationRef, ProviderName, WorkItemExecutionPlanStatus};
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemDependencyHandoffRef {
    pub work_item_id: String,
    pub summary_ref: Option<String>,
    pub summary: Option<String>,
    pub commit_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemExecutionPlan {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub attempt_id: String,
    pub status: WorkItemExecutionPlanStatus,
    pub goal: String,
    #[serde(default)]
    pub allowed_write_scopes: Vec<String>,
    #[serde(default)]
    pub forbidden_write_scopes: Vec<String>,
    #[serde(default)]
    pub dependency_handoffs: Vec<WorkItemDependencyHandoffRef>,
    #[serde(default)]
    pub story_refs: Vec<String>,
    #[serde(default)]
    pub design_refs: Vec<String>,
    #[serde(default)]
    pub openspec_refs: Vec<String>,
    #[serde(default)]
    pub superpowers_contract: String,
    #[serde(default)]
    pub tdd_contract: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_plan_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_summary: Option<String>,
    #[serde(default)]
    pub risk_notes: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemHandoff {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub attempt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_run_ref: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub files_changed: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub diff_summary: String,
    #[serde(default)]
    pub tests_run: Vec<String>,
    #[serde(default)]
    pub test_result_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_summary: Option<String>,
    #[serde(default)]
    pub api_or_contract_changes: Vec<String>,
    #[serde(default)]
    pub open_risks: Vec<String>,
    #[serde(default)]
    pub next_work_item_notes: Vec<String>,
    pub created_at: String,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingRoleRunStatus {
    Running,
    Completed,
    Failed,
    Blocked,
    Superseded,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingRoleRunTrigger {
    Initial,
    RetryTestPlan,
    RerunMissingSteps,
    RetryReview,
    RetryAnalyst,
    RetryInternalReview,
    ManualRerun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingRoleRun {
    pub id: String,
    pub attempt_id: String,
    pub stage: CodingExecutionStage,
    pub role: CodingProviderRole,
    pub run_no: u32,
    pub status: CodingRoleRunStatus,
    pub trigger: CodingRoleRunTrigger,
    pub node_id: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    #[serde(default)]
    pub supersedes_run_id: Option<String>,
    #[serde(default)]
    pub superseded_by_run_id: Option<String>,
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub raw_provider_output_refs: Vec<String>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingRoleRunEventType {
    ProviderPrompt,
    ProviderStart,
    TextDelta,
    ExecutionEvent,
    ToolCall,
    ToolResult,
    StatusChanged,
    PermissionRequest,
    ChoiceRequest,
    MessageComplete,
    ProviderFailed,
    Timeout,
    Aborted,
    PersistenceWarning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingRoleRunEvent {
    pub attempt_id: String,
    pub role_run_id: String,
    pub node_id: Option<String>,
    pub stage: CodingExecutionStage,
    pub role: CodingProviderRole,
    pub sequence: u64,
    pub event_type: CodingRoleRunEventType,
    pub created_at: String,
    pub payload: serde_json::Value,
    pub truncated: bool,
    pub artifact_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingRoleRunEventSummary {
    pub event_count: usize,
    pub last_event_at: Option<String>,
    pub last_event_type: Option<CodingRoleRunEventType>,
    pub last_event_title: Option<String>,
    pub last_event_status: Option<String>,
    pub terminal_event_type: Option<CodingRoleRunEventType>,
    pub terminal_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingRoleRunEventPreview {
    pub sequence: u64,
    pub event_type: CodingRoleRunEventType,
    pub created_at: String,
    pub title: Option<String>,
    pub status: Option<String>,
    pub detail: Option<String>,
    pub truncated: bool,
    pub artifact_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingRoleRunSnapshot {
    #[serde(flatten)]
    pub run: CodingRoleRun,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_summary: Option<CodingRoleRunEventSummary>,
    #[serde(default)]
    pub recent_events: Vec<CodingRoleRunEventPreview>,
}

impl std::ops::Deref for CodingRoleRunSnapshot {
    type Target = CodingRoleRun;

    fn deref(&self) -> &Self::Target {
        &self.run
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingProviderPermissionMode {
    Auto,
    Supervised,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingRolePermissionModes {
    pub coder: CodingProviderPermissionMode,
    pub tester: CodingProviderPermissionMode,
    pub analyst: CodingProviderPermissionMode,
    pub code_reviewer: CodingProviderPermissionMode,
    pub internal_reviewer: CodingProviderPermissionMode,
}

impl Default for CodingRolePermissionModes {
    fn default() -> Self {
        Self {
            coder: CodingProviderPermissionMode::Supervised,
            tester: CodingProviderPermissionMode::Auto,
            analyst: CodingProviderPermissionMode::Auto,
            code_reviewer: CodingProviderPermissionMode::Supervised,
            internal_reviewer: CodingProviderPermissionMode::Supervised,
        }
    }
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
    #[serde(default)]
    pub permission_modes: CodingRolePermissionModes,
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
            permission_modes: CodingRolePermissionModes::default(),
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

    pub fn permission_mode_for_role(
        &self,
        role: &CodingProviderRole,
    ) -> CodingProviderPermissionMode {
        match role {
            CodingProviderRole::Coder => self.permission_modes.coder,
            CodingProviderRole::Tester => self.permission_modes.tester,
            CodingProviderRole::Analyst => self.permission_modes.analyst,
            CodingProviderRole::CodeReviewer => self.permission_modes.code_reviewer,
            CodingProviderRole::InternalReviewer => self.permission_modes.internal_reviewer,
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

    pub fn set_permission_mode_for_role(
        &mut self,
        role: &CodingProviderRole,
        mode: CodingProviderPermissionMode,
    ) {
        match role {
            CodingProviderRole::Coder => self.permission_modes.coder = mode,
            CodingProviderRole::Tester => self.permission_modes.tester = mode,
            CodingProviderRole::Analyst => self.permission_modes.analyst = mode,
            CodingProviderRole::CodeReviewer => self.permission_modes.code_reviewer = mode,
            CodingProviderRole::InternalReviewer => self.permission_modes.internal_reviewer = mode,
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
    #[serde(default)]
    pub provider_conversations: Vec<ProviderConversationRef>,
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
#[serde(rename_all = "snake_case")]
pub enum AnalystDecisionVerdict {
    NeedsFix,
    RerunTesting,
    Proceed,
    HumanRequired,
    Blocked,
}

impl AnalystDecisionVerdict {
    pub fn legacy_chat_verdict(&self) -> AnalystVerdict {
        match self {
            Self::NeedsFix | Self::RerunTesting => AnalystVerdict::NeedsFix,
            Self::Proceed => AnalystVerdict::NoIssue,
            Self::HumanRequired | Self::Blocked => AnalystVerdict::NeedsHumanInput,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalystDecisionNextStage {
    Coding,
    Testing,
    CodeReview,
    ReviewRequest,
    InternalPrReview,
    FinalConfirm,
    HumanGate,
}

impl AnalystDecisionNextStage {
    pub fn execution_stage(&self) -> Option<CodingExecutionStage> {
        match self {
            Self::Coding => Some(CodingExecutionStage::Coding),
            Self::Testing => Some(CodingExecutionStage::Testing),
            Self::CodeReview => Some(CodingExecutionStage::CodeReview),
            Self::ReviewRequest => Some(CodingExecutionStage::ReviewRequest),
            Self::InternalPrReview => Some(CodingExecutionStage::InternalPrReview),
            Self::FinalConfirm => Some(CodingExecutionStage::FinalConfirm),
            Self::HumanGate => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalystReworkInstructions {
    pub summary: String,
    #[serde(default)]
    pub required_changes: Vec<String>,
    #[serde(default)]
    pub verification_expectations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalystHumanGateRecommendation {
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub available_actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalystDecisionRecord {
    pub id: String,
    pub attempt_id: String,
    pub source_stage: CodingExecutionStage,
    pub rework_round: u32,
    pub verdict: AnalystDecisionVerdict,
    pub next_stage: AnalystDecisionNextStage,
    pub reason: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub raw_provider_output_refs: Vec<String>,
    #[serde(default)]
    pub rework_instructions: Option<AnalystReworkInstructions>,
    #[serde(default)]
    pub human_gate: Option<AnalystHumanGateRecommendation>,
    pub created_at: String,
    #[serde(default)]
    pub parse_error: Option<String>,
    #[serde(default)]
    pub role_run_id: Option<String>,
    #[serde(default)]
    pub run_no: Option<u32>,
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
pub struct CodingChoiceOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingChoiceGateStatus {
    Open,
    Resolved,
    Stale,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingChoiceGateResponse {
    pub selected_option_ids: Vec<String>,
    #[serde(default)]
    pub free_text: Option<String>,
    pub responded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingChoiceGate {
    pub gate_id: String,
    pub choice_id: String,
    pub attempt_id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    pub stage: CodingExecutionStage,
    pub role: CodingProviderRole,
    pub provider: ProviderName,
    pub source: String,
    pub prompt: String,
    #[serde(default)]
    pub options: Vec<CodingChoiceOption>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
    pub status: CodingChoiceGateStatus,
    #[serde(default)]
    pub response: Option<CodingChoiceGateResponse>,
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
pub enum TestPlanTool {
    RunCommand,
    ReadFile,
    ListFiles,
    SearchCode,
    ProviderManaged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPlanRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPlanStep {
    pub id: String,
    pub title: String,
    pub intent: String,
    pub required: bool,
    pub tool: TestPlanTool,
    pub risk_level: TestPlanRiskLevel,
    pub command_or_tool_input: serde_json::Value,
    pub evidence_expectation: String,
    #[serde(default)]
    pub related_requirements: Vec<String>,
    #[serde(default)]
    pub related_design_constraints: Vec<String>,
    #[serde(default)]
    pub related_work_item_tasks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPlan {
    pub id: String,
    pub attempt_id: String,
    #[serde(default)]
    pub role_run_id: Option<String>,
    #[serde(default)]
    pub run_no: Option<u32>,
    pub summary: String,
    #[serde(default)]
    pub context_warnings: Vec<String>,
    #[serde(default)]
    pub assumptions: Vec<String>,
    pub steps: Vec<TestPlanStep>,
    pub created_at: String,
    #[serde(default)]
    pub raw_provider_output_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestingStepResult {
    pub step_id: String,
    pub status: TestCommandStatus,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub provider_analysis: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestingUnplannedEvidence {
    pub tool_use_id: String,
    pub tool_name: String,
    pub status: TestCommandStatus,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub provider_analysis: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestingOverallStatus {
    Passed,
    PassedWithWarnings,
    Failed,
    SkippedByUserDecision,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestingReport {
    pub id: String,
    pub attempt_id: String,
    #[serde(default)]
    pub role_run_id: Option<String>,
    #[serde(default)]
    pub run_no: Option<u32>,
    pub commands: Vec<TestCommand>,
    pub overall_status: TestingOverallStatus,
    pub provider_claim: Option<serde_json::Value>,
    pub backend_verified: bool,
    pub started_at: String,
    pub completed_at: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub plan_summary: Option<String>,
    #[serde(default)]
    pub steps: Vec<TestingStepResult>,
    #[serde(default)]
    pub unplanned_commands: Vec<TestCommand>,
    #[serde(default)]
    pub unplanned_evidence: Vec<TestingUnplannedEvidence>,
    #[serde(default)]
    pub missing_required_steps: Vec<String>,
    #[serde(default)]
    pub skipped_required_steps: Vec<String>,
    #[serde(default)]
    pub context_warnings: Vec<String>,
    #[serde(default)]
    pub raw_provider_output_ref: Option<String>,
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
    #[serde(default = "default_review_finding_source_stage")]
    pub source_stage: CodingExecutionStage,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub related_requirements: Vec<String>,
    #[serde(default)]
    pub related_design_constraints: Vec<String>,
    #[serde(default)]
    pub related_work_item_tasks: Vec<String>,
}

fn default_review_finding_source_stage() -> CodingExecutionStage {
    CodingExecutionStage::CodeReview
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
    RetryTestPlan,
    RerunMissingSteps,
    ProvideContext,
    ManualContinue,
    RetryReview,
    RetryAnalyst,
    RetryInternalReview,
    SendRawOutputToAnalyst,
    AcceptTestingResult,
    RerunTesting,
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
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub raw_provider_output_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityGateBypassAudit {
    pub id: String,
    pub attempt_id: String,
    pub gate_id: String,
    pub stage: CodingExecutionStage,
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub skipped_required_steps: Vec<String>,
    pub operator_context: String,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn role_provider_config_deserializes_legacy_json_with_default_permission_modes() {
        let legacy = r#"{
          "coder": "codex",
          "tester": "claude_code",
          "analyst": "claude_code",
          "code_reviewer": "codex",
          "internal_reviewer": "claude_code",
          "review_rounds": 1
        }"#;

        let snapshot: CodingRoleProviderConfigSnapshot =
            serde_json::from_str(legacy).expect("legacy role config");

        assert_eq!(
            snapshot.permission_mode_for_role(&CodingProviderRole::Coder),
            CodingProviderPermissionMode::Supervised
        );
        assert_eq!(
            snapshot.permission_mode_for_role(&CodingProviderRole::Tester),
            CodingProviderPermissionMode::Auto
        );
        assert_eq!(
            snapshot.permission_mode_for_role(&CodingProviderRole::Analyst),
            CodingProviderPermissionMode::Auto
        );
        assert_eq!(
            snapshot.permission_mode_for_role(&CodingProviderRole::CodeReviewer),
            CodingProviderPermissionMode::Supervised
        );
        assert_eq!(
            snapshot.permission_mode_for_role(&CodingProviderRole::InternalReviewer),
            CodingProviderPermissionMode::Supervised
        );
    }

    #[test]
    fn test_plan_and_testing_report_round_trip_preserve_step_evidence() {
        let plan = TestPlan {
            id: "test_plan_0001".to_string(),
            attempt_id: "coding_attempt_0001".to_string(),
            role_run_id: None,
            run_no: None,
            summary: "unit and smoke checks".to_string(),
            context_warnings: vec!["missing_design_spec".to_string()],
            assumptions: vec!["target repo is already checked out".to_string()],
            steps: vec![TestPlanStep {
                id: "unit".to_string(),
                title: "Unit tests".to_string(),
                intent: "verify local unit behavior".to_string(),
                required: true,
                tool: TestPlanTool::RunCommand,
                risk_level: TestPlanRiskLevel::Low,
                command_or_tool_input: json!({"command": ["cargo", "test", "--locked"]}),
                evidence_expectation: "exit 0 and stdout/stderr refs".to_string(),
                related_requirements: vec!["REQ-1".to_string()],
                related_design_constraints: vec!["DES-1".to_string()],
                related_work_item_tasks: vec!["TASK-1".to_string()],
            }],
            created_at: "2026-06-10T00:00:00Z".to_string(),
            raw_provider_output_ref: Some("provider-raw/testing/plan_tests_0001.txt".to_string()),
        };
        let plan_value = serde_json::to_value(&plan).expect("serialize test plan");
        assert_eq!(plan_value["steps"][0]["tool"], "run_command");

        let report = TestingReport {
            id: "testing_report_0001".to_string(),
            attempt_id: "coding_attempt_0001".to_string(),
            role_run_id: None,
            run_no: None,
            commands: Vec::new(),
            overall_status: TestingOverallStatus::PassedWithWarnings,
            provider_claim: Some(json!({"summary": "passed with warnings"})),
            backend_verified: true,
            started_at: "2026-06-10T00:00:00Z".to_string(),
            completed_at: Some("2026-06-10T00:00:01Z".to_string()),
            plan_id: Some("test_plan_0001".to_string()),
            plan_summary: Some("unit and smoke checks".to_string()),
            steps: vec![TestingStepResult {
                step_id: "unit".to_string(),
                status: TestCommandStatus::Passed,
                evidence_refs: vec!["unit.stdout.log".to_string()],
                command: Some(vec![
                    "cargo".to_string(),
                    "test".to_string(),
                    "--locked".to_string(),
                ]),
                provider_analysis: Some("unit tests passed".to_string()),
            }],
            unplanned_commands: Vec::new(),
            unplanned_evidence: Vec::new(),
            missing_required_steps: Vec::new(),
            skipped_required_steps: vec!["security".to_string()],
            context_warnings: vec!["missing_design_spec".to_string()],
            raw_provider_output_ref: Some(
                "provider-raw/testing/execute_test_plan_0001.txt".to_string(),
            ),
        };
        let report_value = serde_json::to_value(&report).expect("serialize testing report");
        assert_eq!(report_value["overall_status"], "passed_with_warnings");
        assert_eq!(report_value["steps"][0]["step_id"], "unit");

        let review = CodeReviewReport {
            id: "code_review_0001".to_string(),
            attempt_id: "coding_attempt_0001".to_string(),
            round: 1,
            verdict: ReviewVerdict::RequestChanges,
            findings: vec![ReviewFinding {
                severity: FindingSeverity::Warning,
                file_path: Some("src/lib.rs".to_string()),
                line: Some(42),
                message: "missing validation".to_string(),
                required_action: Some("add validation".to_string()),
                source_stage: CodingExecutionStage::CodeReview,
                evidence: vec!["diff:src/lib.rs".to_string()],
                related_requirements: vec!["REQ-1".to_string()],
                related_design_constraints: vec!["DES-1".to_string()],
                related_work_item_tasks: vec!["TASK-1".to_string()],
            }],
            tested_evidence_refs: vec!["testing_report_0001.json".to_string()],
            diff_refs: vec!["attempt.diff".to_string()],
            summary: "needs validation".to_string(),
            created_at: "2026-06-10T00:00:02Z".to_string(),
            raw_provider_output_ref: Some(
                "provider-raw/code_review/code_review_0001.txt".to_string(),
            ),
            role_run_id: None,
            run_no: None,
        };
        let review_value = serde_json::to_value(&review).expect("serialize code review");
        assert_eq!(
            review_value["raw_provider_output_ref"],
            "provider-raw/code_review/code_review_0001.txt"
        );

        let gate = CodingGateRequired {
            gate_id: "coding_blocked_gate_0001".to_string(),
            kind: CodingGateKind::Blocked,
            title: "Review blocked".to_string(),
            description: "review payload could not be parsed".to_string(),
            stage: Some(CodingExecutionStage::CodeReview),
            role: Some(CodingProviderRole::CodeReviewer),
            expires_at: None,
            provider_snapshot: None,
            available_actions: vec![CodingGateAction {
                action_id: "retry_review".to_string(),
                label: "重试审查".to_string(),
                action_type: CodingGateActionType::RetryReview,
            }],
            reason_code: Some("review_payload_parse_error".to_string()),
            evidence_refs: vec!["code_review_0001.json".to_string()],
            raw_provider_output_ref: Some(
                "provider-raw/code_review/code_review_0001.txt".to_string(),
            ),
        };
        let gate_value = serde_json::to_value(&gate).expect("serialize gate");
        assert_eq!(gate_value["reason_code"], "review_payload_parse_error");
        assert_eq!(gate_value["evidence_refs"][0], "code_review_0001.json");
        assert_eq!(
            gate_value["raw_provider_output_ref"],
            "provider-raw/code_review/code_review_0001.txt"
        );
    }

    #[test]
    fn legacy_coding_qa_records_deserialize_with_defaults() {
        let legacy_testing_report = r#"{
          "id": "testing_report_0001",
          "attempt_id": "coding_attempt_0001",
          "commands": [],
          "overall_status": "passed",
          "provider_claim": null,
          "backend_verified": true,
          "started_at": "2026-06-10T00:00:00Z",
          "completed_at": "2026-06-10T00:00:01Z"
        }"#;

        let report: TestingReport = serde_json::from_str(legacy_testing_report).unwrap();
        assert_eq!(report.plan_id, None);
        assert!(report.steps.is_empty());
        assert!(report.missing_required_steps.is_empty());
        assert_eq!(report.raw_provider_output_ref, None);

        let legacy_code_review = r#"{
          "id": "code_review_0001",
          "attempt_id": "coding_attempt_0001",
          "round": 1,
          "verdict": "request_changes",
          "findings": [
            {
              "severity": "warning",
              "file_path": "src/lib.rs",
              "line": 42,
              "message": "missing validation",
              "required_action": "add validation"
            }
          ],
          "tested_evidence_refs": [],
          "diff_refs": [],
          "summary": "needs validation",
          "created_at": "2026-06-10T00:00:02Z"
        }"#;
        let review: CodeReviewReport = serde_json::from_str(legacy_code_review).unwrap();
        assert_eq!(review.raw_provider_output_ref, None);
        assert_eq!(
            review.findings[0].source_stage,
            CodingExecutionStage::CodeReview
        );
        assert!(review.findings[0].evidence.is_empty());
        assert!(review.findings[0].related_requirements.is_empty());
        assert!(review.findings[0].related_design_constraints.is_empty());
        assert!(review.findings[0].related_work_item_tasks.is_empty());

        let legacy_internal_review = r#"{
          "id": "internal_pr_review_0001",
          "attempt_id": "coding_attempt_0001",
          "review_request_id": "review_request_0001",
          "verdict": "approve",
          "findings": [],
          "impact_scope": [],
          "pr_description": "ready",
          "commit_message_suggestion": "feat: ready",
          "tested_evidence_refs": [],
          "diff_refs": [],
          "summary": "ready",
          "created_at": "2026-06-10T00:00:03Z"
        }"#;
        let internal_review: InternalPrReview =
            serde_json::from_str(legacy_internal_review).unwrap();
        assert_eq!(internal_review.raw_provider_output_ref, None);

        let legacy_gate = r#"{
          "gate_id": "coding_gate_0001",
          "kind": "stage_gate",
          "title": "Confirm",
          "description": "confirm stage",
          "stage": "testing",
          "role": "tester",
          "expires_at": null,
          "provider_snapshot": null,
          "available_actions": []
        }"#;
        let gate: CodingGateRequired = serde_json::from_str(legacy_gate).unwrap();
        assert_eq!(gate.reason_code, None);
        assert!(gate.evidence_refs.is_empty());
        assert_eq!(gate.raw_provider_output_ref, None);
    }
}
