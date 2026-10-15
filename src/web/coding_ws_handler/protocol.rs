use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::product::coding_models::{
    CodeReviewReport, CodingAttemptStatus, CodingChatEntry, CodingChoiceGate, CodingChoiceQuestion,
    CodingExecutionStage,
    CodingGateRequired as CodingGateRequiredModel, CodingProviderPermissionMode,
    CodingProviderRole, CodingRoleProviderConfigSnapshot, CodingRoleRunSnapshot,
    CodingTimelineNode, CodingTimelineNodeStatus, GroupFinalReadinessSnapshot, InternalPrReview,
    ReviewRequest, WorkItemExecutionPlan,
};
use crate::product::json_store::ProductStoreError;
use crate::product::models::ProviderName;
use crate::product::models::{
    PlanAmendmentManifest, PlanRepairRequest, PlanRepairSessionSnapshotDto, WorkspaceSessionLink,
};
use crate::web::types::{
    CodingExecutionUnitDto, GroupCodingProgressDto, GroupReviewArtifactProjection,
    WorkItemCodingProgressDto,
};
use crate::web::workspace_ws_types::{
    ChoiceOption, ProviderConfigSnapshot, WsExecutionEvent, WsPermissionRiskLevel,
};

/// 刷新回放单元：role-run 审计 journal 重建出的历史执行事件（与实时
/// `coding_execution_event` 同形），附 journal 落盘时间供前端按序装载。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingExecutionEventReplay {
    pub event: WsExecutionEvent,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodingWsOutMessage {
    CodingSessionState {
        project_id: String,
        issue_id: String,
        attempt_id: String,
        attempt_scope: String,
        work_item_group_id: Option<String>,
        current_work_item_id: Option<String>,
        active_unit_id: Option<String>,
        units: Vec<CodingExecutionUnitDto>,
        #[serde(default)]
        group_coding_progress: Box<Option<Vec<WorkItemCodingProgressDto>>>,
        #[serde(default)]
        group_progress: Box<Option<GroupCodingProgressDto>>,
        status: CodingAttemptStatus,
        stage: CodingExecutionStage,
        branch_name: String,
        base_branch: String,
        worktree_path: Option<PathBuf>,
        rework_count: u32,
        max_auto_rework: u32,
        head_commit: Box<Option<String>>,
        pushed_remote: Box<Option<String>>,
        role_provider_config_snapshot: Box<CodingRoleProviderConfigSnapshot>,
        provider_config_snapshot: Box<ProviderConfigSnapshot>,
        chat_entries: Box<Vec<CodingChatEntry>>,
        #[serde(default)]
        execution_events: Box<Vec<CodingExecutionEventReplay>>,
        timeline_nodes: Box<Vec<CodingTimelineNode>>,
        active_node_id: Box<Option<String>>,
        code_review_reports: Box<Vec<CodeReviewReport>>,
        review_request: Box<Option<ReviewRequest>>,
        internal_pr_review: Box<Option<InternalPrReview>>,
        #[serde(default)]
        group_review_artifacts: Box<Option<GroupReviewArtifactProjection>>,
        #[serde(default)]
        group_final_readiness: Box<Option<GroupFinalReadinessSnapshot>>,
        pending_gates: Box<Vec<CodingGateRequiredModel>>,
        pending_choices: Box<Vec<CodingChoiceGate>>,
        role_runs: Box<Vec<CodingRoleRunSnapshot>>,
        work_item_markdown: Box<Option<String>>,
        verification_commands: Box<Vec<String>>,
        work_item_execution_plan: Box<Option<WorkItemExecutionPlan>>,
        linked_plan_repair: Box<Option<PlanRepairSessionSnapshotDto>>,
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
        /// P2-e（REQ-CRO-11）：多题透传——与快照 `pending_choices[].questions`
        /// 同形；旧帧 serde 缺省为空，前端按单一 default 题回落。
        #[serde(default)]
        questions: Vec<CodingChoiceQuestion>,
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
    PlanRepairRequired {
        request: Box<PlanRepairRequestDto>,
        session_link: Option<WorkspaceSessionLinkDto>,
    },
    PlanAmendmentUpdated {
        event_id: String,
        amendment: Box<PlanAmendmentManifestDto>,
    },
}

pub type PlanRepairRequestDto = PlanRepairRequest;

pub type WorkspaceSessionLinkDto = WorkspaceSessionLink;
pub type PlanAmendmentManifestDto = PlanAmendmentManifest;

/// F-43：`coding_choice_request` 帧的唯一构造点。
///
/// 实时发射（引擎 `emit_choice_request`）与 attach 补发（新连接初帧）共用同一
/// 映射，保证两路帧逐字段同形——客户端按 choice id upsert 卡片，两路不同形会
/// 直接表现为「卡在但答不了」。输入刻意取 durable `CodingChoiceGate`：引擎落盘
/// 后即可用它发射，补发侧读同一记录，无需另抄一份映射。
pub(crate) fn coding_choice_request_frame(gate: &CodingChoiceGate) -> CodingWsOutMessage {
    CodingWsOutMessage::CodingChoiceRequest {
        id: gate.choice_id.clone(),
        prompt: gate.prompt.clone(),
        source: gate.source.clone(),
        options: gate
            .options
            .iter()
            .map(|option| ChoiceOption {
                id: option.id.clone(),
                label: option.label.clone(),
                description: option.description.clone(),
            })
            .collect(),
        allow_multiple: gate.allow_multiple,
        allow_free_text: gate.allow_free_text,
        questions: gate.questions.clone(),
    }
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
        /// P0 1.3（Task 9）：完整逐题答案；旧单题客户端缺省为空。
        #[serde(default)]
        answers: Vec<crate::cross_cutting::streaming_provider::ChoiceAnswerData>,
        /// P0 1.3：显式 claim 身份；缺省绑定当前唯一 run 并生成 command_id。
        #[serde(default)]
        command_id: Option<String>,
        #[serde(default)]
        expected_run_id: Option<String>,
    },
    GateResponse {
        gate_id: String,
        action_id: String,
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
    MaxAutoReworkSelect {
        max_auto_rework: u32,
    },
    StageGateConfirm {
        stage: CodingExecutionStage,
    },
    FinalConfirm,
    RetryPush,
    AbortAttempt,
    /// F-16：`awaiting_manual_recovery` 的显式恢复动作——重走 admission CAS
    /// 回到 Running 并重启 runner（人工恢复态唯一非 Abort 放行消息）。
    RecoverCoding,
    /// F-44：`aborted`/`failed` 终态的显式「重新开始」动作——重走 admission CAS
    /// 回到 Running 并重启 runner（终态唯一放行消息；`completed` 不提供重开，
    /// StartCoding 对终态继续 fail-closed 拒绝）。
    RestartCoding,
    RequestManualPause,
    CodingPing,
}

pub(crate) fn coding_attempt_lookup_protocol_error(error: &ProductStoreError) -> (String, String) {
    match error {
        ProductStoreError::NotFound {
            kind: "coding_attempt",
            ..
        } => (
            "coding_attempt_not_found".to_string(),
            "coding attempt not found".to_string(),
        ),
        ProductStoreError::Ambiguous {
            kind: "coding_attempt",
            ..
        } => (
            "coding_attempt_ambiguous".to_string(),
            "coding attempt matches multiple issues; reopen it from Workbench".to_string(),
        ),
        ProductStoreError::IdentityMismatch {
            kind: "coding_attempt",
            ..
        } => (
            "coding_attempt_scope_mismatch".to_string(),
            "coding attempt does not belong to the requested project and issue".to_string(),
        ),
        other => (
            "product_store_error".to_string(),
            format!("coding attempt lookup failed: {other}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(id: &str) -> CodingChoiceQuestion {
        CodingChoiceQuestion {
            id: id.to_string(),
            prompt: format!("{id} prompt"),
            options: vec![],
            allow_multiple: false,
            allow_free_text: true,
        }
    }

    fn choice_gate(questions: Vec<CodingChoiceQuestion>) -> CodingChoiceGate {
        CodingChoiceGate {
            gate_id: "coding_choice_gate_1".to_string(),
            choice_id: "choice_1".to_string(),
            attempt_id: "coding_attempt_1".to_string(),
            node_id: None,
            stage: CodingExecutionStage::Coding,
            role: CodingProviderRole::Coder,
            provider: ProviderName::Fake,
            source: "provider".to_string(),
            prompt: "编码拆分确认".to_string(),
            options: vec![],
            allow_multiple: false,
            allow_free_text: false,
            status: crate::product::coding_models::CodingChoiceGateStatus::Open,
            response: None,
            questions,
            expected_run_id: None,
            created_at: "2026-10-15T00:00:00Z".to_string(),
            updated_at: "2026-10-15T00:00:00Z".to_string(),
        }
    }

    /// P2-e 红测①：带 questions 的 gate 输出的帧逐题带 questions（实时帧与
    /// attach 补发共用本构造点，两路同形）。
    #[test]
    fn coding_choice_request_frame_maps_questions_verbatim() {
        let frame = coding_choice_request_frame(&choice_gate(vec![question("q_0"), question("q_1")]));
        let CodingWsOutMessage::CodingChoiceRequest { questions, .. } = &frame else {
            panic!("expected coding_choice_request frame");
        };
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].id, "q_0");
        assert_eq!(questions[1].id, "q_1");
        assert!(questions[0].allow_free_text);
    }

    /// wire 形状：多题帧序列化含 questions；旧 gate（questions 空）帧同样
    /// 落 `questions: []`，前端按 default 单题回落。
    #[test]
    fn coding_choice_request_frame_serializes_questions() {
        let multi =
            serde_json::to_value(coding_choice_request_frame(&choice_gate(vec![question("q_0")])))
                .unwrap();
        assert_eq!(multi["type"], "coding_choice_request");
        assert_eq!(multi["questions"][0]["id"], "q_0");
        let legacy = serde_json::to_value(coding_choice_request_frame(&choice_gate(vec![])))
            .unwrap();
        assert!(legacy["questions"].as_array().unwrap().is_empty());
    }
}
