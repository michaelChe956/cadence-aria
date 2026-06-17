use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::product::models::{NodeDetail, ProviderName, WorkspaceType};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStage {
    PrepareContext,
    Running,
    AuthorConfirm,
    CrossReview,
    ReviewDecision,
    Revision,
    HumanConfirm,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingWorkspaceStage {
    PrepareContext,
    PlanGeneration,
    PlanConfirm,
    Coding,
    Testing,
    CodeReview,
    Rework,
    HumanConfirm,
    Completed,
}

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
        markdown: String,
        diff: Option<String>,
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
    },
    ReviewDecisionRequired {
        node_id: String,
        round: u32,
        options: Vec<String>,
    },
    SessionState {
        session_id: String,
        workspace_type: WorkspaceType,
        stage: String,
        superpowers_enabled: bool,
        openspec_enabled: bool,
        messages: Vec<WsMessageDto>,
        checkpoints: Vec<WsCheckpointDto>,
        artifact: Option<String>,
        providers: WsProviderConfig,
        timeline_nodes: Vec<TimelineNode>,
        active_node_id: Option<String>,
        artifact_versions: Vec<ArtifactVersion>,
        artifact_version_summaries: Vec<ArtifactVersionSummary>,
        timeline_node_details: HashMap<String, NodeDetail>,
        timeline_node_summaries: HashMap<String, NodeDetailSummary>,
        active_run_id: Option<String>,
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
    Pong,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsInMessage {
    UserMessage {
        content: String,
    },
    ContextNote {
        content: String,
    },
    StartGeneration {
        provider_config: ProviderConfigSnapshot,
        reviewer_enabled: bool,
    },
    Hello {
        session_id: String,
        last_seen_node_id: Option<String>,
    },
    Rollback {
        checkpoint_id: String,
    },
    Confirm,
    ProviderSelect {
        role: String,
        provider: ProviderName,
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
    ReviewDecisionResponse {
        decision: String,
        extra_context: Option<String>,
    },
    AuthorDecision {
        decision: AuthorDecision,
    },
    SelectRevisionPath {
        path: RevisionPath,
        extra_context: Option<String>,
    },
    RequestRevision {
        feedback: StructuredFeedback,
    },
    HumanConfirm {
        decision: HumanConfirmDecision,
        payload: Option<serde_json::Value>,
    },
    RevertWorkItem {
        work_item_id: String,
        feedback: Option<String>,
        clear: bool,
    },
    Abort,
    Ping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RevisionPath {
    Revise,
    ReviseWithContext,
    SkipToHuman,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HumanConfirmDecision {
    Confirm,
    RequestChange,
    Terminate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthorDecision {
    Accept,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredFeedback {
    pub feedback_types: Vec<String>,
    pub description: String,
    pub target_artifact_version: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsPermissionRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsProviderStatus {
    Starting,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsExecutionEventKind {
    Provider,
    Turn,
    Command,
    Output,
    Artifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsExecutionEventStatus {
    Started,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsExecutionEvent {
    pub event_id: String,
    pub node_id: Option<String>,
    pub agent: Option<ProviderName>,
    pub kind: WsExecutionEventKind,
    pub status: WsExecutionEventStatus,
    pub title: String,
    pub detail: Option<String>,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub output: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDefaults {
    pub reviewer: ProviderName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsMessageDto {
    pub id: String,
    pub role: String,
    pub content: String,
    pub checkpoint_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsCheckpointDto {
    pub id: String,
    pub message_index: u32,
    pub stage: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsProviderConfig {
    pub author: ProviderName,
    pub reviewer: Option<ProviderName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArtifactPayload {
    Markdown {
        markdown: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
    },
    WorkItemPlanCandidate {
        candidate: Box<WorkItemPlanCandidateDto>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanCandidateDto {
    pub plan: WorkItemPlanDto,
    pub work_items: Vec<WorkItemCandidateDto>,
    pub verification_plans: Vec<VerificationPlanDto>,
    pub repository_profile: Option<RepositoryProfileDto>,
    pub validator_findings: Vec<ValidatorFindingDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanDto {
    pub id: String,
    pub status: String,
    pub options: WorkItemSplitOptionsDto,
    pub dependency_graph: Vec<WorkItemDependencyEdgeDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemSplitOptionsDto {
    pub include_integration_tests: bool,
    pub include_e2e_tests: bool,
    pub force_frontend_backend_split: bool,
    pub require_execution_plan_confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemDependencyEdgeDto {
    pub from_work_item_id: String,
    pub to_work_item_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemCandidateDto {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub depends_on: Vec<String>,
    pub exclusive_write_scopes: Vec<String>,
    pub verification_plan_ref: Option<String>,
    pub meta: WorkItemCandidateMetaDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemCandidateMetaDto {
    #[serde(default)]
    pub reverted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revert_feedback: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ValidatorFindingDto {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub work_item_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerificationPlanDto {
    pub plan_ref: String,
    pub scope: String,
    pub commands: Vec<VerificationCommandDto>,
    pub manual_checks: Vec<VerificationManualCheckDto>,
    pub required_gates: Vec<String>,
    pub risk_notes: Vec<String>,
    pub confidence: String,
    pub fallback_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerificationCommandDto {
    pub label: String,
    pub command: String,
    pub cwd: String,
    pub purpose: String,
    pub required: bool,
    pub timeout_seconds: u64,
    pub safety: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerificationManualCheckDto {
    pub label: String,
    pub instructions: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryProfileDto {
    pub profile_id: String,
    pub repository_id: String,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub package_managers: Vec<String>,
    pub test_frameworks: Vec<String>,
    pub build_systems: Vec<String>,
    pub detected_layers: Vec<String>,
    pub split_recommendation: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineNodeType {
    PrepareContext,
    ContextNote,
    StartGeneration,
    AuthorConfirm,
    #[serde(alias = "generation")]
    AuthorRun,
    #[serde(alias = "review")]
    ReviewerRun,
    ReviewDecision,
    Revision,
    HumanConfirm,
    AbortedByDisconnect,
    ProtocolError,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineNodeStatus {
    Active,
    Paused,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfigSnapshot {
    pub author: ProviderName,
    pub reviewer: Option<ProviderName>,
    pub review_rounds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineNode {
    pub node_id: String,
    pub node_type: TimelineNodeType,
    pub agent: Option<ProviderName>,
    pub stage: WorkspaceStage,
    pub round: Option<u32>,
    pub status: TimelineNodeStatus,
    pub title: String,
    pub summary: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub artifact_ref: Option<String>,
    pub provider_config_snapshot: ProviderConfigSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdictType {
    Pass,
    Revise,
    NeedsHuman,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFindingSeverity {
    Blocking,
    MustFix,
    StrongRecommendFix,
    Suggestion,
    Minor,
    Optional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub severity: ReviewFindingSeverity,
    pub message: String,
    pub evidence: String,
    pub impact: String,
    pub required_action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewGate {
    RequiresRevision,
    UserConfirmAllowed,
    UserTriageRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewVerdict {
    pub verdict: ReviewVerdictType,
    pub comments: String,
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    #[serde(default = "default_review_gate")]
    pub review_gate: ReviewGate,
}

fn default_review_gate() -> ReviewGate {
    ReviewGate::UserConfirmAllowed
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDecision {
    pub decision: String,
    pub extra_context: Option<String>,
    pub decided_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersion {
    pub version: u32,
    pub markdown: String,
    pub generated_by: ProviderName,
    pub reviewed_by: Option<ProviderName>,
    pub review_verdict: Option<ReviewVerdictType>,
    pub confirmed_by: Option<String>,
    #[serde(default = "default_true")]
    pub is_current: bool,
    pub created_at: String,
    pub source_node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDetailSummary {
    pub node_id: String,
    pub node_type: String,
    pub status: String,
    pub agent_role: Option<String>,
    pub provider_name: Option<String>,
    pub prompt_size: usize,
    pub prompt_preview: Option<String>,
    pub stream_size: usize,
    pub stream_preview: Option<String>,
    pub execution_event_count: usize,
    pub has_large_outputs: bool,
    pub artifact_ref: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersionSummary {
    pub version: u32,
    pub generated_by: ProviderName,
    pub reviewed_by: Option<ProviderName>,
    pub review_verdict: Option<ReviewVerdictType>,
    pub confirmed_by: Option<String>,
    pub is_current: bool,
    pub created_at: String,
    pub source_node_id: String,
    pub markdown_size: usize,
    pub markdown_preview: String,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{
        ArtifactPayload, ChoiceOption, ProviderConfigSnapshot, RepositoryProfileDto, ReviewGate,
        ReviewVerdict, ReviewVerdictType, TimelineNode, TimelineNodeStatus, TimelineNodeType,
        ValidatorFindingDto, VerificationCommandDto, VerificationManualCheckDto,
        VerificationPlanDto, WorkItemCandidateDto, WorkItemCandidateMetaDto,
        WorkItemDependencyEdgeDto, WorkItemPlanCandidateDto, WorkItemPlanDto,
        WorkItemSplitOptionsDto, WorkspaceStage, WsExecutionEvent, WsExecutionEventKind,
        WsExecutionEventStatus, WsInMessage, WsOutMessage, WsPermissionRiskLevel, WsProviderStatus,
    };
    use crate::product::models::{ProviderName, WorkspaceType};

    #[test]
    fn permission_messages_use_snake_case_type_tags() {
        let out = WsOutMessage::PermissionRequest {
            id: "perm_001".to_string(),
            tool_name: "bash".to_string(),
            description: "Run cargo test".to_string(),
            risk_level: WsPermissionRiskLevel::Medium,
        };
        let value = serde_json::to_value(out).unwrap();
        assert_eq!(value["type"], "permission_request");
        assert_eq!(value["risk_level"], "medium");

        let status = WsOutMessage::ProviderStatus {
            status: WsProviderStatus::WaitingApproval,
        };
        let value = serde_json::to_value(status).unwrap();
        assert_eq!(value["type"], "provider_status");
        assert_eq!(value["status"], "waiting_approval");

        let input: WsInMessage = serde_json::from_value(serde_json::json!({
            "type": "permission_response",
            "id": "perm_001",
            "approved": true,
            "reason": null
        }))
        .unwrap();

        assert!(matches!(
            input,
            WsInMessage::PermissionResponse { approved: true, .. }
        ));
    }

    #[test]
    fn permission_message_values_are_constrained() {
        let invalid_risk: Result<WsOutMessage, _> = serde_json::from_value(serde_json::json!({
            "type": "permission_request",
            "id": "perm_001",
            "tool_name": "bash",
            "description": "Run cargo test",
            "risk_level": "critical"
        }));
        assert!(invalid_risk.is_err());

        let invalid_status: Result<WsOutMessage, _> = serde_json::from_value(serde_json::json!({
            "type": "provider_status",
            "status": "ready"
        }));
        assert!(invalid_status.is_err());
    }

    #[test]
    fn execution_event_messages_use_snake_case_type_tags() {
        let out = WsOutMessage::ExecutionEvent {
            event: WsExecutionEvent {
                event_id: "command_cmd_001".to_string(),
                node_id: Some("node_generation_001".to_string()),
                agent: Some(ProviderName::ClaudeCode),
                kind: WsExecutionEventKind::Command,
                status: WsExecutionEventStatus::Completed,
                title: "Command completed".to_string(),
                detail: Some("exit code 0".to_string()),
                command: Some("pwd".to_string()),
                cwd: Some("/tmp/repo".to_string()),
                output: Some("/tmp/repo\n".to_string()),
                exit_code: Some(0),
            },
        };

        let value = serde_json::to_value(out).unwrap();
        assert_eq!(value["type"], "execution_event");
        assert_eq!(value["event"]["kind"], "command");
        assert_eq!(value["event"]["status"], "completed");
        assert_eq!(value["event"]["node_id"], "node_generation_001");
        assert_eq!(value["event"]["agent"], "claude_code");
        assert_eq!(value["event"]["command"], "pwd");
        assert_eq!(value["event"]["cwd"], "/tmp/repo");
    }

    #[test]
    fn workspace_stage_supports_review_decision_and_revision() {
        let decision = serde_json::to_value(WorkspaceStage::ReviewDecision).unwrap();
        let revision = serde_json::to_value(WorkspaceStage::Revision).unwrap();

        assert_eq!(decision, "review_decision");
        assert_eq!(revision, "revision");
    }

    #[test]
    fn timeline_messages_include_node_identity() {
        let node = TimelineNode {
            node_id: "node_review_001".to_string(),
            node_type: TimelineNodeType::ReviewerRun,
            agent: Some(ProviderName::Codex),
            stage: WorkspaceStage::CrossReview,
            round: Some(1),
            status: TimelineNodeStatus::Active,
            title: "Review Round 1".to_string(),
            summary: None,
            started_at: "2026-05-19T00:00:00Z".to_string(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: Some("version_0001".to_string()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 2,
            },
        };

        let created =
            serde_json::to_value(WsOutMessage::TimelineNodeCreated { node: node.clone() }).unwrap();
        assert_eq!(created["type"], "timeline_node_created");
        assert_eq!(created["node"]["node_type"], "reviewer_run");
        assert_eq!(created["node"]["status"], "active");
        assert_eq!(created["node"]["agent"], "codex");

        let chunk = serde_json::to_value(WsOutMessage::StreamChunk {
            role: "assistant".to_string(),
            content: "reviewing".to_string(),
            node_id: Some("node_review_001".to_string()),
        })
        .unwrap();
        assert_eq!(chunk["type"], "stream_chunk");
        assert_eq!(chunk["node_id"], "node_review_001");

        let complete = serde_json::to_value(WsOutMessage::MessageComplete {
            message_id: "msg_002".to_string(),
            checkpoint_id: "checkpoint_001".to_string(),
            node_id: Some("node_review_001".to_string()),
        })
        .unwrap();
        assert_eq!(complete["type"], "message_complete");
        assert_eq!(complete["node_id"], "node_review_001");
    }

    #[test]
    fn review_messages_and_session_state_serialize_as_contract() {
        let verdict = ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "需要补充验收标准".to_string(),
            summary: "补充验收标准后返修".to_string(),
            findings: vec![super::ReviewFinding {
                severity: super::ReviewFindingSeverity::MustFix,
                message: "缺少验收标准".to_string(),
                evidence: "Artifact 未列出验收标准".to_string(),
                impact: "无法进入下一阶段".to_string(),
                required_action: "补充验收标准".to_string(),
            }],
            review_gate: ReviewGate::UserTriageRequired,
        };

        let review_complete = serde_json::to_value(WsOutMessage::ReviewComplete {
            node_id: "node_review_001".to_string(),
            round: 1,
            verdict: verdict.verdict.clone(),
            comments: verdict.comments.clone(),
            summary: verdict.summary.clone(),
            findings: verdict.findings.clone(),
            review_gate: verdict.review_gate.clone(),
        })
        .unwrap();
        assert_eq!(review_complete["type"], "review_complete");
        assert_eq!(review_complete["verdict"], "revise");
        assert_eq!(review_complete["review_gate"], "user_triage_required");
        assert_eq!(review_complete["findings"][0]["severity"], "must_fix");

        let input: WsInMessage = serde_json::from_value(serde_json::json!({
            "type": "review_decision_response",
            "decision": "continue_with_context",
            "extra_context": "请补充边界条件"
        }))
        .unwrap();
        assert!(matches!(
            input,
            WsInMessage::ReviewDecisionResponse {
                decision,
                extra_context: Some(_),
            } if decision == "continue_with_context"
        ));

        let state = serde_json::to_value(WsOutMessage::SessionState {
            session_id: "workspace_session_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            stage: "review_decision".to_string(),
            superpowers_enabled: true,
            openspec_enabled: true,
            messages: Vec::new(),
            checkpoints: Vec::new(),
            artifact: Some("# Story".to_string()),
            providers: super::WsProviderConfig {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
            },
            timeline_nodes: Vec::new(),
            active_node_id: Some("node_review_decision_001".to_string()),
            artifact_versions: Vec::new(),
            artifact_version_summaries: Vec::new(),
            timeline_node_details: std::collections::HashMap::new(),
            timeline_node_summaries: std::collections::HashMap::new(),
            active_run_id: None,
        })
        .unwrap();
        assert_eq!(state["type"], "session_state");
        assert_eq!(state["active_node_id"], "node_review_decision_001");
        assert_eq!(state["superpowers_enabled"], true);
        assert_eq!(state["openspec_enabled"], true);
        assert_eq!(state["timeline_nodes"].as_array().unwrap().len(), 0);
        assert_eq!(state["artifact_versions"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn context_note_roundtrip() {
        let msg = WsInMessage::ContextNote {
            content: "需要支持空查询参数兜底".to_string(),
        };

        let json = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["type"], "context_note");
        assert_eq!(json["content"], "需要支持空查询参数兜底");
        let back: WsInMessage = serde_json::from_value(json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn start_generation_roundtrip() {
        let snapshot = ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
        };
        let msg = WsInMessage::StartGeneration {
            provider_config: snapshot,
            reviewer_enabled: true,
        };

        let json = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["type"], "start_generation");
        assert_eq!(json["reviewer_enabled"], true);
        let back: WsInMessage = serde_json::from_value(json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn protocol_error_outbound_roundtrip() {
        let msg = WsOutMessage::ProtocolError {
            code: "INVALID_MESSAGE_FOR_STAGE".to_string(),
            message: "context_note not allowed in Running".to_string(),
            context: Some(serde_json::json!({"stage": "Running"})),
        };

        let json = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["type"], "protocol_error");
        let back: WsOutMessage = serde_json::from_value(json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn provider_locked_roundtrip() {
        let msg = WsOutMessage::ProviderLocked {
            snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
            },
            locked_at: "2026-05-20T14:35:00Z".to_string(),
        };

        let json = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["type"], "provider_locked");
        let back: WsOutMessage = serde_json::from_value(json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn choice_request_and_response_roundtrip() {
        let out = WsOutMessage::ChoiceRequest {
            id: "choice_001".to_string(),
            prompt: "请选择下一步".to_string(),
            options: vec![
                ChoiceOption {
                    id: "continue".to_string(),
                    label: "继续".to_string(),
                    description: Some("继续当前方案".to_string()),
                },
                ChoiceOption {
                    id: "stop".to_string(),
                    label: "停止".to_string(),
                    description: None,
                },
            ],
            allow_multiple: false,
            allow_free_text: true,
            source: "ask_user_question".to_string(),
        };

        let json = serde_json::to_value(&out).unwrap();

        assert_eq!(json["type"], "choice_request");
        assert_eq!(json["source"], "ask_user_question");
        assert_eq!(json["options"][0]["id"], "continue");
        let back: WsOutMessage = serde_json::from_value(json).unwrap();
        assert_eq!(back, out);

        let input = WsInMessage::ChoiceResponse {
            id: "choice_001".to_string(),
            selected_option_ids: vec!["continue".to_string()],
            free_text: Some("补充说明".to_string()),
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["type"], "choice_response");
        assert_eq!(json["selected_option_ids"][0], "continue");
        let back: WsInMessage = serde_json::from_value(json).unwrap();
        assert_eq!(back, input);
    }

    #[test]
    fn hello_ping_roundtrip() {
        let hello = WsInMessage::Hello {
            session_id: "sess-1".to_string(),
            last_seen_node_id: Some("node-1".to_string()),
        };

        let json = serde_json::to_value(&hello).unwrap();

        assert_eq!(json["type"], "hello");
        let back: WsInMessage = serde_json::from_value(json).unwrap();
        assert_eq!(back, hello);

        let ping = WsInMessage::Ping;
        let json = serde_json::to_value(&ping).unwrap();
        assert_eq!(json["type"], "ping");
    }

    #[test]
    fn timeline_node_type_rename_keeps_legacy_deserialization_aliases() {
        let author = TimelineNodeType::AuthorRun;
        let json = serde_json::to_value(&author).unwrap();
        assert_eq!(json, "author_run");
        let legacy: TimelineNodeType = serde_json::from_value(serde_json::json!("generation"))
            .expect("legacy generation value should deserialize");
        assert_eq!(legacy, TimelineNodeType::AuthorRun);

        let reviewer = TimelineNodeType::ReviewerRun;
        let json = serde_json::to_value(&reviewer).unwrap();
        assert_eq!(json, "reviewer_run");
        let legacy: TimelineNodeType = serde_json::from_value(serde_json::json!("review"))
            .expect("legacy review value should deserialize");
        assert_eq!(legacy, TimelineNodeType::ReviewerRun);
    }

    #[test]
    fn work_item_plan_candidate_dto_roundtrips_through_serde() {
        let dto = WorkItemPlanCandidateDto {
            plan: WorkItemPlanDto {
                id: "issue_work_item_plan_0001".to_string(),
                status: "draft".to_string(),
                options: WorkItemSplitOptionsDto {
                    include_integration_tests: true,
                    include_e2e_tests: false,
                    force_frontend_backend_split: true,
                    require_execution_plan_confirm: false,
                },
                dependency_graph: vec![WorkItemDependencyEdgeDto {
                    from_work_item_id: "wi_001".to_string(),
                    to_work_item_id: "wi_002".to_string(),
                }],
            },
            work_items: vec![WorkItemCandidateDto {
                id: "wi_001".to_string(),
                kind: "backend".to_string(),
                title: "实现爬楼梯问题".to_string(),
                depends_on: vec!["wi_000".to_string()],
                exclusive_write_scopes: vec!["src/product/stairs.rs".to_string()],
                verification_plan_ref: Some("vp_001".to_string()),
                meta: WorkItemCandidateMetaDto {
                    reverted: true,
                    revert_feedback: Some("需要细化边界条件".to_string()),
                },
            }],
            verification_plans: vec![VerificationPlanDto {
                plan_ref: "vp_001".to_string(),
                scope: "unit".to_string(),
                commands: vec![VerificationCommandDto {
                    label: "cargo test".to_string(),
                    command: "cargo test".to_string(),
                    cwd: "".to_string(),
                    purpose: "unit tests".to_string(),
                    required: true,
                    timeout_seconds: 120,
                    safety: "approved".to_string(),
                }],
                manual_checks: vec![VerificationManualCheckDto {
                    label: "人工检查".to_string(),
                    instructions: "检查输出".to_string(),
                    required: false,
                }],
                required_gates: vec![],
                risk_notes: vec![],
                confidence: "high".to_string(),
                fallback_policy: "manual_gate".to_string(),
            }],
            repository_profile: Some(RepositoryProfileDto {
                profile_id: "rp_001".to_string(),
                repository_id: "repo_001".to_string(),
                languages: vec!["rust".to_string()],
                frameworks: vec![],
                package_managers: vec!["cargo".to_string()],
                test_frameworks: vec![],
                build_systems: vec!["cargo".to_string()],
                detected_layers: vec!["backend".to_string()],
                split_recommendation: "backend_only".to_string(),
                confidence: "high".to_string(),
            }),
            validator_findings: vec![ValidatorFindingDto {
                severity: "warning".to_string(),
                code: "W001".to_string(),
                message: "注意边界条件".to_string(),
                work_item_ids: vec!["wi_001".to_string()],
            }],
        };

        let json = serde_json::to_value(&dto).unwrap();
        let back: WorkItemPlanCandidateDto = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back, dto);

        // 显式断言 plan 文档约定的字段路径
        assert_eq!(json["plan"]["id"], "issue_work_item_plan_0001");
        assert_eq!(json["plan"]["status"], "draft");
        assert_eq!(json["work_items"][0]["id"], "wi_001");
        assert_eq!(json["work_items"][0]["kind"], "backend");
        assert_eq!(json["work_items"][0]["verification_plan_ref"], "vp_001");
        assert_eq!(json["work_items"][0]["meta"]["reverted"], true);
        assert_eq!(
            json["work_items"][0]["meta"]["revert_feedback"],
            "需要细化边界条件"
        );
        assert!(json["verification_plans"][0]["plan_ref"] == "vp_001");
        assert!(json["repository_profile"]["profile_id"] == "rp_001");
        assert!(json["validator_findings"][0]["code"] == "W001");
    }

    #[test]
    fn revert_work_item_message_deserializes() {
        let input: WsInMessage = serde_json::from_value(serde_json::json!({
            "type": "revert_work_item",
            "work_item_id": "wi_001",
            "feedback": "需要回退",
            "clear": false
        }))
        .unwrap();

        assert!(matches!(
            input,
            WsInMessage::RevertWorkItem {
                work_item_id,
                feedback,
                clear,
            } if work_item_id == "wi_001" && feedback.as_deref() == Some("需要回退") && !clear
        ));
    }

    #[test]
    fn artifact_payload_markdown_variant_serializes_to_flat_json() {
        let payload = ArtifactPayload::Markdown {
            markdown: "# Plan\n".to_string(),
            diff: Some("@@ -1 +1 @@\n-old\n+new".to_string()),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["markdown"], "# Plan\n");
        assert_eq!(json["diff"], "@@ -1 +1 @@\n-old\n+new");

        let payload_without_diff = ArtifactPayload::Markdown {
            markdown: "# Plan\n".to_string(),
            diff: None,
        };
        let json_without_diff = serde_json::to_value(&payload_without_diff).unwrap();
        assert_eq!(
            json_without_diff,
            serde_json::json!({"markdown": "# Plan\n"})
        );
    }

    #[test]
    fn artifact_payload_candidate_variant_serializes_to_flat_json() {
        let payload = ArtifactPayload::WorkItemPlanCandidate {
            candidate: Box::new(WorkItemPlanCandidateDto {
                plan: WorkItemPlanDto {
                    id: "issue_work_item_plan_0001".to_string(),
                    status: "draft".to_string(),
                    options: WorkItemSplitOptionsDto {
                        include_integration_tests: false,
                        include_e2e_tests: false,
                        force_frontend_backend_split: false,
                        require_execution_plan_confirm: false,
                    },
                    dependency_graph: vec![],
                },
                work_items: vec![WorkItemCandidateDto {
                    id: "wi_001".to_string(),
                    kind: "backend".to_string(),
                    title: "实现爬楼梯问题".to_string(),
                    depends_on: vec![],
                    exclusive_write_scopes: vec!["src/product/stairs.rs".to_string()],
                    verification_plan_ref: None,
                    meta: WorkItemCandidateMetaDto {
                        reverted: false,
                        revert_feedback: None,
                    },
                }],
                verification_plans: vec![],
                repository_profile: None,
                validator_findings: vec![],
            }),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json.get("candidate").is_some());
        assert_eq!(json["candidate"]["plan"]["id"], "issue_work_item_plan_0001");
        assert_eq!(json["candidate"]["plan"]["status"], "draft");
        assert_eq!(json["candidate"]["work_items"][0]["id"], "wi_001");
        assert_eq!(
            json["candidate"]["work_items"][0]["meta"]["reverted"],
            false
        );
        assert!(!json.as_object().unwrap().contains_key("markdown"));
    }

    #[test]
    fn artifact_update_carries_candidate_payload_as_expected_json() {
        let candidate = WorkItemPlanCandidateDto {
            plan: WorkItemPlanDto {
                id: "issue_work_item_plan_0001".to_string(),
                status: "draft".to_string(),
                options: WorkItemSplitOptionsDto {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                dependency_graph: vec![],
            },
            work_items: vec![WorkItemCandidateDto {
                id: "wi_001".to_string(),
                kind: "backend".to_string(),
                title: "实现爬楼梯问题".to_string(),
                depends_on: vec![],
                exclusive_write_scopes: vec!["src/product/stairs.rs".to_string()],
                verification_plan_ref: None,
                meta: WorkItemCandidateMetaDto {
                    reverted: false,
                    revert_feedback: None,
                },
            }],
            verification_plans: vec![],
            repository_profile: None,
            validator_findings: vec![],
        };
        let out = WsOutMessage::ArtifactUpdate {
            version: 7,
            markdown: serde_json::to_string(&candidate).unwrap(),
            diff: None,
        };
        let json = serde_json::to_value(out).unwrap();
        assert_eq!(json["type"], "artifact_update");
        assert_eq!(json["version"], 7);
        let parsed_candidate: WorkItemPlanCandidateDto =
            serde_json::from_str(json["markdown"].as_str().unwrap()).unwrap();
        assert_eq!(parsed_candidate.plan.id, "issue_work_item_plan_0001");
        assert_eq!(parsed_candidate.work_items[0].id, "wi_001");
    }
}
