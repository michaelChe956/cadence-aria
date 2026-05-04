use crate::cross_cutting::git_command::{GitCommandError, args, git_stdout, run_git};
use crate::cross_cutting::integration_queue::IntegrationQueue;
use crate::cross_cutting::worktree::validate_write_path;
use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeProtocolStep, RuntimeStepStatus, RuntimeUnit,
    RuntimeUnitError, RuntimeUnitResult,
};
use serde_json::json;
use std::future::Future;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationPrepareInput {
    pub session_id: String,
    pub task_id: String,
    pub worktask_id: String,
    pub worktree_path: PathBuf,
    pub integration_worktree_path: PathBuf,
    pub integration_branch: String,
    pub base_ref: String,
    pub allowed_write_scope: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntegrationPrepareResult {
    pub protocol_steps: Vec<RuntimeProtocolStep>,
    pub candidate_commit_sha: String,
    pub integration_record_id: String,
    pub queue_position: usize,
    pub integration_branch: String,
    pub pre_merge_sha: String,
    pub integration_worktree_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct IntegrationPrepareError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegrationPrepareUnit;

impl RuntimeUnit for IntegrationPrepareUnit {
    fn unit_id(&self) -> &'static str {
        "integration_prepare"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N20", "N21", "N22"]
    }

    fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> impl Future<Output = Result<RuntimeUnitResult, RuntimeUnitError>> + Send {
        async {
            Err(RuntimeUnitError {
                code: "integration_prepare_requires_git_context".to_string(),
                message: "M20 requires explicit IntegrationPrepareInput".to_string(),
            })
        }
    }
}

pub fn run_integration_prepare(
    input: IntegrationPrepareInput,
    queue: &mut IntegrationQueue,
) -> Result<IntegrationPrepareResult, IntegrationPrepareError> {
    let changed_files = changed_files(&input.worktree_path)?;
    if changed_files.is_empty() {
        return Err(IntegrationPrepareError {
            code: "candidate_commit_empty_diff".to_string(),
            message: "no worktree changes available for candidate commit".to_string(),
        });
    }
    for changed_file in &changed_files {
        validate_write_path(
            &input.worktree_path,
            &input.allowed_write_scope,
            &input.worktree_path.join(changed_file),
            true,
        )
        .map_err(|error| IntegrationPrepareError {
            code: "candidate_commit_scope_violation".to_string(),
            message: error.to_string(),
        })?;
    }

    let mut add_args = vec!["add".to_string(), "--".to_string()];
    add_args.extend(changed_files.clone());
    run_git(&input.worktree_path, &add_args).map_err(map_candidate_git_error)?;
    run_git(
        &input.worktree_path,
        &args(&[
            "commit",
            "-m",
            &format!("aria: {} candidate", input.worktask_id),
        ]),
    )
    .map_err(map_candidate_git_error)?;
    let candidate_commit_sha = git_stdout(&input.worktree_path, &args(&["rev-parse", "HEAD"]))
        .map_err(map_candidate_git_error)?;

    let integration_record = queue.enqueue(&input.worktask_id, &candidate_commit_sha);
    if !input.integration_worktree_path.exists() {
        if let Some(parent) = input.integration_worktree_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| IntegrationPrepareError {
                code: "integration_worktree_create_failed".to_string(),
                message: error.to_string(),
            })?;
        }
        let path = input
            .integration_worktree_path
            .to_string_lossy()
            .to_string();
        run_git(
            &input.worktree_path,
            &args(&[
                "worktree",
                "add",
                "-B",
                &input.integration_branch,
                &path,
                &input.base_ref,
            ]),
        )
        .map_err(map_prepare_git_error)?;
    }
    let pre_merge_sha = git_stdout(
        &input.integration_worktree_path,
        &args(&["rev-parse", "HEAD"]),
    )
    .map_err(map_prepare_git_error)?;

    Ok(IntegrationPrepareResult {
        protocol_steps: vec![
            RuntimeProtocolStep {
                node_id: "N20".to_string(),
                status: RuntimeStepStatus::Completed,
                node_specific_fields: json!({
                    "candidate_commit_sha": candidate_commit_sha,
                    "ready_decision": "ready",
                    "block_reason": null,
                }),
            },
            RuntimeProtocolStep {
                node_id: "N21".to_string(),
                status: RuntimeStepStatus::Completed,
                node_specific_fields: json!({
                    "queue_position": integration_record.queue_position,
                    "integration_record_id": integration_record.integration_record_id,
                }),
            },
            RuntimeProtocolStep {
                node_id: "N22".to_string(),
                status: RuntimeStepStatus::Completed,
                node_specific_fields: json!({
                    "integration_branch": input.integration_branch,
                    "pre_merge_sha": pre_merge_sha,
                    "candidate_commit_sha": candidate_commit_sha,
                }),
            },
        ],
        candidate_commit_sha,
        integration_record_id: integration_record.integration_record_id,
        queue_position: integration_record.queue_position,
        integration_branch: input.integration_branch,
        pre_merge_sha,
        integration_worktree_path: input.integration_worktree_path,
    })
}

fn changed_files(worktree_path: &std::path::Path) -> Result<Vec<String>, IntegrationPrepareError> {
    let status = git_stdout(
        worktree_path,
        &args(&["status", "--porcelain", "--untracked-files=all"]),
    )
    .map_err(map_prepare_git_error)?;
    Ok(status
        .lines()
        .filter_map(|line| line.get(3..))
        .map(str::trim)
        .filter(|path| !path.is_empty() && !is_aria_runtime_path(path))
        .map(ToOwned::to_owned)
        .collect())
}

fn is_aria_runtime_path(path: &str) -> bool {
    path == ".aria" || path.starts_with(".aria/")
}

fn map_candidate_git_error(error: GitCommandError) -> IntegrationPrepareError {
    let message = error.to_string();
    let code = if message.contains("nothing to commit") {
        "candidate_commit_empty_diff"
    } else if message.contains("Author identity unknown") {
        "candidate_commit_author_missing"
    } else if message.contains("hook") {
        "candidate_commit_hook_failed"
    } else if message.contains("sign") {
        "candidate_commit_signing_failed"
    } else {
        "candidate_commit_failed"
    };
    IntegrationPrepareError {
        code: code.to_string(),
        message,
    }
}

fn map_prepare_git_error(error: GitCommandError) -> IntegrationPrepareError {
    IntegrationPrepareError {
        code: "integration_prepare_git_failed".to_string(),
        message: error.to_string(),
    }
}
