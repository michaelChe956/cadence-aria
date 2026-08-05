use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use crate::product::coding_models::ReviewVerdict;
use crate::product::coding_workspace_engine::CodeReviewFlowDecision;
use crate::product::coding_workspace_engine::group_review_errors::{
    GroupReviewExecutionError, GroupReviewOrchestrationError,
};
use crate::product::coding_workspace_engine::group_review_orchestrator::{
    FakeGroupReviewExecutor, GroupReviewExecutionResult, GroupReviewOrchestrator,
};
use crate::product::coding_workspace_engine::group_review_types::{
    GroupDiffIndex, GroupPartitionResult, GroupReviewGraph, GroupReviewMaterialSnapshot,
    GroupShardSpec, ReductionDiffSelection,
};
use crate::product::coding_workspace_engine::plan_defect_routing::{
    GroupReviewerProjectionBinding, internal_review_flow_decision_with_bindings,
};
use crate::product::models::{PlanDefectClass, PlanDefectRoute, ProviderName};
use crate::product::work_item_contract::{BlockerRoute, BlockerRule, WorkItemWritePolicy};
use crate::product::work_item_projection::ReviewerWorkItemProjection;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use tempfile::TempDir;

fn setup() -> (TempDir, CodingAttemptStore, String) {
    let root = TempDir::new().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-item".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            max_auto_rework: 2,
        })
        .expect("attempt");
    (root, store, attempt.id)
}

fn snapshot(attempt_id: String) -> GroupReviewMaterialSnapshot {
    GroupReviewMaterialSnapshot {
        schema_version: 1,
        compiler_version: "test".to_string(),
        attempt_id,
        review_request_id: "review_request_0001".to_string(),
        base_branch: "main".to_string(),
        final_commit: "final".to_string(),
        authoritative_binding_digest: "binding".to_string(),
        unit_records: Vec::new(),
        global_graph: GroupReviewGraph {
            contract_edges: Vec::new(),
            scope_overlaps: Vec::new(),
            commit_reachability:
                crate::product::coding_workspace_engine::group_review_types::CommitReachability {
                    reachable_completion_commits: Vec::new(),
                    unreachable_completion_commits: Vec::new(),
                },
            requirement_coverage:
                crate::product::coding_workspace_engine::group_review_types::RequirementCoverage {
                    covered: Vec::new(),
                    missing: Vec::new(),
                    conflicting: Vec::new(),
                },
        },
        diff_index: GroupDiffIndex {
            files: Vec::new(),
            hunks: Vec::new(),
            shard_selections: Vec::new(),
            reduction_selection: ReductionDiffSelection {
                fragments: Vec::new(),
                total_cross_shard_hunks: 0,
            },
        },
        deterministic_findings: Vec::new(),
        partition_result: GroupPartitionResult {
            shards: vec![GroupShardSpec {
                shard_id: "a".to_string(),
                ordered_unit_run_ids: Vec::new(),
                partition_rationale: Vec::new(),
            }],
            cross_shard_edges: Vec::new(),
        },
        content_hash: "snapshot_hash".to_string(),
    }
}

fn incident_output() -> String {
    format!(
        "GROUP_REVIEW_VERDICT: request_changes\n{}",
        serde_json::json!({
            "verdict": "request_changes",
            "summary": "verification evidence is missing",
            "findings": (0..4).map(|index| serde_json::json!({
                "severity": if index == 0 { "error" } else { "warning" },
                "message": format!("verification evidence is missing {index}"),
                "defect_class": "missing_verification_evidence",
                "reason_code": "verification_evidence_incomplete",
                "repair_target": "VerificationRetry",
                "recommended_route": "VerificationRetry",
                "confidence": "high",
                "evidence": [format!("verification evidence {index} is absent")]
            })).collect::<Vec<_>>(),
            "impact_scope": ["verification"],
            "pr_description": "",
            "commit_message_suggestion": "",
            "tested_evidence_refs": [],
            "diff_refs": []
        })
    )
}

fn verification_binding() -> GroupReviewerProjectionBinding {
    GroupReviewerProjectionBinding {
        logical_work_item_id: "work_item_0001".to_string(),
        projection: ReviewerWorkItemProjection {
            work_item_revision_id: "revision_0001".to_string(),
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
                reason_code: "verification_evidence_incomplete".to_string(),
                route: BlockerRoute::VerificationRetry,
                target_contract_refs: Vec::new(),
            }],
        },
    }
}

#[tokio::test]
async fn incident_aliases_normalize_through_shard_and_reduction_to_verification_retry() {
    let (_root, store, attempt_id) = setup();
    let snapshot = snapshot(attempt_id.clone());
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");
    let executor = FakeGroupReviewExecutor::new(vec![
        Ok(GroupReviewExecutionResult {
            full_output: incident_output(),
            role_run_id: None,
        }),
        Ok(GroupReviewExecutionResult {
            full_output: "GROUP_REVIEW_VERDICT: approve\n{\"verdict\":\"approve\",\"findings\":[]}"
                .to_string(),
            role_run_id: None,
        }),
    ]);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    let shards = orchestrator
        .execute_shards(&snapshot)
        .await
        .expect("incident output must not be shard_output_invalid");
    assert_eq!(shards.len(), 1);
    assert_eq!(shards[0].findings.len(), 4);
    assert!(shards[0].findings.iter().all(|finding| {
        finding.defect_class == PlanDefectClass::VerificationIncomplete
            && finding.repair_target.is_none()
            && finding.recommended_route == PlanDefectRoute::VerificationRetry
            && finding.confidence == Some(crate::product::plan_repair::PlanDefectConfidence::High)
    }));

    let binding = verification_binding();
    let reduction = orchestrator
        .execute_reduction(&snapshot, &shards, std::slice::from_ref(&binding))
        .await
        .expect("normalized verification finding must pass reduction validation");
    assert_eq!(reduction.verdict, ReviewVerdict::RequestChanges);
    assert!(
        reduction
            .findings
            .iter()
            .all(|finding| finding.recommended_route == PlanDefectRoute::VerificationRetry)
    );

    let reviews = store
        .list_internal_pr_reviews("project_0001", "issue_0001", &attempt_id)
        .expect("persisted internal review");
    assert_eq!(reviews.len(), 1);
    assert_eq!(
        internal_review_flow_decision_with_bindings(&reviews[0], &[binding]),
        CodeReviewFlowDecision::RetryVerification
    );
}

#[tokio::test]
async fn unknown_metadata_remains_shard_output_invalid() {
    let (_root, store, attempt_id) = setup();
    let snapshot = snapshot(attempt_id.clone());
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");
    let executor = FakeGroupReviewExecutor::new(vec![
        Ok(GroupReviewExecutionResult {
            full_output: "GROUP_REVIEW_VERDICT: request_changes\n{\"verdict\":\"request_changes\",\"findings\":[{\"message\":\"unknown metadata\",\"defect_class\":\"unrecognized_future_class\"}]}".to_string(),
            role_run_id: None,
        }),
        Err(GroupReviewExecutionError::Internal(
            "repair must not normalize unknown metadata".to_string(),
        )),
    ]);

    let error = GroupReviewOrchestrator::new(&executor, &store)
        .execute_shards(&snapshot)
        .await
        .expect_err("unknown metadata must remain fail-closed");
    assert!(matches!(
        error,
        GroupReviewOrchestrationError::ShardOutputInvalid { .. }
    ));
    let reports = store
        .list_group_review_shard_reports(&attempt_id)
        .expect("shard reports");
    assert_eq!(
        reports[0].run_failure_code.as_deref(),
        Some("shard_output_invalid")
    );
}
