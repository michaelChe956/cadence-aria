use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryProfileConfidence {
    Low,
    Medium,
    High,
}

impl RepositoryProfileConfidence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryProfile {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub provider_run_ref: Option<String>,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub package_managers: Vec<String>,
    pub test_frameworks: Vec<String>,
    pub build_systems: Vec<String>,
    pub verification_capabilities: Vec<String>,
    pub detected_layers: Vec<String>,
    pub split_recommendation: String,
    pub confidence: RepositoryProfileConfidence,
    pub uncertainties: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationScope {
    Unit,
    Integration,
    E2e,
    Build,
    Lint,
    Manual,
    Custom,
}

impl VerificationScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Integration => "integration",
            Self::E2e => "e2e",
            Self::Build => "build",
            Self::Lint => "lint",
            Self::Manual => "manual",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationCommandSource {
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationCommandSafety {
    Approved,
    NeedsManualReview,
}

impl VerificationCommandSafety {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::NeedsManualReview => "needs_manual_review",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationFallbackPolicy {
    ManualGate,
    RepairProviderOutput,
}

impl VerificationFallbackPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ManualGate => "manual_gate",
            Self::RepairProviderOutput => "repair_provider_output",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerificationCommand {
    pub id: String,
    pub label: String,
    pub command: String,
    pub cwd: String,
    pub purpose: String,
    pub required: bool,
    pub timeout_seconds: u64,
    pub source: VerificationCommandSource,
    pub safety: VerificationCommandSafety,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerificationManualCheck {
    pub id: String,
    pub label: String,
    pub instructions: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerificationPlan {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub repository_profile_ref: Option<String>,
    pub provider_run_ref: Option<String>,
    pub scope: VerificationScope,
    pub commands: Vec<VerificationCommand>,
    pub manual_checks: Vec<VerificationManualCheck>,
    pub required_gates: Vec<String>,
    pub risk_notes: Vec<String>,
    pub confidence: RepositoryProfileConfidence,
    pub fallback_policy: VerificationFallbackPolicy,
    pub created_at: String,
    pub updated_at: String,
}
