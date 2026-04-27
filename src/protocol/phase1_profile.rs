use serde::{Deserialize, Serialize};

pub const PHASE1_PROFILE_VERSION: &str = "phase1.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AriaProfile {
    pub profile_version: String,
    pub constraint_check_ref: String,
    pub traceability_refs: Vec<String>,
    pub provider_run_refs: Vec<String>,
    pub projection_refs: Vec<String>,
    #[serde(default)]
    pub worktask_routing: Vec<WorktaskRoutingEntry>,
    #[serde(default)]
    pub coverage_summary: Option<ProfileCoverageSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorktaskRoutingEntry {
    pub worktask_id: String,
    pub source_work_package_id: String,
    pub execution_mode: String,
    pub human_required_reason: Option<String>,
    pub allowed_write_scope: Vec<String>,
    pub traceability_refs: Vec<String>,
    pub verification_commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProfileCoverageSummary {
    pub closed: Vec<String>,
    pub uncovered: Vec<String>,
    pub exempted: Vec<String>,
}
