use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::protocol::artifacts::RiskEntry;
use crate::protocol::nodes::is_protocol_node_id;
use crate::protocol::policies::PolicyMode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RiskRegistrySnapshot {
    pub risk_registry_ref: String,
    pub risk_ids: Vec<String>,
    #[serde(default)]
    pub risks: Vec<RiskEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeSnapshot {
    pub snapshot_id: String,
    pub session_id: String,
    pub task_id: String,
    pub node_id: String,
    pub phase: String,
    pub timestamp: String,
    pub effective_policy: PolicyMode,
    pub artifact_refs: Vec<String>,
    pub provider_run_refs: Vec<String>,
    pub worktree_ref: Option<String>,
    pub rework_counter: u32,
    pub risk_registry: RiskRegistrySnapshot,
    pub loop_counters: BTreeMap<String, u32>,
    pub superseded_artifact_refs: Vec<String>,
    pub node_specific_fields: Value,
    pub projection_refs: Vec<String>,
    pub constraint_bundle_refs: Vec<String>,
}

impl RuntimeSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if !is_protocol_node_id(&self.node_id) {
            return Err(format!("invalid node_id {}", self.node_id));
        }

        if self.risk_registry.risk_registry_ref.is_empty() {
            return Err("risk_registry_ref must not be empty".to_string());
        }

        Ok(())
    }

    pub fn minimal_for_test(node_id: &str) -> Self {
        Self {
            snapshot_id: "snap_test".to_string(),
            session_id: "sess_test".to_string(),
            task_id: "task_test".to_string(),
            node_id: node_id.to_string(),
            phase: "intake".to_string(),
            timestamp: "2026-04-26T00:00:00Z".to_string(),
            effective_policy: PolicyMode::Conservative,
            artifact_refs: vec![],
            provider_run_refs: vec![],
            worktree_ref: None,
            rework_counter: 0,
            risk_registry: RiskRegistrySnapshot {
                risk_registry_ref: "riskreg_task_test_v0001".to_string(),
                risk_ids: vec![],
                risks: vec![],
            },
            loop_counters: BTreeMap::new(),
            superseded_artifact_refs: vec![],
            node_specific_fields: serde_json::json!({}),
            projection_refs: vec![],
            constraint_bundle_refs: vec![],
        }
    }
}
