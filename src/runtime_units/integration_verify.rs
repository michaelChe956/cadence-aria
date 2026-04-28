use crate::cross_cutting::git_command::{args, run_git};
use crate::runtime_units::{RuntimeProtocolStep, RuntimeStepStatus};
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationVerifyInput {
    pub worktask_id: String,
    pub integration_worktree_path: PathBuf,
    pub pre_merge_sha: String,
    pub verify_passed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationVerifyResult {
    pub protocol_step: RuntimeProtocolStep,
    pub verify_decision: String,
    pub next_decision: String,
    pub rollback_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct IntegrationVerifyError {
    pub code: String,
    pub message: String,
}

pub fn run_integration_verify(
    input: IntegrationVerifyInput,
) -> Result<IntegrationVerifyResult, IntegrationVerifyError> {
    if input.verify_passed {
        return Ok(IntegrationVerifyResult {
            protocol_step: RuntimeProtocolStep {
                node_id: "N24".to_string(),
                status: RuntimeStepStatus::Completed,
                node_specific_fields: json!({
                    "verify_decision": "pass",
                    "rollback_reason": null,
                }),
            },
            verify_decision: "pass".to_string(),
            next_decision: "N25".to_string(),
            rollback_ref: None,
        });
    }
    run_git(
        &input.integration_worktree_path,
        &args(&["reset", "--hard", &input.pre_merge_sha]),
    )
    .map_err(|error| IntegrationVerifyError {
        code: "integration_verify_rollback_failed".to_string(),
        message: error.to_string(),
    })?;
    let rollback_ref = format!("rollback_to_{}", input.pre_merge_sha);
    Ok(IntegrationVerifyResult {
        protocol_step: RuntimeProtocolStep {
            node_id: "N24".to_string(),
            status: RuntimeStepStatus::Blocked,
            node_specific_fields: json!({
                "verify_decision": "rollback",
                "rollback_reason": "integration_verify_failed",
                "rollback_ref": rollback_ref,
                "worktask_id": input.worktask_id,
            }),
        },
        verify_decision: "rollback".to_string(),
        next_decision: "N19".to_string(),
        rollback_ref: Some(rollback_ref),
    })
}
