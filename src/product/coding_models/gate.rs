use serde::{Deserialize, Serialize};

use crate::product::models::ProviderName;

use super::execution::{CodingExecutionStage, CodingProviderRole};
use super::provider_config::CodingRoleProviderConfigSnapshot;

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
