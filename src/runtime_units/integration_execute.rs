use crate::cross_cutting::git_command::{args, git_stdout, run_git, GitCommandError};
use crate::runtime_units::{RuntimeProtocolStep, RuntimeStepStatus};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationExecuteInput {
    pub worktask_id: String,
    pub integration_worktree_path: PathBuf,
    pub candidate_commit_sha: String,
    pub pre_merge_sha: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationExecuteResult {
    pub protocol_step: RuntimeProtocolStep,
    pub integration_report: Value,
    pub integration_commit_sha: Option<String>,
    pub post_merge_sha: Option<String>,
    pub rollback_ref: Option<String>,
    pub next_decision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct IntegrationExecuteError {
    pub code: String,
    pub message: String,
}

pub fn run_integration_execute(
    input: IntegrationExecuteInput,
) -> Result<IntegrationExecuteResult, IntegrationExecuteError> {
    if let Err(error) = run_git(
        &input.integration_worktree_path,
        &args(&["cherry-pick", "--no-commit", &input.candidate_commit_sha]),
    ) {
        let _ = run_git(
            &input.integration_worktree_path,
            &args(&["cherry-pick", "--abort"]),
        );
        let rollback_ref = rollback(&input.integration_worktree_path, &input.pre_merge_sha)?;
        return Ok(failed_result(
            input.worktask_id,
            rollback_ref,
            "cherry_pick_conflict",
            error,
        ));
    }
    run_git(
        &input.integration_worktree_path,
        &args(&[
            "commit",
            "-m",
            &format!("aria: integrate {}", input.worktask_id),
        ]),
    )
    .map_err(map_git_error)?;
    let integration_commit_sha = git_stdout(
        &input.integration_worktree_path,
        &args(&["rev-parse", "HEAD"]),
    )
    .map_err(map_git_error)?;
    let post_merge_sha = integration_commit_sha.clone();
    let report = json!({
        "artifact_kind": "integration_report",
        "integrated_worktasks": [input.worktask_id],
        "status": "completed",
        "node_specific_fields": {
            "integration_commit_sha": integration_commit_sha,
            "post_merge_sha": post_merge_sha,
            "rollback_ref": null,
            "next_decision": "verify",
        },
    });
    Ok(IntegrationExecuteResult {
        protocol_step: RuntimeProtocolStep {
            node_id: "N23".to_string(),
            status: RuntimeStepStatus::Completed,
            node_specific_fields: report["node_specific_fields"].clone(),
        },
        integration_report: report,
        integration_commit_sha: Some(integration_commit_sha),
        post_merge_sha: Some(post_merge_sha),
        rollback_ref: None,
        next_decision: "verify".to_string(),
    })
}

fn failed_result(
    worktask_id: String,
    rollback_ref: String,
    reason: &str,
    error: GitCommandError,
) -> IntegrationExecuteResult {
    let report = json!({
        "artifact_kind": "integration_report",
        "integrated_worktasks": [worktask_id],
        "status": "failed",
        "failure_reason": reason,
        "error": error.to_string(),
        "node_specific_fields": {
            "integration_commit_sha": null,
            "post_merge_sha": null,
            "rollback_ref": rollback_ref,
            "next_decision": "N19",
        },
    });
    IntegrationExecuteResult {
        protocol_step: RuntimeProtocolStep {
            node_id: "N23".to_string(),
            status: RuntimeStepStatus::Blocked,
            node_specific_fields: report["node_specific_fields"].clone(),
        },
        integration_report: report,
        integration_commit_sha: None,
        post_merge_sha: None,
        rollback_ref: Some(rollback_ref),
        next_decision: "N19".to_string(),
    }
}

fn rollback(
    path: &std::path::Path,
    pre_merge_sha: &str,
) -> Result<String, IntegrationExecuteError> {
    run_git(path, &args(&["reset", "--hard", pre_merge_sha])).map_err(map_git_error)?;
    Ok(format!("rollback_to_{pre_merge_sha}"))
}

fn map_git_error(error: GitCommandError) -> IntegrationExecuteError {
    IntegrationExecuteError {
        code: "integration_execute_git_failed".to_string(),
        message: error.to_string(),
    }
}
