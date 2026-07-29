use std::path::PathBuf;

use super::setup;
use crate::product::coding_attempt_store::{
    CodingGitOperationKind, CodingGitOperationPhase, CompleteReviewGitOperationInput,
    PrepareCodingGitOperationInput,
};
use crate::product::coding_models::{PushStatus, RemoteKind};
use crate::product::json_store::ProductStoreError;

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
    let repo_path = tmp.path().join("repo");
    let worktree_path = repo_path.join(".worktrees/aria-work-items/work_item_0001/attempt-1");
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
