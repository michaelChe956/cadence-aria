use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command as StdCommand;
use std::time::Duration;

use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use super::{GitWorkspaceError, GitWorkspaceService};

fn git(repo: &Path, args: &[&str]) {
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn head(repo: &Path) -> Vec<u8> {
    StdCommand::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .expect("read head")
        .stdout
}

async fn wait_for_path(path: &Path) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("git hook path");
}

fn read_pid(path: &Path) -> i32 {
    fs::read_to_string(path)
        .expect("read pid")
        .trim()
        .parse()
        .expect("parse pid")
}

fn process_exists(pid: i32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

async fn process_disappeared(pid: i32) -> bool {
    tokio::time::timeout(Duration::from_millis(500), async {
        while process_exists(pid) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

fn initialize_commit_fixture(repo: &Path) -> Vec<u8> {
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(repo, &["add", "README.md"]);
    git(repo, &["commit", "-m", "base"]);
    let before = head(repo);
    fs::write(repo.join("README.md"), "changed\n").expect("write change");
    git(repo, &["add", "README.md"]);
    before
}

#[tokio::test]
async fn cancelled_git_commit_is_killed_reaped_and_never_commits_late() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    let before = initialize_commit_fixture(repo);
    let entered = repo.join("hook-entered");
    let release = repo.join("hook-release");
    let hook = repo.join(".git/hooks/pre-commit");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\ntouch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.01; done\n",
            entered.display(),
            release.display()
        ),
    )
    .expect("write hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("chmod hook");

    let cancellation = CancellationToken::new();
    let service = GitWorkspaceService::new().with_cancellation(cancellation.clone());
    let commit = tokio::spawn({
        let repo = repo.to_path_buf();
        async move { service.git_commit(&repo, "cancelled commit").await }
    });
    wait_for_path(&entered).await;
    cancellation.cancel();
    let result = tokio::time::timeout(Duration::from_millis(500), commit)
        .await
        .expect("cancelled git commit must be reaped")
        .expect("git commit task");
    assert!(matches!(result, Err(GitWorkspaceError::Cancelled { .. })));

    fs::write(&release, "release\n").expect("release hook if process survived");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(before, head(repo), "cancelled git process committed late");
}

#[tokio::test]
async fn aborted_git_commit_future_kills_hook_process_group_without_late_side_effects() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    let before = initialize_commit_fixture(repo);
    let entered = repo.join("abort-hook-entered");
    let hook_pid_path = repo.join("abort-hook.pid");
    let grandchild_pid_path = repo.join("abort-hook-grandchild.pid");
    let late_marker = repo.join("abort-hook-late");
    let hook = repo.join(".git/hooks/pre-commit");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nsh -c 'printf \"%s\" \"$$\" > \"$1\"; sleep 1; touch \"$2\"' git-hook-child '{}' '{}' &\ntouch '{}'\nwait\n",
            hook_pid_path.display(),
            grandchild_pid_path.display(),
            late_marker.display(),
            entered.display(),
        ),
    )
    .expect("write hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("chmod hook");

    let service = GitWorkspaceService::new();
    let commit = tokio::spawn({
        let repo = repo.to_path_buf();
        async move { service.git_commit(&repo, "aborted future commit").await }
    });
    wait_for_path(&entered).await;
    wait_for_path(&grandchild_pid_path).await;
    commit.abort();
    assert!(commit.await.expect_err("abort commit task").is_cancelled());

    let hook_pid = read_pid(&hook_pid_path);
    let grandchild_pid = read_pid(&grandchild_pid_path);
    let hook_gone = process_disappeared(hook_pid).await;
    let grandchild_gone = process_disappeared(grandchild_pid).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    if !hook_gone || !grandchild_gone {
        unsafe {
            let _ = libc::kill(hook_pid, libc::SIGKILL);
            let _ = libc::kill(grandchild_pid, libc::SIGKILL);
        }
    }

    assert!(hook_gone, "aborted git hook remained alive or zombie");
    assert!(
        grandchild_gone,
        "aborted git hook grandchild remained alive or zombie"
    );
    assert!(
        !late_marker.exists(),
        "aborted git hook wrote a late marker"
    );
    assert_eq!(before, head(repo), "aborted git commit changed HEAD late");
}
