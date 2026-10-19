use std::path::{Path, PathBuf};

use super::setup;
use crate::product::coding_attempt_store::{
    CodingGitOperationKind, CodingGitOperationPhase, CompleteReviewGitOperationInput,
    PrepareCodingGitOperationInput,
};
use crate::product::coding_models::{PushStatus, RemoteKind};
use crate::product::json_store::ProductStoreError;

fn run_test_git(cwd: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("git {} failed to start: {error}", args.join(" ")));
    assert!(
        output.status.success(),
        "git {} failed\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// ReviewRequest journal 进入 reviewer 写通道前要求 worktree 可 canonicalize
/// 且 `.git` gitfile / commondir 可解析；测试必须使用真实 git repo + worktree，
/// 且路径来自本用例 TempDir（不得硬编码共享 /tmp 路径，避免并行用例互踩）。
fn init_review_repo_worktree(root: &Path, branch_name: &str) -> (PathBuf, PathBuf) {
    let repo_path = root.join("repo");
    std::fs::create_dir_all(&repo_path).expect("create repo dir");
    run_test_git(&repo_path, &["init"]);
    run_test_git(&repo_path, &["config", "user.email", "aria@example.com"]);
    run_test_git(&repo_path, &["config", "user.name", "Aria Test"]);
    std::fs::write(repo_path.join("README.md"), "initial\n").expect("seed file");
    run_test_git(&repo_path, &["add", "."]);
    run_test_git(&repo_path, &["commit", "-m", "initial"]);
    let worktree_path = repo_path.join(".worktrees/aria-work-items/work_item_0001/attempt-1");
    std::fs::create_dir_all(worktree_path.parent().expect("worktree parent"))
        .expect("create worktree parent");
    run_test_git(
        &repo_path,
        &[
            "worktree",
            "add",
            "-b",
            branch_name,
            worktree_path.to_str().expect("utf-8 worktree path"),
        ],
    );
    (repo_path, worktree_path)
}

#[test]
fn coding_git_operation_journal_is_identity_strict_and_phase_monotonic() {
    let (tmp, store, attempt) = setup();
    let repo_path = tmp.path().join("repo");
    let worktree_path = repo_path.join(".worktrees/aria-work-items/work_item_0001/attempt-1");
    let input = PrepareCodingGitOperationInput {
        kind: CodingGitOperationKind::WorktreePrepare,
        repo_path: repo_path.clone(),
        worktree_path: worktree_path.clone(),
        branch_name: attempt.branch_name.clone(),
        base_branch: attempt.base_branch.clone(),
        before_head: "before-head".to_string(),
        remote: None,
        commit_message: None,
    };
    let journal = store
        .prepare_coding_git_operation(&attempt, input.clone())
        .expect("prepare journal");
    assert_eq!(journal.phase, CodingGitOperationPhase::Before);

    let jump = store.advance_coding_git_operation(
        &attempt,
        &journal,
        CodingGitOperationPhase::WorktreeCreated,
        None,
    );
    assert!(matches!(
        jump,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));

    let branch = store
        .advance_coding_git_operation(
            &attempt,
            &journal,
            CodingGitOperationPhase::BranchCreated,
            None,
        )
        .expect("branch phase");
    let replay = store
        .prepare_coding_git_operation(&attempt, input)
        .expect("identical replay");
    assert_eq!(replay, branch);

    let drift = store.prepare_coding_git_operation(
        &attempt,
        PrepareCodingGitOperationInput {
            kind: CodingGitOperationKind::WorktreePrepare,
            repo_path: PathBuf::from("/different/repo"),
            worktree_path,
            branch_name: attempt.branch_name.clone(),
            base_branch: attempt.base_branch.clone(),
            before_head: "before-head".to_string(),
            remote: None,
            commit_message: None,
        },
    );
    assert!(matches!(
        drift,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn review_git_operation_requires_commit_identity_and_durable_completion_outcome() {
    let (tmp, store, attempt) = setup();
    let (repo_path, worktree_path) =
        init_review_repo_worktree(tmp.path(), &attempt.branch_name);
    let attempt = store
        .update_attempt_worktree_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            worktree_path.clone(),
        )
        .expect("persist worktree");
    let journal = store
        .prepare_coding_git_operation(
            &attempt,
            PrepareCodingGitOperationInput {
                kind: CodingGitOperationKind::ReviewRequest,
                repo_path,
                worktree_path,
                branch_name: attempt.branch_name.clone(),
                base_branch: attempt.base_branch.clone(),
                before_head: "before-head".to_string(),
                remote: Some("origin".to_string()),
                commit_message: Some("feat: review journal".to_string()),
            },
        )
        .expect("prepare review journal");
    let commit_started = store
        .advance_coding_git_operation(
            &attempt,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .expect("commit started");
    let missing_commit = store.advance_coding_git_operation(
        &attempt,
        &commit_started,
        CodingGitOperationPhase::CommitCreated,
        None,
    );
    assert!(matches!(
        missing_commit,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
    let commit_created = store
        .advance_coding_git_operation(
            &attempt,
            &commit_started,
            CodingGitOperationPhase::CommitCreated,
            Some("commit-sha".to_string()),
        )
        .expect("commit created");
    let push_started = store
        .advance_coding_git_operation(
            &attempt,
            &commit_created,
            CodingGitOperationPhase::PushStarted,
            None,
        )
        .expect("push started");
    let completed = store
        .complete_review_coding_git_operation(
            &attempt,
            &push_started,
            CompleteReviewGitOperationInput {
                push_status: PushStatus::Pushed,
                remote_kind: RemoteKind::GenericGit,
                review_request_id: "review_request_0001".to_string(),
                push_error: None,
            },
        )
        .expect("complete review journal");
    assert_eq!(completed.phase, CodingGitOperationPhase::Completed);
    assert_eq!(completed.push_status, Some(PushStatus::Pushed));
    assert_eq!(
        completed.review_request_id.as_deref(),
        Some("review_request_0001")
    );

    let replay = store
        .complete_review_coding_git_operation(
            &attempt,
            &push_started,
            CompleteReviewGitOperationInput {
                push_status: PushStatus::Pushed,
                remote_kind: RemoteKind::GenericGit,
                review_request_id: "review_request_0001".to_string(),
                push_error: None,
            },
        )
        .expect("idempotent complete replay");
    assert_eq!(replay, completed);
    let drift = store.complete_review_coding_git_operation(
        &attempt,
        &completed,
        CompleteReviewGitOperationInput {
            push_status: PushStatus::Failed,
            remote_kind: RemoteKind::GenericGit,
            review_request_id: "review_request_0001".to_string(),
            push_error: None,
        },
    );
    assert!(matches!(
        drift,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

fn completed_review_journal(
    tmp: &tempfile::TempDir,
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    push_status: PushStatus,
) -> crate::product::coding_attempt_store::CodingGitOperationJournal {
    // ReviewRequest journal 只校验 identity、不校验 git 事实,但进入 reviewer
    // 写通道前必须解析到真实 worktree 的 gitdir/common-dir。
    let (repo_path, worktree_path) =
        init_review_repo_worktree(tmp.path(), &attempt.branch_name);
    let attempt = store
        .update_attempt_worktree_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            worktree_path.clone(),
        )
        .expect("persist worktree");
    let journal = store
        .prepare_coding_git_operation(
            &attempt,
            PrepareCodingGitOperationInput {
                kind: CodingGitOperationKind::ReviewRequest,
                repo_path: repo_path.clone(),
                worktree_path: worktree_path.clone(),
                branch_name: attempt.branch_name.clone(),
                base_branch: attempt.base_branch.clone(),
                before_head: "before-head".to_string(),
                remote: Some("origin".to_string()),
                commit_message: Some("feat: retry review journal".to_string()),
            },
        )
        .expect("prepare review journal");
    let commit_started = store
        .advance_coding_git_operation(
            &attempt,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .expect("commit started");
    let commit_created = store
        .advance_coding_git_operation(
            &attempt,
            &commit_started,
            CodingGitOperationPhase::CommitCreated,
            Some("commit-sha".to_string()),
        )
        .expect("commit created");
    let push_started = store
        .advance_coding_git_operation(
            &attempt,
            &commit_created,
            CodingGitOperationPhase::PushStarted,
            None,
        )
        .expect("push started");
    let push_error = (push_status == PushStatus::Failed).then(|| "rejected".to_string());
    store
        .complete_review_coding_git_operation(
            &attempt,
            &push_started,
            CompleteReviewGitOperationInput {
                push_status,
                remote_kind: RemoteKind::GenericGit,
                review_request_id: "review_request_0001".to_string(),
                push_error,
            },
        )
        .expect("complete review journal")
}

#[test]
fn reopen_failed_review_git_operation_reopens_completed_failed_to_push_started() {
    let (tmp, store, attempt) = setup();
    let completed = completed_review_journal(&tmp, &store, &attempt, PushStatus::Failed);
    assert_eq!(completed.phase, CodingGitOperationPhase::Completed);
    assert_eq!(completed.push_status, Some(PushStatus::Failed));

    let attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    let reopened = store
        .reopen_failed_review_git_operation(&attempt, &completed)
        .expect("reopen failed review journal");
    assert_eq!(reopened.phase, CodingGitOperationPhase::PushStarted);
    assert_eq!(reopened.commit_sha.as_deref(), Some("commit-sha"));
    assert_eq!(reopened.push_status, None);
    assert_eq!(reopened.push_error, None);
    assert_eq!(reopened.remote_kind, None);
    assert_eq!(reopened.review_request_id, None);
}

#[test]
fn reopen_failed_review_git_operation_rejects_completed_pushed() {
    let (tmp, store, attempt) = setup();
    let completed = completed_review_journal(&tmp, &store, &attempt, PushStatus::Pushed);

    let attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    let reopened = store.reopen_failed_review_git_operation(&attempt, &completed);
    assert!(matches!(
        reopened,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn reopen_failed_review_git_operation_rejects_non_terminal_phase() {
    let (tmp, store, attempt) = setup();
    let completed = completed_review_journal(&tmp, &store, &attempt, PushStatus::Failed);
    let attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    let reopened = store
        .reopen_failed_review_git_operation(&attempt, &completed)
        .expect("reopen once");
    // PushStarted 为非终态，不得再次重开（编程错误 → IdentityMismatch，绝不静默）。
    let again = store.reopen_failed_review_git_operation(&attempt, &reopened);
    assert!(matches!(
        again,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}
