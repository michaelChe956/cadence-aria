use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssuePhase {
    Clarification,
    Development,
    Acceptance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Draft,
    InProgress,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBindingStatus {
    Created,
    Running,
    Completed,
    Blocked,
    Detached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Open,
    Confirmed,
    ChangeRequested,
    Terminated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateType {
    PolicyControlled,
    HardGate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Dropped,
    NeedsHuman,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    ClaudeCode,
    Codex,
    Fake,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryRecord {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub path: PathBuf,
    pub repo_hash: String,
    pub runtime_root: PathBuf,
    pub default_policy_preset: String,
    pub default_provider_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueRecord {
    pub id: String,
    pub project_id: String,
    pub repo_id: String,
    pub title: String,
    pub description: Option<String>,
    pub change_id: String,
    pub phase: IssuePhase,
    pub status: IssueStatus,
    pub active_binding_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueRuntimeBindingRecord {
    pub id: String,
    pub issue_id: String,
    pub repo_id: String,
    pub change_id: String,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub runtime_root: PathBuf,
    pub task_root: Option<PathBuf>,
    pub status: RuntimeBindingStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GateRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub binding_id: String,
    pub node_id: String,
    pub gate_type: GateType,
    pub status: GateStatus,
    pub artifact_refs: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub resolved_at: Option<String>,
    pub comment: Option<String>,
    pub requested_change: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExecutionRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub binding_id: String,
    pub node_id: String,
    pub status: ExecutionStatus,
    pub event_type: String,
    pub artifact_refs: Vec<String>,
    pub message: Option<String>,
    pub created_at: String,
}
