use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::product::models::ProviderName;
use crate::product::workspace_engine::LinkedWorkspaceAmendmentTarget;

use super::common::{ChoiceAnswer, ProviderConfigSnapshot, StructuredFeedback};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelloRole {
    Driver,
    Observer,
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
    RetryInterruptedRun {
        failed_node_id: String,
    },
    Hello {
        session_id: String,
        last_seen_node_id: Option<String>,
        #[serde(default)]
        role: Option<HelloRole>,
        #[serde(default)]
        after_event_seq: Option<u64>,
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
    RequestRevision {
        feedback: StructuredFeedback,
    },
    WorkItemPlanCompileRecoveryAction {
        action: WorkItemPlanCompileRecoveryActionDto,
        reason: Option<String>,
    },
    ConfirmPlanAmendment {
        amendment_id: String,
    },
    CancelPlanAmendment {
        amendment_id: String,
        reason: Option<String>,
    },
    StartLinkedWorkspaceAmendment {
        target: LinkedWorkspaceAmendmentTarget,
    },
    HumanGateFeedback {
        command_id: String,
        feedback: String,
    },
    Advance {
        command_id: String,
    },
    AbandonHumanGate {
        command_id: String,
    },
    Abort,
    Ping,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceInboundEnvelope {
    pub(crate) message: WsInMessage,
    pub(crate) submitted_fields: BTreeSet<String>,
}

impl From<WsInMessage> for WorkspaceInboundEnvelope {
    fn from(message: WsInMessage) -> Self {
        Self {
            message,
            submitted_fields: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanCompileRecoveryActionDto {
    Continue,
    AbortAndRollback,
    HumanTriage,
}
