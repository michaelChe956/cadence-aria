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
    /// P0 1.3（REQ-WIGA-05）：完整逐题答案（Task 6 `answers` 原样落盘）；
    /// 旧单题 gate 仅有 selected/free_text，serde 缺省为空。
    #[serde(default)]
    pub answers: Vec<crate::cross_cutting::streaming_provider::ChoiceAnswerData>,
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
    /// P0 1.3：多问题透传（Task 6 完整 questions）；旧 gate serde 缺省空，
    /// `effective_questions` 按单一 default 题兼容投影。
    #[serde(default)]
    pub questions: Vec<CodingChoiceQuestion>,
    pub created_at: String,
    pub updated_at: String,
}

/// P0 1.3：coding choice 逐题结构（与 provider `ChoiceQuestionData` 同构）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingChoiceQuestion {
    pub id: String,
    pub prompt: String,
    pub options: Vec<CodingChoiceOption>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
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
    SendToCoder,
    ConfirmStage,
    AcceptRisk,
    Abort,
    RetryPush,
    ManualFix,
    ProvideContext,
    ManualContinue,
    RetryCoding,
    RetryReview,
    RetryInternalReview,
    RetryGroupReviewShard,
    RetryGroupReduction,
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
    #[serde(default)]
    pub diagnostic: Option<CodingGateDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CodingGateDiagnostic {
    #[serde(default)]
    pub actual_value: Option<String>,
    #[serde(default)]
    pub limit: Option<String>,
    pub phase: String,
    pub run_failure_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityGateBypassAudit {
    pub id: String,
    pub attempt_id: String,
    pub gate_id: String,
    pub stage: CodingExecutionStage,
    #[serde(default)]
    pub reason_code: Option<String>,
    pub operator_context: String,
    pub created_at: String,
}

impl CodingChoiceGate {
    /// 旧 gate（无 questions）按单一 default 题兼容：题干/gate 级选项透传，
    /// 不猜测多题结构；新 gate 原样返回。
    pub fn effective_questions(&self) -> Vec<CodingChoiceQuestion> {
        if !self.questions.is_empty() {
            return self.questions.clone();
        }
        vec![CodingChoiceQuestion {
            id: "default".to_string(),
            prompt: self.prompt.clone(),
            options: self.options.clone(),
            allow_multiple: self.allow_multiple,
            allow_free_text: self.allow_free_text,
        }]
    }
}
