use std::path::PathBuf;

use super::setup_store;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CodingGitOperationJournal, CodingGitOperationKind, CodingGitOperationPhase,
    CompleteReviewGitOperationInput,
};
use crate::product::coding_models::{PushStatus, RemoteKind};
use crate::product::json_store::{ProductStoreError, write_json};
use crate::product::logical_codebase::{
    PointerPublication, PointerPublicationBatchKind, PointerPublicationEntry,
    PointerPublicationEntryState, PointerPublicationStatus,
};
use tempfile::TempDir;

const POINTER_PROJECT_ID: &str = "project_0001";
const POINTER_PUBLICATION_ID: &str = "pub_0001";

fn publication_fixture() -> PointerPublication {
    PointerPublication {
        id: POINTER_PUBLICATION_ID.to_string(),
        project_id: POINTER_PROJECT_ID.to_string(),
        logical_codebase_id: "lc_0001".to_string(),
        batch_kind: PointerPublicationBatchKind::Full,
        entries: vec![PointerPublicationEntry {
            member_repo_id: "repo_a".to_string(),
            state: PointerPublicationEntryState::Pending,
            branch_name: None,
            commit_sha: None,
            push_error: None,
            conflict_detail: None,
        }],
        status: PointerPublicationStatus::InProgress,
        created_at: "2026-08-14T00:00:00Z".to_string(),
        updated_at: "2026-08-14T00:00:00Z".to_string(),
    }
}

fn start_pointer_journal(
    tmp: &TempDir,
    store: &CodingAttemptStore,
    publication: &PointerPublication,
) -> CodingGitOperationJournal {
    let repo_path = tmp.path().join("pointer-repo");
    let worktree_path = repo_path.join(".worktrees/aria-pointer/repo_a/pub_0001");
    store
        .start_pointer_publish_git_operation(
            publication,
            &repo_path,
            &worktree_path,
            "aria-pointer/repo_a/pub_0001",
            "main",
            "origin",
            "feat: publish pointer",
        )
        .expect("start pointer journal")
}

fn pointer_journal_path(tmp: &TempDir, publication: &PointerPublication) -> PathBuf {
    tmp.path()
        .join(".aria")
        .join("projects")
        .join(&publication.project_id)
        .join("logical-codebase")
        .join("pointer-publications")
        .join(&publication.id)
        .join("git-operations")
        .join(format!("pointer-pub-{}.json", publication.id))
}

#[test]
fn pointer_publish_journal_full_phase_flow() {
    let (tmp, store) = setup_store();
    let publication = publication_fixture();

    let journal = start_pointer_journal(&tmp, &store, &publication);
    assert_eq!(journal.phase, CodingGitOperationPhase::Before);
    assert_eq!(journal.kind, CodingGitOperationKind::PointerPublish);
    assert_eq!(journal.attempt_id, "pointer-pub-pub_0001");
    assert_eq!(journal.issue_id, "");
    assert_eq!(journal.project_id, publication.project_id);

    // 非法跳转（Before → Completed）被 is_allowed_transition 拒绝
    let jump = store.advance_pointer_publish_git_operation(
        &publication,
        &journal,
        CodingGitOperationPhase::Completed,
        None,
    );
    assert!(matches!(
        jump,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));

    let commit_started = store
        .advance_pointer_publish_git_operation(
            &publication,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .expect("commit started");
    assert_eq!(commit_started.phase, CodingGitOperationPhase::CommitStarted);

    // CommitCreated 必须携带 commit_sha（与 ReviewRequest 规则一致）
    let missing_commit = store.advance_pointer_publish_git_operation(
        &publication,
        &commit_started,
        CodingGitOperationPhase::CommitCreated,
        None,
    );
    assert!(matches!(
        missing_commit,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));

    let commit_created = store
        .advance_pointer_publish_git_operation(
            &publication,
            &commit_started,
            CodingGitOperationPhase::CommitCreated,
            Some("commit-sha".to_string()),
        )
        .expect("commit created");
    assert_eq!(commit_created.commit_sha.as_deref(), Some("commit-sha"));

    let push_started = store
        .advance_pointer_publish_git_operation(
            &publication,
            &commit_created,
            CodingGitOperationPhase::PushStarted,
            None,
        )
        .expect("push started");
    assert_eq!(push_started.phase, CodingGitOperationPhase::PushStarted);

    let completed = store
        .complete_review_pointer_publish_git_operation(
            &publication,
            &push_started,
            CompleteReviewGitOperationInput {
                push_status: PushStatus::Pushed,
                remote_kind: RemoteKind::GenericGit,
                review_request_id: "review_request_0001".to_string(),
                push_error: None,
            },
        )
        .expect("complete pointer journal");
    assert_eq!(completed.phase, CodingGitOperationPhase::Completed);
    assert_eq!(completed.push_status, Some(PushStatus::Pushed));
    assert_eq!(completed.remote_kind, Some(RemoteKind::GenericGit));
    assert_eq!(
        completed.review_request_id.as_deref(),
        Some("review_request_0001")
    );

    // 幂等重放
    let replay = store
        .complete_review_pointer_publish_git_operation(
            &publication,
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

    // journal 落盘在 pointer-publications 分区
    assert!(pointer_journal_path(&tmp, &publication).is_file());
}

#[test]
fn pointer_publish_validate_does_not_require_attempt_identity() {
    // setup_store 不创建任何 attempt 实体：PointerPublish 校验按 publication 走
    // （project_id / attempt_id / branch / base_branch / canonical 路径），
    // 不触碰 attempt.project_id/issue_id/branch_name 相等校验。
    let (tmp, store) = setup_store();
    let publication = publication_fixture();

    let journal = start_pointer_journal(&tmp, &store, &publication);
    assert_eq!(journal.attempt_id, "pointer-pub-pub_0001");
    assert_eq!(journal.issue_id, "");
    assert_eq!(journal.project_id, publication.project_id);

    // 无 attempt 实体时 advance 仍成功 → 证明未走 attempt-bound 校验
    let advanced = store
        .advance_pointer_publish_git_operation(
            &publication,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .expect("advance without any attempt entity");
    assert_eq!(advanced.phase, CodingGitOperationPhase::CommitStarted);
}

#[test]
fn reopen_failed_pointer_publish_resumes_from_push_failed() {
    let (tmp, store) = setup_store();
    let publication = publication_fixture();
    let journal = start_pointer_journal(&tmp, &store, &publication);
    let commit_started = store
        .advance_pointer_publish_git_operation(
            &publication,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .unwrap();
    let commit_created = store
        .advance_pointer_publish_git_operation(
            &publication,
            &commit_started,
            CodingGitOperationPhase::CommitCreated,
            Some("commit-sha".to_string()),
        )
        .unwrap();
    let push_started = store
        .advance_pointer_publish_git_operation(
            &publication,
            &commit_created,
            CodingGitOperationPhase::PushStarted,
            None,
        )
        .unwrap();
    let completed = store
        .complete_review_pointer_publish_git_operation(
            &publication,
            &push_started,
            CompleteReviewGitOperationInput {
                push_status: PushStatus::Failed,
                remote_kind: RemoteKind::GenericGit,
                review_request_id: "review_request_0001".to_string(),
                push_error: Some("rejected".to_string()),
            },
        )
        .unwrap();
    assert_eq!(completed.phase, CodingGitOperationPhase::Completed);
    assert_eq!(completed.push_status, Some(PushStatus::Failed));

    let reopened = store
        .reopen_failed_pointer_publish_git_operation(&publication, &completed)
        .expect("reopen failed pointer journal");
    assert_eq!(reopened.phase, CodingGitOperationPhase::PushStarted);
    assert_eq!(reopened.commit_sha.as_deref(), Some("commit-sha"));
    assert_eq!(reopened.push_status, None);
    assert_eq!(reopened.push_error, None);
    assert_eq!(reopened.remote_kind, None);
    assert_eq!(reopened.review_request_id, None);

    // PushStarted 为非终态，不得再次重开
    let again = store.reopen_failed_pointer_publish_git_operation(&publication, &reopened);
    assert!(matches!(
        again,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn pointer_publish_journal_rejects_pushed_completion_reopen() {
    let (tmp, store) = setup_store();
    let publication = publication_fixture();
    let journal = start_pointer_journal(&tmp, &store, &publication);
    let commit_started = store
        .advance_pointer_publish_git_operation(
            &publication,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .unwrap();
    let commit_created = store
        .advance_pointer_publish_git_operation(
            &publication,
            &commit_started,
            CodingGitOperationPhase::CommitCreated,
            Some("commit-sha".to_string()),
        )
        .unwrap();
    let push_started = store
        .advance_pointer_publish_git_operation(
            &publication,
            &commit_created,
            CodingGitOperationPhase::PushStarted,
            None,
        )
        .unwrap();
    let completed = store
        .complete_review_pointer_publish_git_operation(
            &publication,
            &push_started,
            CompleteReviewGitOperationInput {
                push_status: PushStatus::Pushed,
                remote_kind: RemoteKind::GenericGit,
                review_request_id: "review_request_0001".to_string(),
                push_error: None,
            },
        )
        .unwrap();

    let reopened = store.reopen_failed_pointer_publish_git_operation(&publication, &completed);
    assert!(matches!(
        reopened,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn pointer_publish_journal_kind_dispatch_rejects_wrong_identity() {
    let (tmp, store) = setup_store();
    let publication = publication_fixture();
    let journal = start_pointer_journal(&tmp, &store, &publication);

    // 篡改落盘 attempt_id：按 kind 分流后 PointerPublish 分支应拒绝（按 publication 校验）
    let mut tampered = journal.clone();
    tampered.attempt_id = "pointer-pub-wrong".to_string();
    write_json(&pointer_journal_path(&tmp, &publication), &tampered).unwrap();

    let advanced = store.advance_pointer_publish_git_operation(
        &publication,
        &journal,
        CodingGitOperationPhase::CommitStarted,
        None,
    );
    assert!(matches!(
        advanced,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));

    // 篡改 branch_name 为空：branch 非空校验应拒绝
    let mut tampered = journal.clone();
    tampered.branch_name = " ".to_string();
    write_json(&pointer_journal_path(&tmp, &publication), &tampered).unwrap();
    let advanced = store.advance_pointer_publish_git_operation(
        &publication,
        &journal,
        CodingGitOperationPhase::CommitStarted,
        None,
    );
    assert!(matches!(
        advanced,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn attempt_journal_path_unchanged_after_pointer_kind_addition() {
    let (_tmp, store) = setup_store();
    let attempt_path =
        store.coding_git_operation_path(POINTER_PROJECT_ID, "issue_0001", "attempt_0001");
    assert!(attempt_path.ends_with("coding-attempts/attempt_0001/git-operation.json"));

    let pointer_path = store.pointer_publication_git_operation_path(
        POINTER_PROJECT_ID,
        POINTER_PUBLICATION_ID,
        "pointer-pub-pub_0001",
    );
    assert!(
        pointer_path
            .ends_with("pointer-publications/pub_0001/git-operations/pointer-pub-pub_0001.json")
    );
    assert_ne!(attempt_path, pointer_path);
}
