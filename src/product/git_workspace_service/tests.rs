use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
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
async fn commit_changed_files_return_real_utf8_for_chinese_paths() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(repo, &["add", "README.md"]);
    let c0 = commit(repo, "base");

    fs::create_dir_all(repo.join("cadence/verify")).expect("create verify dir");
    fs::write(
        repo.join("cadence/verify/2026-10-16_验收报告_v1.0.md"),
        "# 验收报告\n",
    )
    .expect("write chinese report");
    git(repo, &["add", "cadence/verify/2026-10-16_验收报告_v1.0.md"]);
    let c1 = commit(repo, "chinese report");

    let service = GitWorkspaceService::new();
    assert_eq!(
        service
            .git_commit_changed_files(repo, &c1)
            .await
            .expect("read commit changed files"),
        vec!["cadence/verify/2026-10-16_验收报告_v1.0.md"]
    );
    assert_eq!(
        service
            .git_commit_range_changed_files(repo, &c0, &c1)
            .await
            .expect("read range changed files"),
        vec!["cadence/verify/2026-10-16_验收报告_v1.0.md"]
    );
}

#[tokio::test]
async fn changed_file_lists_preserve_quotes_backslashes_spaces_and_newlines() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(repo, &["add", "README.md"]);
    let c0 = commit(repo, "base");

    fs::write(repo.join("plain-ascii-report.md"), "ascii\n").expect("write ascii control");
    fs::create_dir_all(repo.join("notes")).expect("create notes dir");
    fs::write(repo.join("notes/a \"quoted\" name.md"), "quote\n").expect("write quoted");
    fs::write(repo.join("notes/back\\slash.txt"), "slash\n").expect("write backslash");
    fs::write(repo.join("notes/has space.md"), "space\n").expect("write space");
    fs::write(repo.join("notes/line\nbreak.md"), "newline\n").expect("write newline");
    git(repo, &["add", "."]);
    let c1 = commit(repo, "special characters");

    let expected = vec![
        "notes/a \"quoted\" name.md",
        "notes/back\\slash.txt",
        "notes/has space.md",
        "notes/line\nbreak.md",
        "plain-ascii-report.md",
    ];
    let service = GitWorkspaceService::new();
    assert_eq!(
        service
            .git_commit_changed_files(repo, &c1)
            .await
            .expect("read commit changed files"),
        expected
    );
    assert_eq!(
        service
            .git_commit_range_changed_files(repo, &c0, &c1)
            .await
            .expect("read range changed files"),
        expected
    );
}

#[tokio::test]
async fn changed_file_lists_fail_closed_on_invalid_utf8_path_bytes() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(repo, &["add", "README.md"]);
    let c0 = commit(repo, "base");

    fs::create_dir_all(repo.join("cadence/verify")).expect("create verify dir");
    fs::write(
        repo.join("cadence/verify/2026-10-16_验收报告_v1.0.md"),
        "# 验收报告\n",
    )
    .expect("write chinese report");
    let invalid_name = OsStr::from_bytes(b"cadence/verify/bad-\xff-report.md");
    fs::write(repo.join(invalid_name), "bad bytes\n").expect("write invalid utf8 name");
    let add_output = StdCommand::new("git")
        .arg("add")
        .arg("--")
        .arg(invalid_name)
        .current_dir(repo)
        .output()
        .expect("git add invalid utf8 path");
    assert!(
        add_output.status.success(),
        "git add invalid utf8 path: {}",
        String::from_utf8_lossy(&add_output.stderr)
    );
    let c1 = commit(repo, "invalid utf8 path");

    let service = GitWorkspaceService::new();
    for result in [
        service.git_commit_changed_files(repo, &c1).await,
        service.git_commit_range_changed_files(repo, &c0, &c1).await,
    ] {
        match result {
            Err(GitWorkspaceError::Parse(message)) => {
                assert!(
                    message.contains("not valid UTF-8"),
                    "unexpected parse error message: {message}"
                );
            }
            other => panic!("expected Parse error for invalid UTF-8 path, got {other:?}"),
        }
    }
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

/// Bug45/T4 改写说明:原两测试以「平台 commit 执行 coder 仓 pre-commit」为
/// 挂起观察点——该前提已被平台受信禁桥(`core.hooksPath=/dev/null`)废除。
/// kill/reap/进程组合同不变,挂起点迁至夹具自有 bare 远端的 pre-receive:
/// 本地传输下 `git push` 仍以 receive-pack→hook 子进程树运行于
/// ManagedProcessChild 的受管进程组内,观察面等价;且远端侧 hook 不属
/// 本地禁桥范围(见 multi_target_delivery/retry_push 同型用法)。
#[tokio::test]
async fn cancelled_platform_push_is_killed_reaped_and_never_lands_late() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    let remote = tmp.path().join("origin.git");
    let branch = "aria/work-items/work_item_0001/attempt-1";
    fs::create_dir_all(&repo).expect("create repo dir");
    fs::create_dir_all(&remote).expect("create remote dir");
    git(&remote, &["init", "--bare"]);
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "base"]);
    git(&repo, &["branch", branch]);
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("remote path"),
        ],
    );

    let entered = tmp.path().join("push-hook-entered");
    let release = tmp.path().join("push-hook-release");
    let hook = remote.join("hooks/pre-receive");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\ntouch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.01; done\nexit 0\n",
            entered.display(),
            release.display()
        ),
    )
    .expect("write hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("chmod hook");

    let cancellation = CancellationToken::new();
    let service = GitWorkspaceService::new().with_cancellation(cancellation.clone());
    let push = tokio::spawn({
        let repo = repo.to_path_buf();
        async move { service.git_push(&repo, "origin", branch).await }
    });
    wait_for_path(&entered).await;
    cancellation.cancel();
    let result = tokio::time::timeout(Duration::from_millis(500), push)
        .await
        .expect("cancelled git push must be reaped")
        .expect("git push task");
    assert!(matches!(result, Err(GitWorkspaceError::Cancelled { .. })));

    fs::write(&release, "release\n").expect("release hook if process survived");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !remote_has_branch(&remote, branch),
        "cancelled git push landed late"
    );
}

#[tokio::test]
async fn aborted_platform_push_kills_hook_process_group_without_late_side_effects() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    let remote = tmp.path().join("origin.git");
    let branch = "aria/work-items/work_item_0001/attempt-1";
    fs::create_dir_all(&repo).expect("create repo dir");
    fs::create_dir_all(&remote).expect("create remote dir");
    git(&remote, &["init", "--bare"]);
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "base"]);
    git(&repo, &["branch", branch]);
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("remote path"),
        ],
    );

    let entered = tmp.path().join("abort-push-hook-entered");
    let hook_pid_path = tmp.path().join("abort-push-hook.pid");
    let grandchild_pid_path = tmp.path().join("abort-push-hook-grandchild.pid");
    let late_marker = tmp.path().join("abort-push-hook-late");
    let hook = remote.join("hooks/pre-receive");
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
    let push = tokio::spawn({
        let repo = repo.to_path_buf();
        async move { service.git_push(&repo, "origin", branch).await }
    });
    wait_for_path(&entered).await;
    wait_for_path(&grandchild_pid_path).await;
    push.abort();
    assert!(push.await.expect_err("abort push task").is_cancelled());

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
    assert!(
        !remote_has_branch(&remote, branch),
        "aborted git push landed late"
    );
}

fn remote_has_branch(remote: &Path, branch: &str) -> bool {
    let output = StdCommand::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .current_dir(remote)
        .output()
        .expect("show-ref remote branch");
    output.status.success()
}

#[tokio::test]
async fn delete_remote_branch_removes_pushed_branch_and_is_idempotent() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    let remote = tmp.path().join("origin.git");
    fs::create_dir_all(&repo).expect("create repo dir");
    fs::create_dir_all(&remote).expect("create remote dir");
    git(&remote, &["init", "--bare"]);
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "base"]);
    let branch = "aria/work-items/work_item_0001/attempt-1";
    git(&repo, &["branch", branch]);
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("remote path"),
        ],
    );
    git(&repo, &["push", "origin", branch]);
    assert!(
        remote_has_branch(&remote, branch),
        "remote branch must exist before deletion"
    );

    let service = GitWorkspaceService::new();
    service
        .delete_remote_branch(&repo, "origin", branch)
        .await
        .expect("delete remote branch");

    assert!(
        !remote_has_branch(&remote, branch),
        "remote branch must be gone after deletion"
    );

    service
        .delete_remote_branch(&repo, "origin", branch)
        .await
        .expect("repeat deletion of missing remote branch is idempotent");
}

#[tokio::test]
async fn delete_remote_branch_rejects_missing_remote() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(repo, &["add", "README.md"]);
    git(repo, &["commit", "-m", "base"]);

    let service = GitWorkspaceService::new();
    let error = service
        .delete_remote_branch(
            repo,
            "does-not-exist",
            "aria/work-items/work_item_0001/attempt-1",
        )
        .await
        .expect_err("missing remote must fail");
    let GitWorkspaceError::CommandFailed { stderr, .. } = &error else {
        panic!("missing remote must surface as CommandFailed, got {error:?}");
    };
    assert!(
        stderr.contains("does-not-exist")
            || stderr.contains("does not appear to be a git repository"),
        "error must carry remote context, got: {stderr}"
    );
}

#[tokio::test]
async fn delete_remote_branch_rejects_unsafe_branch_name() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(repo, &["add", "README.md"]);
    git(repo, &["commit", "-m", "base"]);
    git(
        repo,
        &["remote", "add", "origin", "https://example.test/origin.git"],
    );

    let service = GitWorkspaceService::new();
    let error = service
        .delete_remote_branch(repo, "origin", "release/v1")
        .await
        .expect_err("unsafe branch name must fail");
    assert!(
        matches!(error, GitWorkspaceError::UnsafePath(_)),
        "unsafe branch name must surface as UnsafePath, got {error:?}"
    );
}

#[tokio::test]
async fn git_add_work_item_changes_fails_closed_on_unreadable_artifact() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(repo, &["add", "README.md"]);
    git(repo, &["commit", "-m", "base"]);

    // F-15 现场形态：coding run 的 agent 在 worktree 里落出无读位的产物
    // （naruto issue_0062 的 tests/backend/fixtures/bad-data/unreadable.json，
    // chmod 000）。git add -A 对其报 unable to index file，且文件留在 worktree
    // 会让同 worktree 后续所有 run 的 staging 永久失败。
    let fixture_dir = repo.join("tests/backend/fixtures/bad-data");
    fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let unreadable = fixture_dir.join("unreadable.json");
    fs::write(&unreadable, "{}\n").expect("write unreadable fixture");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmod 000 fixture");

    let service = GitWorkspaceService::new();
    let error = service
        .git_add_work_item_changes(repo)
        .await
        .expect_err("unreadable artifact must fail closed before git add");
    let GitWorkspaceError::UnreadableArtifact { path, mode } = &error else {
        panic!("expected UnreadableArtifact, got: {error:?}");
    };
    assert_eq!(path, "tests/backend/fixtures/bad-data/unreadable.json");
    assert_eq!(*mode, 0o000);
    let message = error.to_string();
    assert!(
        message.contains("chmod 644"),
        "error must carry the remedy for the agent, got: {message}"
    );

    // agent 自修权限后同一路径放行，staging 恢复正常。
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).expect("chmod 644 fixture");
    service
        .git_add_work_item_changes(repo)
        .await
        .expect("readable artifact stages normally");
}

#[tokio::test]
async fn git_add_work_item_changes_allows_partial_read_bits_and_deletions() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    fs::write(repo.join("owner-only.txt"), "owner read\n").expect("write owner-only");
    git(repo, &["add", "."]);
    git(repo, &["commit", "-m", "base"]);

    // 边界一：仅属主可读（0o400）不拦——git 以当前用户运行可正常索引，
    // 预检只拦「任何用户都读不了」的自我死锁形态。
    fs::write(repo.join("owner-only.txt"), "changed\n").expect("modify owner-only");
    fs::set_permissions(
        repo.join("owner-only.txt"),
        fs::Permissions::from_mode(0o400),
    )
    .expect("chmod 400 owner-only");

    // 边界二：tracked 文件被删除不拦——git add -A 只记录删除，不读内容。
    fs::remove_file(repo.join("README.md")).expect("delete tracked file");

    let service = GitWorkspaceService::new();
    service
        .git_add_work_item_changes(repo)
        .await
        .expect("partial read bits and deletions must stage normally");
}

#[tokio::test]
async fn git_add_work_item_changes_fails_closed_when_owner_read_bit_missing_but_group_readable() {
    // k3 W1K3 P2：0o044（属主读位 0、组/其他可读）——git 以属主身份运行且 Linux
    // 不向 group/other 回退，add 仍 Permission denied；属主读位判定（0o400==0）
    // 必须拦截。修前 0o444 掩码漏过此形态（实测 add 退出 128）。
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::write(repo.join("README.md"), "base\n").expect("write base");
    git(repo, &["add", "README.md"]);
    git(repo, &["commit", "-m", "base"]);
    let owner_blind = repo.join("owner-blind.txt");
    fs::write(&owner_blind, "owner blind\n").expect("write owner-blind");
    fs::set_permissions(&owner_blind, fs::Permissions::from_mode(0o044))
        .expect("chmod 044 fixture");

    let service = GitWorkspaceService::new();
    let error = service
        .git_add_work_item_changes(repo)
        .await
        .expect_err("owner read bit zero must fail closed");
    assert!(
        matches!(error, GitWorkspaceError::UnreadableArtifact { .. }),
        "got: {error:?}"
    );

    // 自修 644 后同路径放行。
    fs::set_permissions(&owner_blind, fs::Permissions::from_mode(0o644))
        .expect("chmod 644 fixture");
    service
        .git_add_work_item_changes(repo)
        .await
        .expect("self-healed 644 stages normally");
}

/// P1-1：commit 前的第二道防线必须覆盖全部平台本地产物路径——旧过滤只认
/// `.aria` 与 `.aria/coding-artifacts/`，`.aria/evidence-token`、
/// `.codegraph/…`、`.verify/…` 全部漏网（exclude 未就位/产物已被跟踪时
/// 会随 `git add -A` 进 commit）；`__pycache__`/`.pyc` 剔除保持不变。
#[tokio::test]
async fn git_add_work_item_changes_unstages_platform_local_artifacts() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::create_dir_all(repo.join("src")).expect("create src dir");
    fs::write(repo.join("src/main.rs"), "fn main() {}\n").expect("write main");
    git(repo, &["add", "."]);
    git(repo, &["commit", "-m", "base"]);

    // exclude 未就位的现场形态：平台产物以 untracked 进入 `git add -A`
    // （令牌文件按真实形态 0600，属主可读不触发 F-15）。
    let token = repo.join(".aria").join("evidence-token");
    fs::create_dir_all(token.parent().expect("aria dir")).expect("create aria dir");
    fs::write(&token, "session-token").expect("write token");
    fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).expect("chmod 600 token");
    fs::create_dir_all(repo.join(".codegraph")).expect("create codegraph dir");
    fs::write(repo.join(".codegraph").join("index.db"), b"x").expect("write codegraph");
    fs::create_dir_all(repo.join(".verify")).expect("create verify dir");
    fs::write(repo.join(".verify").join("report.txt"), b"x").expect("write verify");
    fs::create_dir_all(repo.join("__pycache__")).expect("create pycache dir");
    fs::write(repo.join("__pycache__").join("mod.pyc"), b"x").expect("write pyc");
    // 正常业务改动必须留在暂存区。
    fs::write(repo.join("src/main.rs"), "fn main() { changed }\n").expect("modify main");

    let service = GitWorkspaceService::new();
    service
        .git_add_work_item_changes(repo)
        .await
        .expect("stage work item changes");

    let staged = staged_files(repo);
    assert!(
        staged.iter().any(|path| path == "src/main.rs"),
        "business change must stay staged, got: {staged:?}"
    );
    for leaked in [
        ".aria/evidence-token",
        ".codegraph/index.db",
        ".verify/report.txt",
        "__pycache__/mod.pyc",
    ] {
        assert!(
            !staged.iter().any(|path| path == leaked),
            "`{leaked}` must be unstaged before the work item commit, got: {staged:?}"
        );
    }
}

/// F3：仓内名为 `.verify` / `.aria` 的**普通文件**（不是目录）不是平台产物
/// ——git 的 `.verify/`（尾斜杠）只匹配目录，提交过滤按目录语义判定，
/// 同名普通文件必须留在暂存区（否则交付事实与实际提交不一致）。
#[tokio::test]
async fn git_add_work_item_changes_keeps_same_named_plain_files_staged() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    fs::create_dir_all(repo.join("src")).expect("create src dir");
    fs::write(repo.join("src/main.rs"), "fn main() {}\n").expect("write main");
    git(repo, &["add", "."]);
    git(repo, &["commit", "-m", "base"]);

    // 同名普通文件（不是目录）：`.verify` / `.aria` 是仓内真实文件。
    fs::write(repo.join(".verify"), b"plain file\n").expect("write .verify file");
    fs::write(repo.join(".aria"), b"plain file\n").expect("write .aria file");

    let service = GitWorkspaceService::new();
    service
        .git_add_work_item_changes(repo)
        .await
        .expect("stage work item changes");

    let staged = staged_files(repo);
    for kept in [".verify", ".aria"] {
        assert!(
            staged.iter().any(|path| path == kept),
            "same-named plain file `{kept}` must stay staged, got: {staged:?}"
        );
    }
}

fn staged_files(repo: &Path) -> Vec<String> {
    let output = StdCommand::new("git")
        .args(["diff", "--cached", "--name-only", "-z"])
        .current_dir(repo)
        .output()
        .expect("read staged files");
    assert!(
        output.status.success(),
        "git diff --cached: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn git_remote_configured_distinguishes_missing_from_unreachable_remote() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test User"]);
    let service = GitWorkspaceService::new();

    assert!(
        !service
            .git_remote_configured(repo, "origin")
            .await
            .expect("no remote configured"),
        "repo without remotes must report origin unconfigured"
    );

    // 已配置但指向不存在的路径（不可达）仍判定为「已配置」，不得被当作本地交付。
    git(
        repo,
        &[
            "remote",
            "add",
            "origin",
            &tmp.path().join("missing.git").to_string_lossy(),
        ],
    );
    assert!(
        service
            .git_remote_configured(repo, "origin")
            .await
            .expect("configured remote"),
        "configured-but-unreachable origin must report configured"
    );
    assert!(
        !service
            .git_remote_configured(repo, "upstream")
            .await
            .expect("other remote name"),
        "only the exact remote name counts"
    );
}

/// Bug1(owner 2026-10-16 现场):plan 级删除后「注册已删、目录残留」。
/// `git worktree remove --force` 对未注册目录报 `is not a working tree`,
/// 删除链由此卡死且残留目录永不消失;remove 必须把残留目录补删干净并幂等。
#[tokio::test]
async fn remove_worktree_clears_unregistered_leftover_directory_idempotently() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    initialize_commit_fixture(repo);
    let worktree_path = repo
        .join(".worktrees")
        .join("aria-issues")
        .join("issue_0001")
        .join("uuid-leftover");
    fs::create_dir_all(&worktree_path).expect("create leftover dir");
    fs::write(worktree_path.join("leftover.txt"), "residue").expect("write residue");

    let service = GitWorkspaceService::new();
    service
        .remove_worktree(repo, &worktree_path)
        .await
        .expect("unregistered leftover directory must be removed, not fatal");
    assert!(
        !worktree_path.exists(),
        "leftover directory must be gone after remove"
    );

    service
        .remove_worktree(repo, &worktree_path)
        .await
        .expect("second remove on missing path is idempotent");
    assert!(!worktree_path.exists());
}

/// Bug1 新建侧:未注册残留目录会让 `git worktree add` 以 `already exists`
/// 失败(现场表现为「已存在部分内容」)。create 必须与删除残留兼容:
/// 清掉残留后重建为合法 worktree,后续 remove 正常回收。
#[tokio::test]
async fn create_worktree_replaces_unregistered_leftover_directory() {
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path();
    initialize_commit_fixture(repo);
    let default_branch = String::from_utf8(
        StdCommand::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .current_dir(repo)
            .output()
            .expect("read default branch")
            .stdout,
    )
    .expect("utf8 branch");
    let branch = "aria/issues/issue_0001/uuid-reuse";
    let worktree_path = repo
        .join(".worktrees")
        .join("aria-issues")
        .join("issue_0001")
        .join("uuid-reuse");
    fs::create_dir_all(&worktree_path).expect("create leftover dir");
    fs::write(worktree_path.join("leftover.txt"), "residue").expect("write residue");

    let service = GitWorkspaceService::new();
    service
        .create_branch(repo, branch, default_branch.trim())
        .await
        .expect("create branch");
    service
        .create_worktree(repo, branch, &worktree_path)
        .await
        .expect("creation must tolerate deletion residue instead of failing already-exists");
    assert!(
        worktree_path.join(".git").exists(),
        "recreated path must be a real worktree"
    );
    assert!(
        !worktree_path.join("leftover.txt").exists(),
        "residue content must not leak into the new worktree"
    );

    service
        .remove_worktree(repo, &worktree_path)
        .await
        .expect("remove registered worktree after recreation");
    assert!(!worktree_path.exists());
}

// ---- Bug45/T4:平台受信 Git 启动禁 hooks/fsmonitor 桥 ----

/// Bug45/T4 fixture 根:遵守批2 GC5(测试临时物在 /tmp/aria-bug45/<task>
/// 私有子目录,不散落 owner HOME 或真实仓库)。
fn bridge_fixture_dir() -> tempfile::TempDir {
    let base = Path::new("/tmp/aria-bug45/t4");
    fs::create_dir_all(base).expect("create /tmp/aria-bug45/t4 fixture base");
    tempfile::tempdir_in(base).expect("fixture dir under /tmp/aria-bug45/t4")
}

fn write_executable_script(path: &Path, body: &str) {
    fs::write(path, body).expect("write executable fixture");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod executable fixture");
}

/// coder 域 git config 读取(exit 1 = 未设置)。
fn git_config_value(repo: &Path, key: &str) -> Option<String> {
    let output = StdCommand::new("git")
        .args(["config", "--get", key])
        .current_dir(repo)
        .output()
        .expect("read git config");
    if output.status.success() {
        Some(
            String::from_utf8(output.stdout)
                .expect("utf8 config value")
                .trim()
                .to_string(),
        )
    } else {
        None
    }
}

/// Bug45 夹具:模拟 coder 植入的「仓库执行桥」——pre-commit /
/// post-checkout hook 与 `core.fsmonitor` 探针脚本,只落标记文件、恒
/// exit 0/1,不影响 git 自身成败;平台受信 git 不得触发任何一个。
fn repo_with_execution_bridges() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
    let temp = bridge_fixture_dir();
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet", "-b", "main"]);
    git(&repo, &["config", "user.email", "aria-t4@example.invalid"]);
    git(&repo, &["config", "user.name", "Aria T4"]);
    fs::write(repo.join("README.md"), "# bridge fixture\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "--quiet", "-m", "initial"]);

    let pre_commit_probe = repo.join(".git/aria-pre-commit-fired");
    write_executable_script(
        &repo.join(".git/hooks/pre-commit"),
        &format!("#!/bin/sh\n: > {}\nexit 0\n", pre_commit_probe.display()),
    );
    let post_checkout_probe = repo.join(".git/aria-post-checkout-fired");
    write_executable_script(
        &repo.join(".git/hooks/post-checkout"),
        &format!("#!/bin/sh\n: > {}\nexit 0\n", post_checkout_probe.display()),
    );
    let fsmonitor_probe = repo.join(".git/aria-fsmonitor-fired");
    let fsmonitor_hook = repo.join(".git/aria-fsmonitor-hook.sh");
    write_executable_script(
        &fsmonitor_hook,
        &format!("#!/bin/sh\n: > {}\nexit 1\n", fsmonitor_probe.display()),
    );
    git(
        &repo,
        &[
            "config",
            "core.fsmonitor",
            fsmonitor_hook.to_str().expect("utf8 fsmonitor hook path"),
        ],
    );
    (
        temp,
        repo,
        pre_commit_probe,
        post_checkout_probe,
        fsmonitor_probe,
    )
}

#[tokio::test]
async fn platform_status_does_not_spawn_repo_fsmonitor_probe() {
    let (_temp, repo, _pre_commit_probe, _post_checkout_probe, fsmonitor_probe) =
        repo_with_execution_bridges();

    GitWorkspaceService::new()
        .git_status(&repo)
        .await
        .expect("platform git_status must succeed");
    assert!(
        !fsmonitor_probe.exists(),
        "Bug45: 平台 git_status 不得执行仓库 core.fsmonitor 探针"
    );
}

#[tokio::test]
async fn platform_commit_does_not_execute_repo_pre_commit_hook() {
    let (_temp, repo, pre_commit_probe, _post_checkout_probe, fsmonitor_probe) =
        repo_with_execution_bridges();
    let service = GitWorkspaceService::new();
    fs::write(repo.join("platform-change.txt"), "staged by platform\n").unwrap();
    service.git_add_all(&repo).await.expect("platform add");

    let commit = service
        .git_commit(&repo, "aria: platform bridge")
        .await
        .expect("platform git_commit must succeed");
    assert!(!commit.commit_sha.is_empty());
    assert!(
        !pre_commit_probe.exists(),
        "Bug45: 平台 git_commit 不得执行仓库 pre-commit hook"
    );
    assert!(
        !fsmonitor_probe.exists(),
        "Bug45: 平台 git_commit 亦不得执行仓库 core.fsmonitor 探针"
    );
    // 平台禁桥是命令级覆盖,不改仓库 config:fsmonitor 配置原样保留、
    // core.hooksPath 未被写入。
    assert_eq!(
        git_config_value(&repo, "core.fsmonitor")
            .expect("coder 的 core.fsmonitor 配置必须原样保留"),
        repo.join(".git/aria-fsmonitor-hook.sh")
            .to_str()
            .expect("utf8 hook path"),
    );
    assert!(
        git_config_value(&repo, "core.hooksPath").is_none(),
        "平台禁桥不得把 core.hooksPath 写入仓库 config"
    );
}

#[tokio::test]
async fn platform_worktree_add_does_not_execute_repo_post_checkout_hook() {
    let (_temp, repo, _pre_commit_probe, post_checkout_probe, _fsmonitor_probe) =
        repo_with_execution_bridges();
    let service = GitWorkspaceService::new();
    let branch = "aria/issues/t4-bridge";
    service
        .create_branch(&repo, branch, "main")
        .await
        .expect("create branch");
    let worktree_path = repo.join(".worktrees/aria-issues/t4-bridge");
    service
        .create_worktree(&repo, branch, &worktree_path)
        .await
        .expect("platform create_worktree must succeed");
    assert!(
        worktree_path.join(".git").exists(),
        "worktree must actually be created"
    );
    assert!(
        !post_checkout_probe.exists(),
        "Bug45: 平台 worktree add 不得执行仓库 post-checkout hook"
    );
}

/// 信任域对照:coder 自己的 git(无禁桥)仍执行其仓库 hooks 与
/// fsmonitor 探针——平台禁桥不得外溢成「禁 Git 能力」或改仓库语义。
#[test]
fn coder_domain_git_still_runs_repo_hooks_and_fsmonitor() {
    let (_temp, repo, pre_commit_probe, _post_checkout_probe, fsmonitor_probe) =
        repo_with_execution_bridges();

    git(&repo, &["status", "--porcelain"]);
    assert!(
        fsmonitor_probe.exists(),
        "coder 域 status 应触发其自身 fsmonitor 探针"
    );

    fs::write(repo.join("coder-change.txt"), "coder keeps hooks\n").unwrap();
    git(&repo, &["add", "coder-change.txt"]);
    git(&repo, &["commit", "--quiet", "-m", "coder keeps hooks"]);
    assert!(
        pre_commit_probe.exists(),
        "coder 域 commit 应执行其自身 pre-commit hook(开发提交保护不被绕过)"
    );
}
