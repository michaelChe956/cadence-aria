use std::path::Path;
use std::process::Command;

use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::models::LifecycleWorkItemRecord;

use super::EvaluationRepoContext;
use super::sanitize::{push_warning_once, sanitize_diff_text};

pub(super) fn repo_context(
    attempt: &CodingExecutionAttempt,
    work_item: Option<&LifecycleWorkItemRecord>,
    diff_base: Option<&str>,
    warnings: &mut Vec<String>,
) -> EvaluationRepoContext {
    let (changed_files, diff_stat, diff_truncated) = match (
        attempt.worktree_path.as_ref(),
        diff_base.filter(|base| !base.trim().is_empty()),
    ) {
        (Some(worktree_path), Some(diff_base)) => diff_context(worktree_path, diff_base, warnings),
        (Some(_), None) => {
            push_warning_once(warnings, "diff_base_missing");
            (Vec::new(), String::new(), false)
        }
        (None, _) => (Vec::new(), String::new(), false),
    };
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
    // Bug45/T4 修复轮:evaluation 上下文对 coder/reviewer 可写的受管
    // worktree 执行 diff/status,属平台受信启动面;子命令之前注入禁桥
    // 参数(与 cross_cutting::git_command::TRUSTED_GIT_BRIDGE_ARGS 同源),
    // 不得询问仓库 core.fsmonitor 探针。
    let output = Command::new("git")
        .args(crate::cross_cutting::git_command::TRUSTED_GIT_BRIDGE_ARGS)
        .args(args)
        .current_dir(worktree_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod bridge_tests {
    use super::*;

    /// Bug45/T4 修复轮:evaluation git_stdout 是平台受信启动面,不得执行
    /// coder 可写仓的 core.fsmonitor 探针。
    #[test]
    fn evaluation_git_stdout_does_not_spawn_repo_fsmonitor_probe() {
        use std::os::unix::fs::PermissionsExt;

        let base = Path::new("/tmp/aria-bug45/t4");
        std::fs::create_dir_all(base).expect("create /tmp/aria-bug45/t4 fixture base");
        let temp = tempfile::tempdir_in(base).expect("fixture dir under /tmp/aria-bug45/t4");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let raw = |argv: &[&str]| {
            let output = Command::new("git")
                .args(argv)
                .current_dir(&repo)
                .output()
                .expect("raw fixture git");
            assert!(output.status.success());
        };
        raw(&["init", "--quiet", "-b", "main"]);
        raw(&["config", "user.email", "aria-t4@example.invalid"]);
        raw(&["config", "user.name", "Aria T4"]);
        std::fs::write(repo.join("README.md"), "x\n").unwrap();
        raw(&["add", "."]);
        raw(&["commit", "--quiet", "-m", "init"]);

        let fsmonitor_probe = repo.join(".git/aria-fsmonitor-fired");
        let fsmonitor_hook = repo.join(".git/aria-fsmonitor-hook.sh");
        std::fs::write(
            &fsmonitor_hook,
            format!("#!/bin/sh\n: > {}\nexit 1\n", fsmonitor_probe.display()),
        )
        .unwrap();
        std::fs::set_permissions(&fsmonitor_hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        raw(&[
            "config",
            "core.fsmonitor",
            fsmonitor_hook.to_str().expect("utf8 hook path"),
        ]);

        assert!(
            git_stdout(&repo, &["diff", "--name-only", "HEAD~0"]).is_some(),
            "bridged diff must succeed"
        );
        assert!(
            !fsmonitor_probe.exists(),
            "Bug45: evaluation git_stdout 不得执行仓库 core.fsmonitor 探针"
        );

        raw(&["status", "--porcelain"]);
        assert!(
            fsmonitor_probe.exists(),
            "对照失败:coder 域 status 应触发其自身 fsmonitor 探针,夹具失效"
        );
    }
}
