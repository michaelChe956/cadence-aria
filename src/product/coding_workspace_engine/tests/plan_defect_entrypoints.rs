use super::*;
use crate::product::work_item_contract::{BlockerRoute, BlockerRule, WorkItemWritePolicy};
use crate::product::work_item_projection::ReviewerWorkItemProjection;

#[test]
fn coding_plan_repair_entrypoints_internal_and_group_review_use_group_reviewer_route() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .unwrap();
    let request = ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt.id.clone(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: attempt.base_branch.clone(),
        branch_name: attempt.branch_name.clone(),
        commit_sha: "commit_0001".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: Vec::new(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    };
    let projection = reviewer_projection_fixture();

    for source_stage in ["internal_pr_review", "group_final_review"] {
        let output = serde_json::json!({
            "verdict": "blocked",
            "findings": [{
                "source_stage": source_stage,
                "severity": "error",
                "defect_class": "current_work_item_invalid",
                "reason_code": "current_work_item_contract_invalid",
                "message": "current contract invalid",
                "contract_refs": [],
                "capability_refs": [],
                "repair_target": {
                    "kind": "current_work_item",
                    "logical_work_item_ids": ["work_item_0001"],
                    "work_item_revision_ids": ["work_item_revision_0001"]
                },
                "recommended_route": "plan_repair",
                "confidence": "high",
                "evidence": []
            }]
        })
        .to_string();
        let review = engine
            .build_internal_pr_review(&attempt, &request, &output, None, &role_run)
            .unwrap();

        assert_eq!(
            internal_review_flow_decision(&review, &projection),
            CodeReviewFlowDecision::StartPlanRepair
        );
    }
}

#[tokio::test]
async fn coding_plan_repair_entrypoints_internal_review_execution_persists_safe_route() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let request = ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt.id.clone(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: attempt.base_branch.clone(),
        branch_name: attempt.branch_name.clone(),
        commit_sha: git_stdout(
            attempt.worktree_path.as_ref().unwrap(),
            &["rev-parse", "HEAD"],
        ),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: Vec::new(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    };
    store.save_review_request(&attempt, &request).unwrap();
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = super::provider_execution_context::CapturingProjectionProvider::new(
        super::provider_execution_context::review_plan_defect_output(),
    );

    let review = engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .unwrap();

    assert_eq!(review.verdict, ReviewVerdict::Blocked);
    let entry = store
        .list_chat_entries(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap()
        .into_iter()
        .find(|entry| {
            entry
                .metadata
                .as_ref()
                .is_some_and(|metadata| metadata["source"] == "internal_pr_review")
        })
        .expect("internal review chat entry");
    assert_eq!(
        entry.metadata.as_ref().unwrap()["plan_defect_route"],
        "stop_for_human_triage"
    );
}

fn reviewer_projection_fixture() -> ReviewerWorkItemProjection {
    ReviewerWorkItemProjection {
        work_item_revision_id: "work_item_revision_0001".to_string(),
        criterion_refs: Vec::new(),
        requirement_matrix: Vec::new(),
        scope_policy: WorkItemWritePolicy {
            exclusive_scopes: Vec::new(),
            forbidden_scopes: Vec::new(),
        },
        input_contract_checks: Vec::new(),
        output_contract_checks: Vec::new(),
        verification_evidence_rules: Vec::new(),
        blocker_routing: vec![BlockerRule {
            reason_code: "current_work_item_contract_invalid".to_string(),
            route: BlockerRoute::PlanRepairCurrent,
            target_contract_refs: Vec::new(),
        }],
    }
}
