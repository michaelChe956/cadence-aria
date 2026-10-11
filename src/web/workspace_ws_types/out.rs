use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::product::logical_codebase::provider_admission_preflight::BootstrapActionKind;
use crate::product::models::{
    HumanPresentationRevision, LcFreshOperationState, LcRecoveryRunContext,
    LcResumeRecoveryRecord, LcResumeRecoveryState, NodeDetail, PlanRepairSessionSnapshotDto,
    ProviderName, WorkspaceSessionStatus, WorkspaceType,
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

/// F-27：session_state 顶层挂起 choice 投影元素——与 `ChoiceRequest` 帧
/// 字段同构（同一 options/questions DTO），供前端收帧对账补挂 choice 卡。
/// F-59 增补：`created_at_ms`（provider pending 登记时刻，epoch ms——等待
/// 提示条已等待时长/901s 超时倒计时锚点，TextFallback 无等待界故缺省）与
/// `role`（发问角色 author/reviewer）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WsPendingChoiceRequest {
    pub id: String,
    pub prompt: String,
    pub options: Vec<ChoiceOption>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
    pub questions: Vec<ChoiceQuestion>,
    pub source: String,
    /// F-59：挂起起始时刻（epoch ms）；TextFallback pending 不携带。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at_ms: Option<u64>,
    /// F-59：发问角色（author/reviewer）；旧载荷缺省按 author 对账。
    #[serde(default = "default_pending_choice_role")]
    pub role: String,
    /// P0 1.3：应答须绑定的 run 化身（manager 注入；引擎投影/durable 基线
    /// 无活跃 run 时缺省 None——旧载荷兼容，前端不猜 run）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_run_id: Option<String>,
}

fn default_pending_choice_role() -> String {
    "author".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverableInterruptedRun {
    pub failed_node_id: String,
    pub operation: RecoverableInterruptedOperation,
    pub label: String,
}

/// A1c（Task 2.2）：显式 fresh operation 的受理/回放投递标记（设计 §5.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LcResumeRecoveryDelivery {
    Accepted,
    Replayed,
}

/// A1c（Task 2.2）：SessionState.lc_resume_recovery.last_operation 投影
/// （接口字典：operation_id/state/reason?/fresh_node?）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcFreshOperationDto {
    pub operation_id: String,
    pub state: LcFreshOperationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fresh_node: Option<String>,
}

/// A1c（Task 2.2）：LC 恢复对外纯投影（RecoveryDto）——最新 durable 事实的
/// 必要字段：recovery_id/state/reason_code/provider/failed_node_id/run_kind/
/// 时间/allowed_actions＋last_operation。native session ID、full fingerprint
/// 与秘密配置一律不下发；resolved 保留证据与 last_operation 供 Replay。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcResumeRecoveryDto {
    pub recovery_id: String,
    pub state: LcResumeRecoveryState,
    pub reason_code: String,
    pub provider: ProviderName,
    pub failed_node_id: String,
    pub run_kind: String,
    pub detected_at: String,
    pub updated_at: String,
    pub allowed_actions: Vec<BootstrapActionKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_operation: Option<LcFreshOperationDto>,
}

impl LcResumeRecoveryDto {
    /// durable `LcResumeRecoveryRecord` 的对外纯投影。last_operation 取
    /// record.operation_id 指向的已受理结果（缺锚时回退最新一条，证据不丢）。
    pub fn from_record(record: &LcResumeRecoveryRecord) -> Self {
        let last_operation = match record.operation_id.as_deref() {
            Some(current) => record
                .fresh_operations
                .iter()
                .rev()
                .find(|operation| operation.operation_id == current),
            None => record.fresh_operations.last(),
        }
        .map(|operation| LcFreshOperationDto {
            operation_id: operation.operation_id.clone(),
            state: operation.state,
            reason: operation.reason_code.clone(),
            fresh_node: operation.fresh_node_id.clone(),
        });
        Self {
            recovery_id: record.recovery_id.clone(),
            state: record.state,
            reason_code: record.reason_code.clone(),
            provider: record.provider.clone(),
            failed_node_id: record.failed_provider_node_id.clone(),
            run_kind: lc_recovery_run_kind(&record.run_context).to_string(),
            detected_at: record.detected_at.clone(),
            updated_at: record.updated_at.clone(),
            // A/B 发布纪律（设计 §4.5）：B0 与 covered-run fresh handler 接齐
            // 前一律空集——waiting 不投可启动半成品（Plan Task 2.6 判据：
            // waiting allowed 为空直到 B 齐），后端普通 generation/Revision/
            // retry 入口真实拒收兜底。
            allowed_actions: Vec::new(),
            last_operation,
        }
    }
}

/// run_context 的对外 kind 名与 durable serde tag（snake_case）严格一致，
/// 不引入第二套字符串协议；未知 kind 拒收由 durable 反序列化承载。
fn lc_recovery_run_kind(context: &LcRecoveryRunContext) -> &'static str {
    match context {
        LcRecoveryRunContext::Author { .. } => "author",
        LcRecoveryRunContext::AuthorChoiceFollowup { .. } => "author_choice_followup",
        LcRecoveryRunContext::AuthorFeedback { .. } => "author_feedback",
        LcRecoveryRunContext::ReviewerRevision { .. } => "reviewer_revision",
        LcRecoveryRunContext::LegacyOutlineAuthor { .. } => "legacy_outline_author",
        LcRecoveryRunContext::LegacyOutlineRevision { .. } => "legacy_outline_revision",
        LcRecoveryRunContext::LegacyOutlineRebuild { .. } => "legacy_outline_rebuild",
        LcRecoveryRunContext::LegacyDraft { .. } => "legacy_draft",
        LcRecoveryRunContext::LegacyBatch { .. } => "legacy_batch",
        LcRecoveryRunContext::LegacyPlanRevision { .. } => "legacy_plan_revision",
        LcRecoveryRunContext::ScAuthor { .. } => "sc_author",
        LcRecoveryRunContext::ScManualRevision { .. } => "sc_manual_revision",
    }
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
        /// P0 1.2（REQ-WIGA-08）：durable automation 归属投影；None=未知
        ///（读取失败/未注入），缺省省略保持 wire 兼容。引擎自建帧默认 client，
        /// 对外帧由 manager 以 durable 事实覆盖。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        automation: Option<crate::product::models::automation::AutomationOwnership>,
        /// A1c（Task 2.2）：durable LC 恢复纯投影（waiting/fresh/resolved 主
        /// 状态＋last_operation）；空/旧 wire 缺省省略。resolved 保留证据供
        /// Replay；allowed_actions 在 B 波接齐前恒空（A/B 发布纪律）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lc_resume_recovery: Option<LcResumeRecoveryDto>,
    },
    /// A1c（Task 2.2）：显式 fresh operation 的 typed 受理/回放结果（设计
    /// §5.1）。接受/回放同形态；operation_state 取 durable 事实，不从客户
    /// 端乐观态猜；无 native/秘密字段。结果同时随 authoritative
    /// SessionState.lc_resume_recovery.last_operation 供 observer/重连对账。
    LcResumeRecoveryOperationResult {
        recovery_id: String,
        operation_id: String,
        delivery: LcResumeRecoveryDelivery,
        operation_state: LcFreshOperationState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fresh_node_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason_code: Option<String>,
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
            automation: None,
            lc_resume_recovery: None,
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
            created_at_ms: None,
            role: "author".to_string(),
            expected_run_id: None,
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
                // F-59：role 恒序列化（旧载荷缺省 author）；created_at_ms
                // None 时省略（TextFallback 无等待界）。
                "role": "author",
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

        let value = serde_json::to_value(message).unwrap();

        assert_eq!(
            value["provider_start_ledger"],
            serde_json::json!([{
                "provider_start_idempotency_key": "start:author:round-1",
                "started": true,
            }])
        );
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
