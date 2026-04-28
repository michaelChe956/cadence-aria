use serde::{Deserialize, Serialize};

use crate::protocol::constraints::OpenSpecBootstrapStatus;
use crate::protocol::policies::PolicyMode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskSummary {
    pub task_id: String,
    pub phase: String,
    pub change_id: String,
    pub effective_policy: PolicyMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskRuntimeState {
    pub task_id: String,
    pub phase: String,
    pub change_id: String,
    pub effective_policy: PolicyMode,
    pub intake_ref: String,
    pub risk_registry_ref: String,
    pub openspec_bootstrap_status: OpenSpecBootstrapStatus,
    pub protocol_steps: Vec<String>,
}

impl From<&TaskRuntimeState> for TaskSummary {
    fn from(task: &TaskRuntimeState) -> Self {
        Self {
            task_id: task.task_id.clone(),
            phase: task.phase.clone(),
            change_id: task.change_id.clone(),
            effective_policy: task.effective_policy.clone(),
        }
    }
}
