use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::product::coding_models::{
    AnalystDecisionRecord, CodeReviewReport, CodingAttemptStatus, CodingChatEntry,
    CodingChoiceGate, CodingExecutionStage, CodingGateRequired as CodingGateRequiredModel,
    CodingProviderPermissionMode, CodingProviderRole, CodingRoleProviderConfigSnapshot,
    CodingRoleRunSnapshot, CodingTimelineNode, CodingTimelineNodeStatus, InternalPrReview,
    ReviewRequest, TestingReport, WorkItemExecutionPlan, WorkItemHandoff,
};
use crate::product::models::ProviderName;
use crate::web::types::CodingExecutionUnitDto;
use crate::web::workspace_ws_types::{
    ChoiceOption, ProviderConfigSnapshot, WsExecutionEvent, WsPermissionRiskLevel,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodingWsOutMessage {
    CodingSessionState {
        attempt_id: String,
        attempt_scope: String,
        work_item_group_id: Option<String>,
        current_work_item_id: Option<String>,
        active_unit_id: Option<String>,
        units: Vec<CodingExecutionUnitDto>,
        status: CodingAttemptStatus,
        stage: CodingExecutionStage,
        branch_name: String,
        base_branch: String,
        worktree_path: Option<PathBuf>,
        rework_count: u32,
        max_auto_rework: u32,
        head_commit: Option<String>,
        pushed_remote: Option<String>,
        role_provider_config_snapshot: Box<CodingRoleProviderConfigSnapshot>,
        provider_config_snapshot: Box<ProviderConfigSnapshot>,
        chat_entries: Box<Vec<CodingChatEntry>>,
        timeline_nodes: Box<Vec<CodingTimelineNode>>,
        active_node_id: Option<String>,
        testing_report: Box<Option<TestingReport>>,
        code_review_reports: Box<Vec<CodeReviewReport>>,
        review_request: Box<Option<ReviewRequest>>,
        internal_pr_review: Box<Option<InternalPrReview>>,
        pending_gates: Box<Vec<CodingGateRequiredModel>>,
        pending_choices: Box<Vec<CodingChoiceGate>>,
        latest_analyst_decision: Box<Option<AnalystDecisionRecord>>,
        role_runs: Box<Vec<CodingRoleRunSnapshot>>,
        work_item_markdown: Option<String>,
        verification_commands: Box<Vec<String>>,
        work_item_execution_plan: Box<Option<WorkItemExecutionPlan>>,
        work_item_handoff: Box<Option<WorkItemHandoff>>,
    },
    CodingStageChange {
        stage: CodingExecutionStage,
    },
    CodingTimelineNodeCreated {
        node: CodingTimelineNode,
    },
    CodingTimelineNodeUpdated {
        node_id: String,
        status: CodingTimelineNodeStatus,
        summary: Option<String>,
        completed_at: Option<String>,
    },
    CodingExecutionEvent {
        event: WsExecutionEvent,
    },
    CodingPermissionRequest {
        id: String,
        tool_name: String,
        description: String,
        risk_level: WsPermissionRiskLevel,
    },
    CodingChoiceRequest {
        id: String,
        prompt: String,
        source: String,
        options: Vec<ChoiceOption>,
        allow_multiple: bool,
        allow_free_text: bool,
    },
    CodingChoiceResponseAck {
        id: String,
        selected_option_ids: Vec<String>,
        free_text: Option<String>,
    },
    CodingStreamChunk {
        content: String,
        node_id: Option<String>,
    },
    CodingMessageComplete {
        node_id: Option<String>,
    },
    TestingReportUpdate {
        report: Box<TestingReport>,
    },
    CodeReviewComplete {
        report: Box<CodeReviewReport>,
    },
    ReviewRequestUpdate {
        review_request: Box<ReviewRequest>,
    },
    InternalPrReviewComplete {
        review: Box<InternalPrReview>,
    },
    CodingGateRequired {
        gate: CodingGateRequiredModel,
    },
    CodingChatEntryCreated {
        entry: CodingChatEntry,
    },
    CodingProviderConfigUpdated {
        role: CodingProviderRole,
        provider: ProviderName,
    },
    CodingProtocolError {
        code: String,
        message: String,
    },
    CodingPong,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodingWsInMessage {
    CodingHello {
        attempt_id: String,
        last_seen_node_id: Option<String>,
    },
    StartCoding,
    ContextNote {
        content: String,
    },
    PermissionResponse {
        id: String,
        approved: bool,
        reason: Option<String>,
    },
    ChoiceResponse {
        id: String,
        selected_option_ids: Vec<String>,
        free_text: Option<String>,
    },
    GateResponse {
        gate_id: String,
        action_id: String,
        extra_context: Option<String>,
    },
    ContinueRework {
        extra_context: Option<String>,
    },
    ProviderSelect {
        role: String,
        provider: ProviderName,
    },
    PermissionModeSelect {
        role: String,
        permission_mode: CodingProviderPermissionMode,
    },
    StageGateConfirm {
        stage: CodingExecutionStage,
    },
    FinalConfirm,
    AbortAttempt,
    RequestManualPause,
    CodingPing,
}
