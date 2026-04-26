pub mod intake_capture;
pub mod session_bootstrap;
pub mod task_init;

use serde_json::Value;
use std::future::Future;

#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalNodeInput {
    pub session_id: String,
    pub task_id: Option<String>,
    pub node_id: String,
    pub risk_registry_ref: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonContext {
    pub workspace_root: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeProtocolStep {
    pub node_id: String,
    pub status: RuntimeStepStatus,
    pub node_specific_fields: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStepStatus {
    Completed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeUnitResult {
    pub protocol_steps: Vec<RuntimeProtocolStep>,
    pub produced_artifact_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeUnitError {
    pub code: String,
    pub message: String,
}

pub trait RuntimeUnit {
    fn unit_id(&self) -> &'static str;

    fn covered_protocol_nodes(&self) -> Vec<&'static str>;

    fn execute(
        &self,
        input: CanonicalNodeInput,
        ctx: &DaemonContext,
    ) -> impl Future<Output = Result<RuntimeUnitResult, RuntimeUnitError>> + Send;
}

pub fn completed_step(node_id: &str) -> RuntimeProtocolStep {
    RuntimeProtocolStep {
        node_id: node_id.to_string(),
        status: RuntimeStepStatus::Completed,
        node_specific_fields: serde_json::json!({}),
    }
}
