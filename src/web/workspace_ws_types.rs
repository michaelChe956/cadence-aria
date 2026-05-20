use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::product::models::{NodeDetail, ProviderName, WorkspaceType};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStage {
    PrepareContext,
    Running,
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
        messages: Vec<WsMessageDto>,
        checkpoints: Vec<WsCheckpointDto>,
        artifact: Option<String>,
        providers: WsProviderConfig,
        timeline_nodes: Vec<TimelineNode>,
        active_node_id: Option<String>,
        artifact_versions: Vec<ArtifactVersion>,
        timeline_node_details: HashMap<String, NodeDetail>,
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
    ReviewDecisionResponse {
        decision: String,
        extra_context: Option<String>,
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
#[serde(rename_all = "snake_case")]
pub enum TimelineNodeType {
    PrepareContext,
    ContextNote,
    StartGeneration,
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
pub struct ReviewVerdict {
    pub verdict: ReviewVerdictType,
    pub comments: String,
    pub summary: String,
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
    pub created_at: String,
    pub source_node_id: String,
}

#[cfg(test)]
mod tests {
    use super::{
        ProviderConfigSnapshot, ReviewVerdict, ReviewVerdictType, TimelineNode, TimelineNodeStatus,
        TimelineNodeType, WorkspaceStage, WsExecutionEvent, WsExecutionEventKind,
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
        };

        let review_complete = serde_json::to_value(WsOutMessage::ReviewComplete {
            node_id: "node_review_001".to_string(),
            round: 1,
            verdict: verdict.verdict.clone(),
            comments: verdict.comments.clone(),
            summary: verdict.summary.clone(),
        })
        .unwrap();
        assert_eq!(review_complete["type"], "review_complete");
        assert_eq!(review_complete["verdict"], "revise");

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
            timeline_node_details: std::collections::HashMap::new(),
            active_run_id: None,
        })
        .unwrap();
        assert_eq!(state["type"], "session_state");
        assert_eq!(state["active_node_id"], "node_review_decision_001");
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
}
