use std::path::PathBuf;

use super::setup_store;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CodingGitOperationJournal, CodingGitOperationKind, CodingGitOperationPhase,
    CompleteReviewGitOperationInput,
};
use crate::product::coding_models::{PushStatus, RemoteKind};
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::logical_codebase::{
    PointerPublication, PointerPublicationBatchKind, PointerPublicationEntry,
    PointerPublicationEntryState, PointerPublicationStatus,
};
use tempfile::TempDir;

const POINTER_PROJECT_ID: &str = "project_0001";
const POINTER_PUBLICATION_ID: &str = "pub_0001";
const POINTER_MEMBER_REPO_ID: &str = "repo_a";

fn publication_fixture() -> PointerPublication {
    PointerPublication {
        id: POINTER_PUBLICATION_ID.to_string(),
        project_id: POINTER_PROJECT_ID.to_string(),
        logical_codebase_id: "lc_0001".to_string(),
        batch_kind: PointerPublicationBatchKind::Full,
        entries: vec![PointerPublicationEntry {
            member_repo_id: POINTER_MEMBER_REPO_ID.to_string(),
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
    member_repo_id: &str,
) -> CodingGitOperationJournal {
    let repo_path = tmp.path().join("pointer-repo");
    let worktree_path = repo_path.join(format!(
        ".worktrees/aria-pointer/{member_repo_id}/{}",
        publication.id
    ));
    store
        .start_pointer_publish_git_operation(
            publication,
            member_repo_id,
            &repo_path,
            &worktree_path,
            &format!("aria-pointer/{member_repo_id}/{}", publication.id),
            "main",
            "origin",
            "feat: publish pointer",
        )
        .expect("start pointer journal")
}

fn pointer_journal_path(
    tmp: &TempDir,
    publication: &PointerPublication,
    member_repo_id: &str,
) -> PathBuf {
    tmp.path()
        .join(".aria")
        .join("projects")
        .join(&publication.project_id)
        .join("logical-codebase")
        .join("pointer-publications")
        .join(&publication.id)
        .join("git-operations")
        .join(format!(
            "pointer-pub-{}-{member_repo_id}.json",
            publication.id
        ))
}

#[test]
fn pointer_publish_journal_full_phase_flow() {
    let (tmp, store) = setup_store();
    let publication = publication_fixture();

    let journal = start_pointer_journal(&tmp, &store, &publication, POINTER_MEMBER_REPO_ID);
    assert_eq!(journal.phase, CodingGitOperationPhase::Before);
    assert_eq!(journal.kind, CodingGitOperationKind::PointerPublish);
    assert_eq!(journal.attempt_id, "pointer-pub-pub_0001-repo_a");
    assert_eq!(journal.issue_id, "");
    assert_eq!(journal.project_id, publication.project_id);

    // 非法跳转（Before → Completed）被 is_allowed_transition 拒绝
    let jump = store.advance_pointer_publish_git_operation(
        &publication,
        POINTER_MEMBER_REPO_ID,
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
            POINTER_MEMBER_REPO_ID,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .expect("commit started");
    assert_eq!(commit_started.phase, CodingGitOperationPhase::CommitStarted);

    // CommitCreated 必须携带 commit_sha（与 ReviewRequest 规则一致）
    let missing_commit = store.advance_pointer_publish_git_operation(
        &publication,
        POINTER_MEMBER_REPO_ID,
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
            POINTER_MEMBER_REPO_ID,
            &commit_started,
            CodingGitOperationPhase::CommitCreated,
            Some("commit-sha".to_string()),
        )
        .expect("commit created");
    assert_eq!(commit_created.commit_sha.as_deref(), Some("commit-sha"));

    let push_started = store
        .advance_pointer_publish_git_operation(
            &publication,
            POINTER_MEMBER_REPO_ID,
            &commit_created,
            CodingGitOperationPhase::PushStarted,
            None,
        )
        .expect("push started");
    assert_eq!(push_started.phase, CodingGitOperationPhase::PushStarted);

    let completed = store
        .complete_review_pointer_publish_git_operation(
            &publication,
            POINTER_MEMBER_REPO_ID,
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
            POINTER_MEMBER_REPO_ID,
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

    // journal 落盘在 pointer-publications 分区，journal_id 含成员仓维度
    assert!(pointer_journal_path(&tmp, &publication, POINTER_MEMBER_REPO_ID).is_file());
}

#[test]
fn pointer_publish_validate_does_not_require_attempt_identity() {
    // setup_store 不创建任何 attempt 实体：PointerPublish 校验按 publication 走
    // （project_id / attempt_id / branch / base_branch / canonical 路径），
    // 不触碰 attempt.project_id/issue_id/branch_name 相等校验。
    let (tmp, store) = setup_store();
    let publication = publication_fixture();

    let journal = start_pointer_journal(&tmp, &store, &publication, POINTER_MEMBER_REPO_ID);
    assert_eq!(journal.attempt_id, "pointer-pub-pub_0001-repo_a");
    assert_eq!(journal.issue_id, "");
    assert_eq!(journal.project_id, publication.project_id);

    // 无 attempt 实体时 advance 仍成功 → 证明未走 attempt-bound 校验
    let advanced = store
        .advance_pointer_publish_git_operation(
            &publication,
            POINTER_MEMBER_REPO_ID,
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
    let journal = start_pointer_journal(&tmp, &store, &publication, POINTER_MEMBER_REPO_ID);
    let commit_started = store
        .advance_pointer_publish_git_operation(
            &publication,
            POINTER_MEMBER_REPO_ID,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .unwrap();
    let commit_created = store
        .advance_pointer_publish_git_operation(
            &publication,
            POINTER_MEMBER_REPO_ID,
            &commit_started,
            CodingGitOperationPhase::CommitCreated,
            Some("commit-sha".to_string()),
        )
        .unwrap();
    let push_started = store
        .advance_pointer_publish_git_operation(
            &publication,
            POINTER_MEMBER_REPO_ID,
            &commit_created,
            CodingGitOperationPhase::PushStarted,
            None,
        )
        .unwrap();
    let completed = store
        .complete_review_pointer_publish_git_operation(
            &publication,
            POINTER_MEMBER_REPO_ID,
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
        .reopen_failed_pointer_publish_git_operation(
            &publication,
            POINTER_MEMBER_REPO_ID,
            &completed,
        )
        .expect("reopen failed pointer journal");
    assert_eq!(reopened.phase, CodingGitOperationPhase::PushStarted);
    assert_eq!(reopened.commit_sha.as_deref(), Some("commit-sha"));
    assert_eq!(reopened.push_status, None);
    assert_eq!(reopened.push_error, None);
    assert_eq!(reopened.remote_kind, None);
    assert_eq!(reopened.review_request_id, None);

    // PushStarted 为非终态，不得再次重开
    let again = store.reopen_failed_pointer_publish_git_operation(
        &publication,
        POINTER_MEMBER_REPO_ID,
        &reopened,
    );
    assert!(matches!(
        again,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn pointer_publish_journal_rejects_pushed_completion_reopen() {
    let (tmp, store) = setup_store();
    let publication = publication_fixture();
    let journal = start_pointer_journal(&tmp, &store, &publication, POINTER_MEMBER_REPO_ID);
    let commit_started = store
        .advance_pointer_publish_git_operation(
            &publication,
            POINTER_MEMBER_REPO_ID,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .unwrap();
    let commit_created = store
        .advance_pointer_publish_git_operation(
            &publication,
            POINTER_MEMBER_REPO_ID,
            &commit_started,
            CodingGitOperationPhase::CommitCreated,
            Some("commit-sha".to_string()),
        )
        .unwrap();
    let push_started = store
        .advance_pointer_publish_git_operation(
            &publication,
            POINTER_MEMBER_REPO_ID,
            &commit_created,
            CodingGitOperationPhase::PushStarted,
            None,
        )
        .unwrap();
    let completed = store
        .complete_review_pointer_publish_git_operation(
            &publication,
            POINTER_MEMBER_REPO_ID,
            &push_started,
            CompleteReviewGitOperationInput {
                push_status: PushStatus::Pushed,
                remote_kind: RemoteKind::GenericGit,
                review_request_id: "review_request_0001".to_string(),
                push_error: None,
            },
        )
        .unwrap();

    let reopened = store.reopen_failed_pointer_publish_git_operation(
        &publication,
        POINTER_MEMBER_REPO_ID,
        &completed,
    );
    assert!(matches!(
        reopened,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn pointer_publish_journal_kind_dispatch_rejects_wrong_identity() {
    let (tmp, store) = setup_store();
    let publication = publication_fixture();
    let journal = start_pointer_journal(&tmp, &store, &publication, POINTER_MEMBER_REPO_ID);

    // 篡改落盘 attempt_id：按 kind 分流后 PointerPublish 分支应拒绝（按 publication 校验）
    let mut tampered = journal.clone();
    tampered.attempt_id = "pointer-pub-wrong".to_string();
    write_json(
        &pointer_journal_path(&tmp, &publication, POINTER_MEMBER_REPO_ID),
        &tampered,
    )
    .unwrap();

    let advanced = store.advance_pointer_publish_git_operation(
        &publication,
        POINTER_MEMBER_REPO_ID,
        &journal,
        CodingGitOperationPhase::CommitStarted,
        None,
    );
    assert!(matches!(
        advanced,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));

    // 篡改 branch_name 为空：分支模式/非空校验应拒绝
    let mut tampered = journal.clone();
    tampered.branch_name = " ".to_string();
    write_json(
        &pointer_journal_path(&tmp, &publication, POINTER_MEMBER_REPO_ID),
        &tampered,
    )
    .unwrap();
    let advanced = store.advance_pointer_publish_git_operation(
        &publication,
        POINTER_MEMBER_REPO_ID,
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
        "pointer-pub-pub_0001-repo_a",
    );
    assert!(pointer_path.ends_with(
        "pointer-publications/pub_0001/git-operations/pointer-pub-pub_0001-repo_a.json"
    ));
    assert_ne!(attempt_path, pointer_path);
}

#[test]
fn pointer_publish_distinct_member_repo_journals_do_not_overwrite() {
    let (tmp, store) = setup_store();
    let mut publication = publication_fixture();
    publication.entries.push(PointerPublicationEntry {
        member_repo_id: "repo_b".to_string(),
        state: PointerPublicationEntryState::Pending,
        branch_name: None,
        commit_sha: None,
        push_error: None,
        conflict_detail: None,
    });

    let repo_path = tmp.path().join("pointer-repo");
    let worktree_a = repo_path.join(".worktrees/aria-pointer/repo_a/pub_0001");
    let worktree_b = repo_path.join(".worktrees/aria-pointer/repo_b/pub_0001");

    let journal_a = store
        .start_pointer_publish_git_operation(
            &publication,
            "repo_a",
            &repo_path,
            &worktree_a,
            "aria-pointer/repo_a/pub_0001",
            "main",
            "origin",
            "feat: publish pointer",
        )
        .expect("repo_a journal");

    // repo_a 推进到非终态 CommitStarted：旧实现单 journal 文件会阻塞 repo_b start
    let advanced_a = store
        .advance_pointer_publish_git_operation(
            &publication,
            "repo_a",
            &journal_a,
            CodingGitOperationPhase::CommitStarted,
            None,
        )
        .expect("advance repo_a");
    assert_eq!(advanced_a.phase, CodingGitOperationPhase::CommitStarted);

    // repo_b 独立 journal：不被 repo_a 非终态残留阻塞，也不覆盖 repo_a
    let journal_b = store
        .start_pointer_publish_git_operation(
            &publication,
            "repo_b",
            &repo_path,
            &worktree_b,
            "aria-pointer/repo_b/pub_0001",
            "main",
            "origin",
            "feat: publish pointer",
        )
        .expect("repo_b journal");
    assert_eq!(journal_b.phase, CodingGitOperationPhase::Before);
    assert_eq!(journal_b.attempt_id, "pointer-pub-pub_0001-repo_b");

    let path_a = pointer_journal_path(&tmp, &publication, "repo_a");
    let path_b = pointer_journal_path(&tmp, &publication, "repo_b");
    assert_ne!(path_a, path_b);
    assert!(path_a.is_file());
    assert!(path_b.is_file());

    // repo_a journal 仍为 CommitStarted，未被 repo_b start 覆盖
    let persisted_a: CodingGitOperationJournal = read_json(&path_a).unwrap();
    assert_eq!(persisted_a.phase, CodingGitOperationPhase::CommitStarted);
    assert_eq!(persisted_a.attempt_id, "pointer-pub-pub_0001-repo_a");
}

#[test]
fn pointer_publish_rejects_branch_name_not_matching_pattern() {
    let (tmp, store) = setup_store();
    let publication = publication_fixture();
    let repo_path = tmp.path().join("pointer-repo");
    let worktree_path = repo_path.join(".worktrees/aria-pointer/repo_a/pub_0001");

    // 分支名成员仓维度与 member_repo_id 不符 → 被模式校验拒绝
    let err = store.start_pointer_publish_git_operation(
        &publication,
        "repo_a",
        &repo_path,
        &worktree_path,
        "aria-pointer/repo_b/pub_0001",
        "main",
        "origin",
        "feat: publish pointer",
    );
    assert!(matches!(
        err,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn pointer_publish_rejects_non_empty_issue_id() {
    let (tmp, store) = setup_store();
    let publication = publication_fixture();
    let journal = start_pointer_journal(&tmp, &store, &publication, POINTER_MEMBER_REPO_ID);

    let mut tampered = journal.clone();
    tampered.issue_id = "issue_0001".to_string();
    write_json(
        &pointer_journal_path(&tmp, &publication, POINTER_MEMBER_REPO_ID),
        &tampered,
    )
    .unwrap();

    let advanced = store.advance_pointer_publish_git_operation(
        &publication,
        POINTER_MEMBER_REPO_ID,
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
fn pointer_publish_rejects_member_not_in_publication_entries() {
    let (tmp, store) = setup_store();
    let publication = publication_fixture(); // entries 仅含 repo_a
    let repo_path = tmp.path().join("pointer-repo");
    let worktree_path = repo_path.join(".worktrees/aria-pointer/repo_b/pub_0001");

    // member_repo_id 不在 publication.entries 中 → 被归属校验拒绝
    let err = store.start_pointer_publish_git_operation(
        &publication,
        "repo_b",
        &repo_path,
        &worktree_path,
        "aria-pointer/repo_b/pub_0001",
        "main",
        "origin",
        "feat: publish pointer",
    );
    assert!(matches!(
        err,
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}
