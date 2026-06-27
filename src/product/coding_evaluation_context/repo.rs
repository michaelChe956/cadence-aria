use std::path::Path;
use std::process::Command;

use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::models::LifecycleWorkItemRecord;

use super::EvaluationRepoContext;
use super::sanitize::{push_warning_once, sanitize_diff_text};

pub(super) fn repo_context(
    attempt: &CodingExecutionAttempt,
    work_item: Option<&LifecycleWorkItemRecord>,
    warnings: &mut Vec<String>,
) -> EvaluationRepoContext {
    let (changed_files, diff_stat, diff_truncated) = attempt
        .worktree_path
        .as_ref()
        .map_or((Vec::new(), String::new(), false), |worktree_path| {
            diff_context(worktree_path, &attempt.base_branch, warnings)
        });
    EvaluationRepoContext {
        repository_id: work_item.map(|work_item| work_item.repository_id.clone()),
        branch_name: attempt.branch_name.clone(),
        base_branch: attempt.base_branch.clone(),
        worktree_path: attempt
            .worktree_path
            .as_ref()
            .map(|path| path.display().to_string()),
        changed_files,
        diff_stat,
        diff_truncated,
    }
}

fn diff_context(
    worktree_path: &Path,
    base_branch: &str,
    warnings: &mut Vec<String>,
) -> (Vec<String>, String, bool) {
    let Some(name_only) = git_stdout(worktree_path, &["diff", "--name-only", base_branch]) else {
        push_warning_once(warnings, "diff_unavailable");
        return (Vec::new(), String::new(), false);
    };
    let stat = git_stdout(worktree_path, &["diff", "--stat", base_branch]).unwrap_or_default();
    let untracked = git_stdout(
        worktree_path,
        &["ls-files", "--others", "--exclude-standard"],
    )
    .unwrap_or_default();

    let mut changed_files = name_only
        .lines()
        .chain(untracked.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    changed_files.sort();
    changed_files.dedup();

    let combined_stat = if untracked.trim().is_empty() {
        stat
    } else {
        format!("{stat}\nUntracked files:\n{untracked}")
    };
    let (diff_stat, diff_truncated) = sanitize_diff_text(&combined_stat);
    (changed_files, diff_stat, diff_truncated)
}

fn git_stdout(worktree_path: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}
