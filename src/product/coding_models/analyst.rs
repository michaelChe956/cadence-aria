use serde::{Deserialize, Serialize};

use super::execution::CodingExecutionStage;

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
