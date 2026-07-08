use serde::{Deserialize, Serialize};

use crate::product::models::ProviderName;

use super::common::{ChoiceAnswer, ProviderConfigSnapshot, StructuredFeedback};

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
        #[serde(default)]
        answers: Vec<ChoiceAnswer>,
    },
    ReviewDecisionResponse {
        decision: String,
        extra_context: Option<String>,
    },
    AuthorDecision {
        decision: AuthorDecision,
    },
    SelectWorkItemGenerationMode {
        mode: WorkItemGenerationModeDto,
    },
    SelectRevisionPath {
        path: RevisionPath,
        extra_context: Option<String>,
    },
    RequestRevision {
        feedback: StructuredFeedback,
    },
    RequestOutlineRevision {
        feedback: Option<String>,
    },
    WorkItemDraftDecision {
        outline_id: String,
        decision: WorkItemDraftDecisionDto,
        feedback: Option<String>,
    },
    WorkItemBatchDecision {
        decision: WorkItemBatchDecisionDto,
        feedback: Option<String>,
        first_affected_outline_id: Option<String>,
    },
    WorkItemPlanCompileRecoveryAction {
        action: WorkItemPlanCompileRecoveryActionDto,
        reason: Option<String>,
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
#[serde(rename_all = "snake_case")]
pub enum WorkItemGenerationModeDto {
    Serial,
    Batch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemDraftDecisionDto {
    Accept,
    Rewrite,
    Pause,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemBatchDecisionDto {
    AcceptAll,
    RewriteBatch,
    Pause,
    DowngradeToSerial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanCompileRecoveryActionDto {
    Continue,
    AbortAndRollback,
    HumanTriage,
}
