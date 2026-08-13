use crate::product::coding_models::{RemoteKind, ReviewRequest, ReviewRequestKind};

fn manual_recovery_attempt_fixture() -> CodingExecutionAttempt {
    CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::AwaitingManualRecovery,
        version: 0,
        manual_recovery_reason: Some("code_review_blocked".to_string()),
        admission_ticket_consumed_at: None,
        stage: CodingExecutionStage::CodeReview,
        base_branch: "HEAD".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Fake),
            review_rounds: 1,
            permission_modes: WorkspaceRolePermissionModes::default(),
        },
        provider_conversations: Vec::new(),
        rework_count: 0,
        max_auto_rework: 2,
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        created_at: "2026-06-12T00:00:00Z".to_string(),
        updated_at: "2026-06-12T00:00:00Z".to_string(),
        target_snapshot: None,
        completed_at: None,
    }
}

fn store_fixture() -> (tempfile::TempDir, CodingAttemptStore) {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CodingAttemptStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    (tmp, store)
}

fn failed_review_request_fixture(attempt: &CodingExecutionAttempt) -> ReviewRequest {
    ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt.id.clone(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: attempt.branch_name.clone(),
        commit_sha: "sha111".to_string(),
        push_status: PushStatus::Failed,
        external_url: None,
        manual_instructions: Vec::new(),
        push_error: Some("push rejected".to_string()),
        created_at: "2026-06-12T00:00:00Z".to_string(),
        updated_at: "2026-06-12T00:00:00Z".to_string(),
    }
}

#[test]
fn coding_attempt_dto_exposes_manual_recovery_reason() {
    let (_tmp, store) = store_fixture();
    let dto = coding_attempt_dto(&store, &manual_recovery_attempt_fixture()).unwrap();
    assert_eq!(dto.status, "awaiting_manual_recovery");
    assert_eq!(
        dto.manual_recovery_reason,
        Some("code_review_blocked".to_string())
    );
}

#[test]
fn coding_attempt_dto_manual_recovery_reason_is_none_when_absent() {
    let (_tmp, store) = store_fixture();
    let mut attempt = manual_recovery_attempt_fixture();
    attempt.manual_recovery_reason = None;
    let dto = coding_attempt_dto(&store, &attempt).unwrap();
    assert_eq!(dto.status, "awaiting_manual_recovery");
    assert_eq!(dto.manual_recovery_reason, None);
}

#[test]
fn coding_attempt_dto_projects_failed_push_status_from_review_request() {
    let (_tmp, store) = store_fixture();
    let mut attempt = manual_recovery_attempt_fixture();
    attempt.status = CodingAttemptStatus::Completed;
    attempt.head_commit = Some("sha111".to_string());
    store.write_coding_attempt_for_test(&attempt).unwrap();
    store
        .save_review_request(&attempt, &failed_review_request_fixture(&attempt))
        .unwrap();

    let dto = coding_attempt_dto(&store, &attempt).unwrap();

    assert_eq!(dto.push_status.as_deref(), Some("failed"));
}

#[test]
fn coding_attempt_dto_falls_back_to_pushed_remote_without_review_request() {
    let (_tmp, store) = store_fixture();
    let mut attempt = manual_recovery_attempt_fixture();
    attempt.pushed_remote = Some("origin".to_string());
    store.write_coding_attempt_for_test(&attempt).unwrap();

    let dto = coding_attempt_dto(&store, &attempt).unwrap();

    assert_eq!(dto.push_status.as_deref(), Some("pushed"));
}
