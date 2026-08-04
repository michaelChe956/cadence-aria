use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use crate::product::coding_models::{
    CodingExecutionStage, FindingSeverity, GroupReviewShardReport, ReviewFinding, ReviewVerdict,
};
use crate::product::coding_workspace_engine::group_review_orchestrator::{
    FakeGroupReviewExecutor, GroupReviewExecutionResult, GroupReviewOrchestrationError,
    GroupReviewOrchestrator, finding_fingerprint, merge_findings, reduce_verdict,
};
use crate::product::coding_workspace_engine::group_review_types::{
    GroupDiffIndex, GroupPartitionResult, GroupReviewGraph, GroupReviewMaterialSnapshot,
    GroupShardSpec, ReductionDiffSelection,
};
use crate::product::models::{PlanDefectClass, PlanDefectEvidence, PlanDefectRoute, ProviderName};
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

fn snapshot(attempt_id: String, shard_ids: &[&str]) -> GroupReviewMaterialSnapshot {
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
            shards: shard_ids
                .iter()
                .map(|id| GroupShardSpec {
                    shard_id: (*id).to_string(),
                    ordered_unit_run_ids: Vec::new(),
                    partition_rationale: Vec::new(),
                })
                .collect(),
            cross_shard_edges: Vec::new(),
        },
        content_hash: "snapshot_hash".to_string(),
    }
}

fn finding(message: &str, severity: FindingSeverity) -> ReviewFinding {
    ReviewFinding {
        severity,
        file_path: Some("src/lib.rs".to_string()),
        line: None,
        message: message.to_string(),
        required_action: None,
        source_stage: CodingExecutionStage::InternalPrReview,
        evidence: vec!["evidence_a".to_string()],
        plan_defect_evidence: Vec::new(),
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
        defect_class: PlanDefectClass::ImplementationDefect,
        reason_code: Some("reason".to_string()),
        contract_refs: vec!["contract_a".to_string()],
        capability_refs: vec!["capability_a".to_string()],
        repair_target: None,
        recommended_route: PlanDefectRoute::CoderRework,
        confidence: None,
    }
}

fn shard(
    attempt_id: &str,
    shard_id: &str,
    verdict: ReviewVerdict,
    findings: Vec<ReviewFinding>,
) -> GroupReviewShardReport {
    GroupReviewShardReport {
        id: format!("group_review_shard_{shard_id}"),
        attempt_id: attempt_id.to_string(),
        snapshot_hash: "snapshot_hash".to_string(),
        shard_id: shard_id.to_string(),
        ordered_unit_run_ids: Vec::new(),
        partition_rationale: Vec::new(),
        verdict,
        findings,
        unresolved_obligations: Vec::new(),
        selected_diff_refs: Vec::new(),
        raw_provider_output_refs: Vec::new(),
        role_run_ids: Vec::new(),
        run_failure_code: None,
    }
}

#[test]
fn fingerprints_and_merges_equivalent_findings() {
    let mut second = finding("same", FindingSeverity::Error);
    second.evidence = vec!["evidence_b".to_string()];
    let reports = vec![shard(
        "attempt",
        "a",
        ReviewVerdict::Approve,
        vec![finding("same", FindingSeverity::Warning)],
    )];
    let merged = merge_findings(&reports, vec![second]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].severity, FindingSeverity::Error);
    assert_eq!(
        merged[0].evidence,
        vec!["evidence_a".to_string(), "evidence_b".to_string()]
    );
}

#[test]
fn fingerprint_changes_for_structural_identity_fields() {
    let base = finding("base", FindingSeverity::Warning);
    let mut changed = base.clone();
    changed.reason_code = Some("other".to_string());
    assert_ne!(finding_fingerprint(&base), finding_fingerprint(&changed));
    changed = base.clone();
    changed.defect_class = PlanDefectClass::DesignAmendmentRequired;
    assert_ne!(finding_fingerprint(&base), finding_fingerprint(&changed));
    changed = base.clone();
    changed.plan_defect_evidence = vec![PlanDefectEvidence {
        kind: "hunk".to_string(),
        source_ref: "different_hunk_hash".to_string(),
        message: String::new(),
    }];
    assert_ne!(finding_fingerprint(&base), finding_fingerprint(&changed));
    changed = base.clone();
    changed.contract_refs = vec!["other".to_string()];
    assert_ne!(finding_fingerprint(&base), finding_fingerprint(&changed));
    changed = base.clone();
    changed.capability_refs = vec!["other".to_string()];
    assert_ne!(finding_fingerprint(&base), finding_fingerprint(&changed));
    changed = base.clone();
    changed.file_path = Some("src/other.rs".to_string());
    assert_ne!(finding_fingerprint(&base), finding_fingerprint(&changed));
}

#[test]
fn reduce_verdict_prioritizes_blocked_then_request_changes() {
    assert_eq!(
        reduce_verdict(vec![ReviewVerdict::Approve]),
        ReviewVerdict::Approve
    );
    assert_eq!(
        reduce_verdict(vec![ReviewVerdict::Approve, ReviewVerdict::RequestChanges]),
        ReviewVerdict::RequestChanges
    );
    assert_eq!(
        reduce_verdict(vec![ReviewVerdict::RequestChanges, ReviewVerdict::Blocked]),
        ReviewVerdict::Blocked
    );
}

#[tokio::test]
async fn reduction_requires_all_shards_before_provider_execution() {
    let (_root, store, attempt_id) = setup();
    let snapshot = snapshot(attempt_id.clone(), &["a", "b"]);
    let executor = FakeGroupReviewExecutor::new(Vec::new());
    let error = GroupReviewOrchestrator::new(&executor, &store)
        .execute_reduction(
            &snapshot,
            &[shard(&attempt_id, "a", ReviewVerdict::Approve, Vec::new())],
            &[],
        )
        .await
        .expect_err("missing shard");
    assert!(matches!(
        error,
        GroupReviewOrchestrationError::ReductionNotReady
    ));
    assert!(executor.prompts().is_empty());
}

#[tokio::test]
async fn reduction_executes_for_non_approve_shards_and_persists_internal_review() {
    let (_root, store, attempt_id) = setup();
    let snapshot = snapshot(attempt_id.clone(), &["a"]);
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("active");
    let executor = FakeGroupReviewExecutor::new(vec![Ok(GroupReviewExecutionResult {
        full_output: "GROUP_REVIEW_VERDICT\n{\"verdict\":\"approve\",\"summary\":\"done\"}"
            .to_string(),
        provider_session_id: None,
    })]);
    let reduction = GroupReviewOrchestrator::new(&executor, &store)
        .execute_reduction(
            &snapshot,
            &[shard(
                &attempt_id,
                "a",
                ReviewVerdict::RequestChanges,
                Vec::new(),
            )],
            &[],
        )
        .await
        .expect("reduction");
    assert_eq!(reduction.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(
        store
            .list_internal_pr_reviews("project_0001", "issue_0001", &attempt_id)
            .expect("reviews")
            .len(),
        1
    );
}

#[tokio::test]
async fn reduction_rejects_invalid_authoritative_finding_target() {
    let (_root, store, attempt_id) = setup();
    let snapshot = snapshot(attempt_id.clone(), &["a"]);
    let output = "GROUP_REVIEW_VERDICT\n{\"verdict\":\"request_changes\",\"findings\":[{\"message\":\"invalid plan target\",\"defect_class\":\"design_amendment_required\",\"reason_code\":\"unknown\"}]}";
    let executor = FakeGroupReviewExecutor::new(vec![Ok(GroupReviewExecutionResult {
        full_output: output.to_string(),
        provider_session_id: None,
    })]);
    let error = GroupReviewOrchestrator::new(&executor, &store)
        .execute_reduction(
            &snapshot,
            &[shard(&attempt_id, "a", ReviewVerdict::Approve, Vec::new())],
            &[],
        )
        .await
        .expect_err("invalid plan finding must fail closed");
    assert!(matches!(
        error,
        GroupReviewOrchestrationError::ReductionOutputInvalid { .. }
    ));
}

#[tokio::test]
async fn reduction_rejects_more_than_sixteen_findings() {
    let (_root, store, attempt_id) = setup();
    let snapshot = snapshot(attempt_id.clone(), &["a"]);
    let findings = (0..17)
        .map(|index| serde_json::json!({"message": format!("finding {index}")}))
        .collect::<Vec<_>>();
    let output = format!(
        "GROUP_REVIEW_VERDICT\n{}",
        serde_json::json!({"verdict":"approve", "findings": findings})
    );
    let executor = FakeGroupReviewExecutor::new(vec![Ok(GroupReviewExecutionResult {
        full_output: output,
        provider_session_id: None,
    })]);
    let error = GroupReviewOrchestrator::new(&executor, &store)
        .execute_reduction(
            &snapshot,
            &[shard(&attempt_id, "a", ReviewVerdict::Approve, Vec::new())],
            &[],
        )
        .await
        .expect_err("limit");
    assert!(matches!(
        error,
        GroupReviewOrchestrationError::ReductionOutputInvalid { .. }
    ));
}
