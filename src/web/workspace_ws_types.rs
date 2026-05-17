use serde::{Deserialize, Serialize};

use crate::product::models::{ProviderName, WorkspaceType};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStage {
    PrepareContext,
    Running,
    CrossReview,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsOutMessage {
    StreamChunk {
        role: String,
        content: String,
    },
    MessageComplete {
        message_id: String,
        checkpoint_id: String,
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
    SessionState {
        session_id: String,
        workspace_type: WorkspaceType,
        stage: String,
        messages: Vec<WsMessageDto>,
        checkpoints: Vec<WsCheckpointDto>,
        artifact: Option<String>,
        providers: WsProviderConfig,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsInMessage {
    UserMessage {
        content: String,
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
    Abort,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDefaults {
    pub reviewer: ProviderName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMessageDto {
    pub id: String,
    pub role: String,
    pub content: String,
    pub checkpoint_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsCheckpointDto {
    pub id: String,
    pub message_index: u32,
    pub stage: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsProviderConfig {
    pub author: ProviderName,
    pub reviewer: Option<ProviderName>,
}

#[cfg(test)]
mod tests {
    use super::{WsInMessage, WsOutMessage, WsPermissionRiskLevel, WsProviderStatus};

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
}
