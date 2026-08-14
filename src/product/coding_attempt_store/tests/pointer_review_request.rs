use super::setup_store;
use crate::product::coding_models::{
    PushStatus, RemoteKind, ReviewRequest, ReviewRequestKind, ReviewRequestOwnerKind,
};

fn pointer_review_request(publication_id: &str, member_repo_id: &str) -> ReviewRequest {
    ReviewRequest {
        id: format!("rr-{publication_id}-{member_repo_id}"),
        attempt_id: format!("pointer-pub-{publication_id}"),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: format!("aria-pointer/{member_repo_id}/{publication_id}"),
        commit_sha: "abc123".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: Vec::new(),
        push_error: None,
        owner_kind: ReviewRequestOwnerKind::PointerPublication,
        pointer_publication_id: Some(publication_id.to_string()),
        revoked: false,
        created_at: "2026-08-14T00:00:00Z".to_string(),
        updated_at: "2026-08-14T00:00:00Z".to_string(),
    }
}

#[test]
fn pointer_review_request_roundtrips_in_publication_partition() {
    let (tmp, store) = setup_store();
    let request = pointer_review_request("pub_0001", "repo_a");

    store
        .save_pointer_review_request("project_0001", "pub_0001", &request)
        .expect("save pointer review request");

    let listed = store
        .list_pointer_review_requests("project_0001", "pub_0001")
        .expect("list pointer review requests");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], request);
    assert_eq!(
        listed[0].owner_kind,
        ReviewRequestOwnerKind::PointerPublication
    );
    assert_eq!(listed[0].attempt_id, "pointer-pub-pub_0001");

    // 落盘在独立分区：pointer-publications/{pid}/review-requests/{id}.json
    assert!(
        tmp.path()
            .join(".aria/projects/project_0001/logical-codebase/pointer-publications/pub_0001/review-requests/rr-pub_0001-repo_a.json")
            .is_file()
    );
}

#[test]
fn pointer_review_request_overwrites_by_id_and_lists_per_publication() {
    let (_tmp, store) = setup_store();
    let first = pointer_review_request("pub_0001", "repo_a");
    store
        .save_pointer_review_request("project_0001", "pub_0001", &first)
        .unwrap();

    // 同 id 覆盖更新（每仓一条）
    let mut updated = first.clone();
    updated.revoked = true;
    store
        .save_pointer_review_request("project_0001", "pub_0001", &updated)
        .unwrap();

    // 另一 publication 独立分区，互不混入
    let other = pointer_review_request("pub_0002", "repo_a");
    store
        .save_pointer_review_request("project_0001", "pub_0002", &other)
        .unwrap();

    let pub_1 = store
        .list_pointer_review_requests("project_0001", "pub_0001")
        .unwrap();
    assert_eq!(pub_1.len(), 1);
    assert!(pub_1[0].revoked);

    let pub_2 = store
        .list_pointer_review_requests("project_0001", "pub_0002")
        .unwrap();
    assert_eq!(pub_2.len(), 1);
    assert!(!pub_2[0].revoked);
}
