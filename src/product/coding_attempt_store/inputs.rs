use std::path::PathBuf;

use crate::product::coding_models::{
    AttemptTargetSnapshot, CodingChoiceOption, CodingChoiceQuestion, CodingExecutionStage,
    CodingExecutionUnitStatus, CodingGateAction, CodingProviderRole, CodingStartRunPolicy,
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
    pub target_snapshot: Option<AttemptTargetSnapshot>,
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
    pub target_snapshot: Option<AttemptTargetSnapshot>,
    pub max_auto_rework: u32,
    /// P2 Task 2：group attempt 首建时冻结的首启策略；普通/手工/legacy/
    /// 多 target 调用方显式 Manual，Task 1 自动服务核验后才传 AutoStartOnce。
    pub start_run_policy: CodingStartRunPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCodingExecutionUnitInput {
    pub attempt_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub dependency_logical_work_item_ids: Vec<String>,
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
    pub questions: Vec<CodingChoiceQuestion>,
}

/// `resolve_choice_gate` 的应答载荷：原样落入 `CodingChoiceGateResponse`
///（responded_at 由 store 落盘时补齐）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveChoiceGateInput {
    pub selected_option_ids: Vec<String>,
    pub free_text: Option<String>,
    pub answers: Vec<crate::cross_cutting::streaming_provider::ChoiceAnswerData>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateQualityBypassAuditInput {
    pub attempt_id: String,
    pub gate_id: String,
    pub stage: CodingExecutionStage,
    pub reason_code: Option<String>,
    pub operator_context: String,
}
