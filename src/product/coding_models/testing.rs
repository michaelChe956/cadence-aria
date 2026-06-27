use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
