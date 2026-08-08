use std::collections::BTreeSet;

use super::*;
use crate::cross_cutting::git_command::{args, git_stdout, run_git};
use crate::product::coding_workspace_engine::group_review_types::{CompletionDiff, GroupGitFacts};
use crate::product::coding_workspace_engine::plan_defect_routing::AuthoritativeGroupReviewerBinding;

impl CodingWorkspaceEngine {
    // Legacy group review executor retained for regression coverage
    // (`group_review_runner`, `group_review_compatibility`, and `group_review_e2e`)
    // and legacy artifact reader support. No production callers remain after the
    // Task 6 recovery reroute. Remove only when legacy shard/reduction artifacts
    // no longer need regression coverage.
    #[allow(dead_code)]
    pub(crate) async fn collect_group_git_facts(
        &self,
        attempt: &CodingExecutionAttempt,
        bindings: &[AuthoritativeGroupReviewerBinding],
        review_request: &ReviewRequest,
        worktree_path: &Path,
    ) -> Result<GroupGitFacts, CodingWorkspaceEngineError> {
        let final_commit = review_request.commit_sha.clone();
        let final_diff = git_stdout(
            worktree_path,
            &args(&["diff", &attempt.base_branch, &final_commit]),
        )
        .map_err(|error| {
            CodingWorkspaceEngineError::GroupReviewGitFact(format!("final_diff_failed: {error}"))
        })?;
        let diff_stat = git_stdout(
            worktree_path,
            &args(&["diff", "--numstat", &attempt.base_branch, &final_commit]),
        )
        .map_err(|error| {
            CodingWorkspaceEngineError::GroupReviewGitFact(format!(
                "final_diff_stat_failed: {error}"
            ))
        })?;

        let mut sorted_bindings = bindings.iter().collect::<Vec<_>>();
        sorted_bindings.sort_by(|left, right| left.run.id.cmp(&right.run.id));
        let mut completion_diffs = Vec::with_capacity(sorted_bindings.len());
        let mut completion_commit_in_final = BTreeSet::new();

        for binding in sorted_bindings {
            let completion_commit = binding.run.completion_commit.clone().ok_or_else(|| {
                CodingWorkspaceEngineError::CompletionCommitMissing(binding.run.id.clone())
            })?;
            let base_commit = binding
                .run
                .start_commit
                .clone()
                .unwrap_or_else(|| attempt.base_branch.clone());
            let patch = git_stdout(
                worktree_path,
                &args(&["diff", &base_commit, &completion_commit]),
            )
            .map_err(|error| {
                CodingWorkspaceEngineError::GroupReviewGitFact(format!(
                    "completion_diff_failed: {}: {error}",
                    binding.run.id
                ))
            })?;
            let reachability_args = args(&[
                "merge-base",
                "--is-ancestor",
                &completion_commit,
                &final_commit,
            ]);
            match run_git(worktree_path, &reachability_args) {
                Ok(_) => {
                    completion_commit_in_final.insert(completion_commit.clone());
                }
                Err(crate::cross_cutting::git_command::GitCommandError::Failed { record })
                    if record.exit_code == Some(1) => {}
                Err(error) => {
                    return Err(CodingWorkspaceEngineError::GroupReviewGitFact(format!(
                        "reachability_failed: {}: {error}",
                        binding.run.id
                    )));
                }
            }
            completion_diffs.push(CompletionDiff {
                unit_run_id: binding.run.id.clone(),
                base_commit,
                completion_commit,
                patch,
                file_stats: Vec::new(),
                hunks: Vec::new(),
            });
        }

        Ok(GroupGitFacts {
            diff_stat,
            completion_diffs,
            final_diff,
            final_commit,
            completion_commit_in_final,
        })
    }
}
