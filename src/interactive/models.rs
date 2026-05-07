use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRunStatus {
    Started,
    Completed,
    Failed,
    Blocked,
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    Active,
    Superseded,
    Candidate,
    Rejected,
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Markdown,
    Json,
    Source,
    Test,
    Log,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskSession {
    pub session_id: String,
    pub task_id: String,
    pub created_at: String,
    pub status: String,
    pub turn_ids: Vec<String>,
    pub active_turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct InteractionTurn {
    pub turn_id: String,
    pub session_id: String,
    pub node_id: String,
    pub provider_type: String,
    pub prompt_snapshot: String,
    pub input_summary: Value,
    pub checkpoint_before: Option<String>,
    pub provider_run_id: Option<String>,
    pub output_artifact_refs: Vec<String>,
    pub changed_files: Vec<String>,
    pub status: TurnStatus,
    pub dropped: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NodeRun {
    pub node_run_id: String,
    pub node_id: String,
    pub turn_id: Option<String>,
    pub provider_run_id: Option<String>,
    pub input_refs: Vec<String>,
    pub output_schema: Option<String>,
    pub artifact_refs: Vec<String>,
    pub status: NodeRunStatus,
    pub duration_ms: Option<u64>,
    pub diagnostic_refs: Vec<String>,
    pub dropped: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeCheckpoint {
    pub checkpoint_id: String,
    pub task_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub git_head: Option<String>,
    pub dirty_summary: Value,
    pub state_snapshot_ref: String,
    pub projection_snapshot_ref: String,
    pub artifact_boundary: usize,
    pub provider_run_boundary: usize,
    pub node_run_boundary: usize,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactIndexEntry {
    pub artifact_ref: String,
    pub artifact_kind: String,
    pub producer_node: Option<String>,
    pub path: String,
    pub summary: String,
    pub status: ArtifactStatus,
    pub content_type: ContentType,
    pub traceability_refs: Vec<String>,
    pub dropped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceProjection {
    pub workspace_root: String,
    pub active_task_id: Option<String>,
    pub active_session_id: Option<String>,
    pub overview: Value,
    pub sessions: Vec<TaskSession>,
    pub timeline: Vec<Value>,
    pub artifact_index: Vec<ArtifactIndexEntry>,
    pub diagnostics: Vec<Value>,
    pub available_actions: Vec<String>,
}
