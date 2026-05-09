use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateTaskRequest {
    pub request_text: String,
    pub change_id: String,
    pub policy_preset: String,
    pub provider_mode: String,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateTaskResponse {
    pub task_id: String,
    pub session_id: String,
    pub change_id: String,
    pub phase: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PendingProviderStepDto {
    pub node_id: String,
    pub provider_type: String,
    pub runtime_role: String,
    pub adapter_role: String,
    pub prompt: String,
    pub input_summary: Value,
    pub canonical_input_refs: Vec<String>,
    pub context_files: Vec<String>,
    pub output_schema: String,
    pub allowed_write_scope: Vec<String>,
    pub forbidden_actions: Vec<String>,
    pub verification_commands: Vec<String>,
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AdvanceTaskResponse {
    Advanced {
        projection_version: u64,
    },
    PausedForApproval {
        pending_step: PendingProviderStepDto,
    },
    Completed {
        projection_version: u64,
    },
}

impl AdvanceTaskResponse {
    pub fn expect_pending_step(self) -> Option<PendingProviderStepDto> {
        match self {
            AdvanceTaskResponse::PausedForApproval { pending_step } => Some(pending_step),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConfirmTaskRequest {
    pub checkpoint_id: String,
    pub prompt: String,
    pub policy_override: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConfirmTaskResponse {
    pub status: String,
    pub node_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackPreviewRequest {
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WebEvent {
    pub cursor: u64,
    pub event_type: String,
    pub task_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskListResponse {
    pub tasks: Vec<TaskListItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskListItem {
    pub task_id: String,
    pub change_id: Option<String>,
    pub phase: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactContentResponse {
    pub artifact_ref: String,
    pub artifact_kind: String,
    pub producer_node: Option<String>,
    pub path: String,
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FileContentResponse {
    pub path: String,
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FileDiffResponse {
    pub base_checkpoint: String,
    pub path: String,
    pub diff: String,
}
