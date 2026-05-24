use std::fs;
use std::path::Path;
use std::process::Command;

use cadence_aria::product::coding_models::{PushStatus, RemoteKind};
use cadence_aria::product::git_workspace_service::GitWorkspaceService;
use tempfile::tempdir;

#[tokio::test]
async fn git_workspace_service_drives_branch_worktree_commit_diff_and_push() {
    let root = tempdir().expect("tempdir");
    let repo = root.path().join("repo");
    let remote = root.path().join("remote.git");
    init_repo(&repo);
    run_git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
    run_git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );

    let service = GitWorkspaceService::new();
    service
        .create_branch(&repo, "aria/work-items/work_item_0001/attempt-1", "HEAD")
        .await
        .expect("create branch");
    let worktree = repo
        .join(".worktrees")
        .join("aria-work-items")
        .join("work_item_0001")
        .join("attempt-1");
    service
        .create_worktree(&repo, "aria/work-items/work_item_0001/attempt-1", &worktree)
        .await
        .expect("create worktree");
    fs::write(worktree.join("src.txt"), "hello\nworld\n").expect("modify file");

    let status = service.git_status(&worktree).await.expect("git status");
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].path, "src.txt");

    service.git_add_all(&worktree).await.expect("git add");
    let commit = service
        .git_commit(&worktree, "feat: update src")
        .await
        .expect("git commit");
    assert_eq!(commit.commit_sha.len(), 40);

    let diff = service
        .git_diff_stat(&worktree, "master")
        .await
        .expect("diff stat");
    assert_eq!(diff.files[0].path, "src.txt");
    assert!(diff.insertions >= 1);

    let push = service
        .git_push(
            &worktree,
            "origin",
            "aria/work-items/work_item_0001/attempt-1",
        )
        .await
        .expect("git push");
    assert_eq!(push.status, PushStatus::Pushed);

    let remote_kind = service
        .detect_remote_kind(&repo)
        .await
        .expect("remote kind");
    assert_eq!(remote_kind, RemoteKind::GenericGit);
}

fn init_repo(repo: &Path) {
    fs::create_dir_all(repo).expect("create repo");
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "aria@example.com"]);
    run_git(repo, &["config", "user.name", "Aria Test"]);
    fs::write(repo.join("src.txt"), "hello\n").expect("seed file");
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:{}\nstderr:{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
