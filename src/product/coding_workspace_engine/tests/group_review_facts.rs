use super::*;
use crate::product::coding_models::{
    CodingUnitRun, CodingUnitRunStatus, PushStatus, RemoteKind, ReviewRequest, ReviewRequestKind,
};
use crate::product::coding_workspace_engine::plan_defect_routing::{
    AuthoritativeGroupReviewerBinding, GroupReviewerProjectionBinding,
};
use crate::product::work_item_contract::WorkItemWritePolicy;
use crate::product::work_item_projection::ReviewerWorkItemProjection;

fn facts_binding(
    id: &str,
    start_commit: Option<String>,
    completion_commit: Option<String>,
) -> AuthoritativeGroupReviewerBinding {
    AuthoritativeGroupReviewerBinding {
        order_index: 0,
        run: CodingUnitRun {
            id: id.to_string(),
            unit_id: format!("unit_{id}"),
            execution_no: 1,
            work_item_revision_id: format!("revision_{id}"),
            resolved_handoff_revision_ids: Vec::new(),
            canonical_contract_hash: format!("contract_{id}"),
            projection_bundle_id: format!("bundle_{id}"),
            projection_compiler_version: "v1".to_string(),
            coder_provider_renderer_version: "v1".to_string(),
            reviewer_provider_renderer_version: "v1".to_string(),
            internal_reviewer_provider_renderer_version: None,
            coder_projection_hash: "coder".to_string(),
            reviewer_projection_hash: "reviewer".to_string(),
            coder_execution_context_hash: None,
            reviewer_execution_context_hash: None,
            internal_reviewer_execution_context_hash: None,
            status: CodingUnitRunStatus::Completed,
            unit_rework_count: 0,
            verification_retry_count: 0,
            operational_retry_count: 0,
            plan_repair_count: 0,
            start_commit,
            completion_commit,
            created_at: "2026-08-04T00:00:00Z".to_string(),
            updated_at: "2026-08-04T00:00:00Z".to_string(),
        },
        projection_binding: GroupReviewerProjectionBinding {
            logical_work_item_id: format!("work_{id}"),
            projection: ReviewerWorkItemProjection {
                work_item_revision_id: format!("revision_{id}"),
                criterion_refs: Vec::new(),
                requirement_matrix: Vec::new(),
                scope_policy: WorkItemWritePolicy {
                    exclusive_scopes: Vec::new(),
                    forbidden_scopes: Vec::new(),
                },
                input_contract_checks: Vec::new(),
                output_contract_checks: Vec::new(),
                verification_evidence_rules: Vec::new(),
                blocker_routing: Vec::new(),
            },
        },
    }
}

fn facts_request(attempt: &CodingExecutionAttempt, final_commit: String) -> ReviewRequest {
    ReviewRequest {
        id: "review_request_facts".to_string(),
        attempt_id: attempt.id.clone(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: attempt.base_branch.clone(),
        branch_name: attempt.branch_name.clone(),
        commit_sha: final_commit,
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: Vec::new(),
        push_error: None,
        created_at: "2026-08-04T00:00:00Z".to_string(),
        updated_at: "2026-08-04T00:00:00Z".to_string(),
    }
}

#[tokio::test]
async fn collect_group_git_facts_collects_sorted_patches_stats_and_reachability() {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    init_test_git_repo(&worktree);
    let base_commit = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    fs::write(worktree.join("alpha.txt"), "alpha\n").expect("alpha");
    run_test_git(&worktree, &["add", "."]);
    run_test_git(&worktree, &["commit", "-m", "alpha"]);
    let alpha_commit = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    fs::write(worktree.join("beta.txt"), "beta\n").expect("beta");
    run_test_git(&worktree, &["add", "."]);
    run_test_git(&worktree, &["commit", "-m", "beta"]);
    let final_commit = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx);
    let mut attempt = test_attempt("attempt_group_git_facts");
    attempt.base_branch = base_commit.clone();
    attempt.worktree_path = Some(worktree.clone());
    let bindings = vec![
        facts_binding(
            "run_z",
            Some(alpha_commit.clone()),
            Some(final_commit.clone()),
        ),
        facts_binding(
            "run_a",
            Some(base_commit.clone()),
            Some(alpha_commit.clone()),
        ),
    ];

    let facts = engine
        .collect_group_git_facts(
            &attempt,
            &bindings,
            &facts_request(&attempt, final_commit.clone()),
            &worktree,
        )
        .await
        .expect("collect group git facts");

    assert_eq!(facts.final_commit, final_commit);
    assert!(facts.final_diff.contains("alpha.txt"));
    assert!(facts.final_diff.contains("beta.txt"));
    assert!(facts.diff_stat.contains("1\t0\talpha.txt"));
    assert!(facts.diff_stat.contains("1\t0\tbeta.txt"));
    assert_eq!(
        facts
            .completion_diffs
            .iter()
            .map(|diff| diff.unit_run_id.as_str())
            .collect::<Vec<_>>(),
        vec!["run_a", "run_z"]
    );
    assert!(facts.completion_diffs[0].patch.contains("alpha.txt"));
    assert!(facts.completion_diffs[1].patch.contains("beta.txt"));
    assert!(
        facts
            .completion_diffs
            .iter()
            .all(|diff| diff.hunks.is_empty())
    );
    assert!(
        facts
            .completion_diffs
            .iter()
            .all(|diff| diff.file_stats.is_empty())
    );
    assert!(facts.completion_commit_in_final.contains(&alpha_commit));
    assert!(
        facts
            .completion_commit_in_final
            .contains(&facts.final_commit)
    );
}

#[tokio::test]
async fn collect_group_git_facts_fails_closed_without_completion_commit() {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    init_test_git_repo(&worktree);
    let head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx);
    let mut attempt = test_attempt("attempt_group_git_facts_missing_commit");
    attempt.base_branch = head.clone();
    let binding = facts_binding("run_missing", Some(head.clone()), None);

    let error = engine
        .collect_group_git_facts(
            &attempt,
            &[binding],
            &facts_request(&attempt, head),
            &worktree,
        )
        .await
        .expect_err("missing completion commit must fail closed");

    assert!(error.to_string().contains("run_missing"));
}
