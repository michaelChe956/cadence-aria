use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::product::models::{
    HumanPresentationRevision, NodeDetail, PlanRepairSessionSnapshotDto, WorkspaceSessionStatus,
    WorkspaceType,
};
use crate::product::work_item_plan_policy::{
    HumanGateSnapshot, PolicyDiagnostic, ProviderStartLedgerEntry, RepairReservation,
    ReviewInvocationScope, RunHistory, RunPolicy, WorkItemPlanFlowKind,
};
use crate::product::workspace_engine::LinkedWorkspaceSessionSnapshot;

use super::artifact::ArtifactPayload;
use super::artifact_version::{ArtifactVersion, ArtifactVersionSummary};
use super::common::{
    ChoiceOption, ChoiceQuestion, ProviderConfigSnapshot, ProviderDefaults, WsCheckpointDto,
    WsExecutionEvent, WsMessageDto, WsPermissionRiskLevel, WsProviderConfig, WsProviderStatus,
};
use super::review::{
    ReviewFinding, ReviewGate, ReviewVerdictType, StructuredOutputDiagnostic,
    WorkItemPlanReviewComplete,
};
use super::timeline::{NodeDetailSummary, TimelineNode, TimelineNodeStatus};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoverableInterruptedOperation {
    Review,
    WorkItemPlanAuthorGeneration,
    WorkItemDraftGeneration,
    Revision,
}

/// F-27：session_state 顶层挂起 choice 投影元素——与 `ChoiceRequest` 帧字段
/// 同构（同一 options/questions DTO），供前端收帧对账补挂 choice 卡。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WsPendingChoiceRequest {
    pub id: String,
    pub prompt: String,
    pub options: Vec<ChoiceOption>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
    pub questions: Vec<ChoiceQuestion>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverableInterruptedRun {
    pub failed_node_id: String,
    pub operation: RecoverableInterruptedOperation,
    pub label: String,
}

// SessionState 必须按既有 WebSocket wire schema 内联完整 durable snapshot；为节省
// Rust 内存布局而装箱会改变公共枚举字段，故保留现有表示。
/// 广播由 session manager 在序列化边界额外注入可选顶层 `event_seq`，不在此枚举
/// 各变体重复字段；旧 wire 客户端可忽略该增量字段。
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsOutMessage {
    StreamChunk {
        role: String,
        content: String,
        node_id: Option<String>,
    },
    MessageComplete {
        message_id: String,
        checkpoint_id: String,
        node_id: Option<String>,
    },
    StageChange {
        stage: String,
    },
    ArtifactUpdate {
        version: u32,
        #[serde(flatten)]
        payload: ArtifactPayload,
    },
    ProviderSelectRequest {
        stage: String,
        defaults: ProviderDefaults,
    },
    PermissionRequest {
        id: String,
        tool_name: String,
        description: String,
        risk_level: WsPermissionRiskLevel,
    },
    ChoiceRequest {
        id: String,
        prompt: String,
        options: Vec<ChoiceOption>,
        allow_multiple: bool,
        allow_free_text: bool,
        questions: Vec<ChoiceQuestion>,
        source: String,
    },
    ProviderStatus {
        status: WsProviderStatus,
    },
    ExecutionEvent {
        event: WsExecutionEvent,
    },
    TimelineNodeCreated {
        node: TimelineNode,
    },
    TimelineNodeUpdated {
        node_id: String,
        status: TimelineNodeStatus,
        summary: Option<String>,
        completed_at: Option<String>,
    },
    ReviewComplete {
        node_id: String,
        round: u32,
        verdict: ReviewVerdictType,
        comments: String,
        summary: String,
        findings: Vec<ReviewFinding>,
        review_gate: ReviewGate,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        work_item_plan_review: Option<WorkItemPlanReviewComplete>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        structured_output_diagnostic: Option<StructuredOutputDiagnostic>,
    },
    ReviewDecisionRequired {
        node_id: String,
        round: u32,
        options: Vec<String>,
    },
    // 退役留档（T5/REQ-RET-02）：HumanPresentationRevisionSaved/SaveFailed
    // 出站变体随保存命令族删除（唯一生产点=已删 inbound 臂）。
    LinkedWorkspaceAmendmentCreated {
        snapshot: LinkedWorkspaceSessionSnapshot,
    },
    HumanGateTurnOpen {
        turn_id: String,
        command_id: String,
        remaining_budget: u32,
    },
    HumanGateTurnCompleted {
        turn_id: String,
        artifact_ref: String,
    },
    HumanGateTurnFailed {
        turn_id: String,
        failure_class: String,
        message: String,
    },
    HumanGateBusy {
        turn_id: String,
    },
    HumanGateClosed {
        decision: String,
        stage: String,
    },
    AdvanceCompleted {
        command_id: String,
        attempt_id: String,
        workspace_entry: String,
        /// REQ-MTG-03（OQ1，WP2 additive）：多 target 拆分的全集绑定（单 target
        /// 空集不发送——wire 零变化；多 target 全集，`attempt_id` 为拓扑序首个）。
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        target_attempts: Vec<crate::product::advance_store::AdvanceTargetAttemptBinding>,
    },
    AdvanceRejected {
        command_id: String,
        code: String,
        reason: String,
    },
    SessionState {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        connection_id: Option<String>,
        session_id: String,
        workspace_type: WorkspaceType,
        stage: String,
        superpowers_enabled: bool,
        openspec_enabled: bool,
        messages: Vec<WsMessageDto>,
        checkpoints: Vec<WsCheckpointDto>,
        artifact: Option<ArtifactPayload>,
        providers: WsProviderConfig,
        timeline_nodes: Vec<TimelineNode>,
        active_node_id: Option<String>,
        artifact_versions: Vec<ArtifactVersion>,
        artifact_version_summaries: Vec<ArtifactVersionSummary>,
        timeline_node_details: HashMap<String, NodeDetail>,
        timeline_node_summaries: HashMap<String, NodeDetailSummary>,
        active_run_id: Option<String>,
        human_presentation_revisions: Vec<HumanPresentationRevision>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reviewer_enabled_at_start: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recoverable_interrupted_run: Option<RecoverableInterruptedRun>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan_repair: Option<Box<PlanRepairSessionSnapshotDto>>,
        session_status: WorkspaceSessionStatus,
        flow_kind: WorkItemPlanFlowKind,
        run_policy: RunPolicy,
        run_history: RunHistory,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        review_invocation_scope: Option<Box<ReviewInvocationScope>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        human_gate_snapshot: Option<HumanGateSnapshot>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repair_reservation: Option<Box<RepairReservation>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        policy_diagnostics: Vec<PolicyDiagnostic>,
        #[serde(default)]
        provider_start_ledger: Vec<ProviderStartLedgerEntry>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        single_candidate_phase: Option<crate::product::models::SingleCandidatePhase>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        work_item_plan_source_revision_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan_candidate_ir_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mechanical_report_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        publication_provenance_ref: Option<String>,
        /// F-27：挂起 choice 全量投影（provider pending + TextFallback pending），
        /// 空时省略保持 wire 兼容；元素与 choice_request 帧字段同构。
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pending_choice_requests: Vec<WsPendingChoiceRequest>,
    },
    Error {
        message: String,
    },
    ProtocolError {
        code: String,
        message: String,
        context: Option<serde_json::Value>,
    },
    ProviderLocked {
        snapshot: ProviderConfigSnapshot,
        locked_at: String,
    },
    /// attachment 直播队列溢出；客户端必须关闭该连接并带 cursor 重连以恢复一致状态。
    ResyncRequired {
        event_seq: u64,
    },
    Pong,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::product::models::ProviderName;
    use crate::product::work_item_plan_policy::{
        ProviderStartLedgerEntry, RunHistory, RunPolicy, WorkItemPlanFlowKind,
    };

    fn work_item_plan_session_state() -> WsOutMessage {
        WsOutMessage::SessionState {
            connection_id: None,
            session_id: "session_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            stage: "prepare_context".to_string(),
            superpowers_enabled: false,
            openspec_enabled: false,
            messages: Vec::new(),
            checkpoints: Vec::new(),
            artifact: None,
            providers: WsProviderConfig {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
            },
            timeline_nodes: Vec::new(),
            active_node_id: None,
            artifact_versions: Vec::new(),
            artifact_version_summaries: Vec::new(),
            timeline_node_details: HashMap::new(),
            timeline_node_summaries: HashMap::new(),
            active_run_id: None,
            human_presentation_revisions: Vec::new(),
            reviewer_enabled_at_start: None,
            recoverable_interrupted_run: None,
            plan_repair: None,
            session_status: WorkspaceSessionStatus::StoppedNeedsHuman,
            flow_kind: WorkItemPlanFlowKind::SingleCandidate,
            run_policy: RunPolicy::AutoIfValid,
            run_history: RunHistory::default(),
            review_invocation_scope: None,
            human_gate_snapshot: None,
            repair_reservation: None,
            policy_diagnostics: Vec::new(),
            provider_start_ledger: Vec::new(),
            single_candidate_phase: Some(crate::product::models::SingleCandidatePhase::Evaluate),
            work_item_plan_source_revision_ref: Some("source-ref".to_string()),
            plan_candidate_ir_ref: Some("ir-ref".to_string()),
            mechanical_report_ref: Some("report-ref".to_string()),
            publication_provenance_ref: Some("provenance-ref".to_string()),
            pending_choice_requests: Vec::new(),
        }
    }

    #[test]
    fn session_state_serializes_work_item_plan_durable_fields() {
        let value = serde_json::to_value(work_item_plan_session_state()).unwrap();

        assert_eq!(value["session_status"], "stopped_needs_human");
        assert_eq!(value["flow_kind"], "single_candidate");
        assert_eq!(value["run_policy"], "auto_if_valid");
        assert_eq!(value["provider_start_ledger"], serde_json::json!([]));
        assert_eq!(
            value["run_history"],
            serde_json::json!({
                "seen_fingerprints": [],
                "repairs_used": 0,
                "manual_repairs_used": 0,
                "transitions_used": 0,
                "initial_review_count": 0,
                "verification_review_count": 0,
                "review_cycles": {},
            })
        );
    }

    /// F-27 wire 契约：空投影省略字段（旧客户端零变化）；非空投影元素与
    /// choice_request 帧字段同构。
    #[test]
    fn session_state_omits_empty_pending_choice_requests() {
        let value = serde_json::to_value(work_item_plan_session_state()).unwrap();

        assert!(
            value.get("pending_choice_requests").is_none(),
            "empty pending projection must be omitted from the wire"
        );
    }

    #[test]
    fn session_state_serializes_pending_choice_requests_isomorphic_to_choice_frame() {
        let mut message = work_item_plan_session_state();
        let WsOutMessage::SessionState {
            pending_choice_requests,
            ..
        } = &mut message
        else {
            unreachable!("fixture must be a session_state message");
        };
        pending_choice_requests.push(WsPendingChoiceRequest {
            id: "choice_0001".to_string(),
            prompt: "继续方式？".to_string(),
            options: vec![ChoiceOption {
                id: "opt_0".to_string(),
                label: "继续 author".to_string(),
                description: None,
            }],
            allow_multiple: false,
            allow_free_text: true,
            questions: Vec::new(),
            source: "text_fallback".to_string(),
        });

        let value = serde_json::to_value(message).unwrap();

        assert_eq!(
            value["pending_choice_requests"],
            serde_json::json!([{
                "id": "choice_0001",
                "prompt": "继续方式？",
                "options": [{ "id": "opt_0", "label": "继续 author", "description": null }],
                "allow_multiple": false,
                "allow_free_text": true,
                "questions": [],
                "source": "text_fallback",
            }])
        );
    }
    #[test]
    fn session_state_preserves_nonempty_provider_start_ledger() {
        let mut message = work_item_plan_session_state();
        let WsOutMessage::SessionState {
            provider_start_ledger,
            ..
        } = &mut message
        else {
            unreachable!("fixture must be a session_state message");
        };
        provider_start_ledger.push(ProviderStartLedgerEntry {
            provider_start_idempotency_key: "start:author:round-1".to_string(),
            started: true,
            provider: None,
            started_at: None,
        });
    }
    #[test]
    fn session_state_serializes_optional_connection_id() {
        let mut message = work_item_plan_session_state();
        let WsOutMessage::SessionState { connection_id, .. } = &mut message else {
            unreachable!("fixture must be a session_state message");
        };
        *connection_id = Some("conn-1".to_string());

        let value = serde_json::to_value(message).unwrap();

        assert_eq!(value["connection_id"], "conn-1");
    }
}
