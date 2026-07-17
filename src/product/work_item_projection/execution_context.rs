use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderExecutionEnvelope {
    pub repository_state_ref: String,
    pub resolved_handoff_revision_ids: Vec<String>,
    pub unit_run_id: String,
    pub previous_actionable_review: Option<String>,
    pub start_commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerExecutionEnvelope {
    pub unit_run_id: String,
    pub diff_ref: String,
    pub test_evidence_refs: Vec<String>,
    pub handoff_revision_ids: Vec<String>,
    pub contract_delta_refs: Vec<String>,
    pub completion_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedExecutionContext {
    pub text: String,
    pub renderer_version: String,
    pub content_hash: String,
}
