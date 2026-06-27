use std::path::PathBuf;

use crate::product::coding_models::{
    CodingChoiceOption, CodingExecutionStage, CodingExecutionUnitStatus, CodingGateAction,
    CodingProviderRole,
};
use crate::product::models::ProviderName;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCodingAttemptInput {
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub base_branch: String,
    pub branch_name: String,
    pub worktree_path: Option<PathBuf>,
    pub provider_config_snapshot: ProviderConfigSnapshot,
    pub max_auto_rework: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateGroupCodingAttemptInput {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub current_work_item_id: String,
    pub base_branch: String,
    pub branch_name: String,
    pub worktree_path: Option<PathBuf>,
    pub provider_config_snapshot: ProviderConfigSnapshot,
    pub max_auto_rework: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCodingExecutionUnitInput {
    pub attempt_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub work_item_id: String,
    pub order_index: u32,
    pub status: CodingExecutionUnitStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateBlockedGateInput {
    pub attempt_id: String,
    pub stage: CodingExecutionStage,
    pub node_id: Option<String>,
    pub role: Option<CodingProviderRole>,
    pub title: String,
    pub description: String,
    pub reason_code: Option<String>,
    pub evidence_refs: Vec<String>,
    pub raw_provider_output_ref: Option<String>,
    pub available_actions: Vec<CodingGateAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateChoiceGateInput {
    pub attempt_id: String,
    pub choice_id: String,
    pub stage: CodingExecutionStage,
    pub node_id: Option<String>,
    pub role: CodingProviderRole,
    pub provider: ProviderName,
    pub source: String,
    pub prompt: String,
    pub options: Vec<CodingChoiceOption>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateQualityBypassAuditInput {
    pub attempt_id: String,
    pub gate_id: String,
    pub stage: CodingExecutionStage,
    pub reason_code: Option<String>,
    pub skipped_required_steps: Vec<String>,
    pub operator_context: String,
}
