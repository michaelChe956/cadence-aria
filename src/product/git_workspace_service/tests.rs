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

fn git_with_commit_date(repo: &Path, args: &[&str], commit_date: &str) {
    let output = StdCommand::new("git")
        .args(args)
        .env("GIT_AUTHOR_DATE", commit_date)
        .env("GIT_COMMITTER_DATE", commit_date)
        .current_dir(repo)
        .output()
        .expect("run dated git fixture command");
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

fn commit(repo: &Path, message: &str) -> String {
    git(repo, &["commit", "-m", message]);
    String::from_utf8(head(repo))
        .expect("HEAD is utf-8")
        .trim()
        .to_string()
}

fn commit_at(repo: &Path, message: &str, commit_date: &str) -> String {
    git_with_commit_date(repo, &["commit", "-m", message], commit_date);
    String::from_utf8(head(repo))
        .expect("HEAD is utf-8")
        .trim()
        .to_string()
}

#[tokio::test]
async fn commit_range_includes_initial_and_rework_commits() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(repo, &["add", "README.md"]);
    let c0 = commit(repo, "base");

    fs::write(repo.join("initial.txt"), "initial coder change\n").expect("write initial");
    git(repo, &["add", "initial.txt"]);
    let c1 = commit(repo, "initial coder change");

    fs::write(repo.join("initial.txt"), "reworked coder change\n").expect("rework initial");
    fs::write(repo.join("rework.txt"), "rework commit\n").expect("write rework");
    git(repo, &["add", "initial.txt", "rework.txt"]);
    let c2 = commit(repo, "coder rework");

    let service = GitWorkspaceService::new();
    assert_eq!(
        service
            .git_commit_range_commits(repo, &c0, &c2)
            .await
            .expect("read commit range"),
        vec![c1, c2.clone()]
    );
    assert_eq!(
        service
            .git_commit_range_changed_files(repo, &c0, &c2)
            .await
            .expect("read changed files"),
        vec!["initial.txt", "rework.txt"]
    );
}

#[tokio::test]
async fn equal_commit_range_is_an_empty_observation() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(repo, &["add", "README.md"]);
    let c0 = commit(repo, "base");

    let service = GitWorkspaceService::new();
    assert!(
        service
            .git_commit_range_commits(repo, &c0, &c0)
            .await
            .expect("read empty commit range")
            .is_empty()
    );
    assert!(
        service
            .git_commit_range_changed_files(repo, &c0, &c0)
            .await
            .expect("read empty changed-file range")
            .is_empty()
    );
}

#[tokio::test]
async fn commit_range_commits_uses_topological_order_for_merge_history() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(repo, &["add", "README.md"]);
    let c0 = commit_at(repo, "base", "1000000000 +0000");

    fs::write(repo.join("main-1.txt"), "main one\n").expect("write main one");
    git(repo, &["add", "main-1.txt"]);
    let main1 = commit_at(repo, "main one", "1000000100 +0000");
    git(repo, &["branch", "feature", &c0]);

    fs::write(repo.join("main-2.txt"), "main two\n").expect("write main two");
    git(repo, &["add", "main-2.txt"]);
    let main2 = commit_at(repo, "main two", "1000000300 +0000");

    git(repo, &["switch", "feature"]);
    fs::write(repo.join("feature-1.txt"), "feature one\n").expect("write feature one");
    git(repo, &["add", "feature-1.txt"]);
    let feature1 = commit_at(repo, "feature one", "1000000200 +0000");
    fs::write(repo.join("feature-2.txt"), "feature two\n").expect("write feature two");
    git(repo, &["add", "feature-2.txt"]);
    let feature2 = commit_at(repo, "feature two", "1000000400 +0000");

    git(repo, &["switch", "-"]);
    git_with_commit_date(
        repo,
        &["merge", "--no-ff", "feature", "-m", "merge feature"],
        "1000000500 +0000",
    );
    let merge = String::from_utf8(head(repo))
        .expect("HEAD is utf-8")
        .trim()
        .to_string();

    let service = GitWorkspaceService::new();
    assert_eq!(
        service
            .git_commit_range_commits(repo, &c0, &merge)
            .await
            .expect("read merge commit range"),
        vec![main1, main2, feature1, feature2, merge]
    );
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
