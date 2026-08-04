use crate::product::coding_models::{
    CodingUnitRun, CodingUnitRunStatus, CompactFindingDigest, PushStatus, RemoteKind,
    ReviewRequest, ReviewRequestKind, ReviewVerdict, UnitReviewConclusionSnapshot,
};
use crate::product::coding_workspace_engine::group_review_material::{
    GroupReviewMaterialSnapshotDraft, ShardPromptMeasurer, compile_group_review_material,
};
use crate::product::coding_workspace_engine::group_review_types::{
    CompletionDiff, DiffFileStat, GroupGitFacts, GroupShardSpec,
};
use crate::product::coding_workspace_engine::plan_defect_routing::{
    AuthoritativeGroupReviewerBinding, GroupReviewerProjectionBinding,
};
use crate::product::work_item_contract::{
    BlockerRoute, ContractCompatibilityPolicy, PromisedOutputContract, RequiredInputContract,
    VerificationCheck, WorkItemWritePolicy,
};
use crate::product::work_item_projection::{ReviewerRequirementCheck, ReviewerWorkItemProjection};

struct FixedMeasurer;
impl ShardPromptMeasurer for FixedMeasurer {
    fn measure_shard(&self, _: &GroupReviewMaterialSnapshotDraft, _: &GroupShardSpec) -> usize {
        0
    }
}

fn binding(
    index: u32,
    id: &str,
    input: Vec<RequiredInputContract>,
    output: Vec<PromisedOutputContract>,
) -> AuthoritativeGroupReviewerBinding {
    AuthoritativeGroupReviewerBinding {
        order_index: index,
        run: CodingUnitRun {
            id: format!("run_{id}"),
            unit_id: id.to_string(),
            execution_no: 1,
            work_item_revision_id: format!("revision_{id}"),
            resolved_handoff_revision_ids: Vec::new(),
            canonical_contract_hash: format!("contract_hash_{id}"),
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
            start_commit: Some("base".to_string()),
            completion_commit: Some(format!("commit_{id}")),
            created_at: "ignored".to_string(),
            updated_at: "ignored".to_string(),
        },
        projection_binding: GroupReviewerProjectionBinding {
            logical_work_item_id: format!("work_{id}"),
            projection: ReviewerWorkItemProjection {
                work_item_revision_id: format!("revision_{id}"),
                criterion_refs: vec![format!("criterion_{id}")],
                requirement_matrix: vec![ReviewerRequirementCheck {
                    criterion_id: format!("criterion_{id}"),
                    requirement_refs: vec![format!("req_{id}")],
                    required_evidence: Vec::new(),
                    failure_route: BlockerRoute::CoderRework,
                }],
                scope_policy: WorkItemWritePolicy {
                    exclusive_scopes: vec![format!("src/{id}/")],
                    forbidden_scopes: Vec::new(),
                },
                input_contract_checks: input,
                output_contract_checks: output,
                verification_evidence_rules: vec![VerificationCheck {
                    check_id: "check".to_string(),
                    command: Some("cargo test".to_string()),
                    manual_instruction: None,
                    required: true,
                    non_zero_test_execution_required: true,
                }],
                blocker_routing: Vec::new(),
            },
        },
    }
}

fn snapshot(binding: &AuthoritativeGroupReviewerBinding) -> UnitReviewConclusionSnapshot {
    UnitReviewConclusionSnapshot {
        attempt_id: "attempt".to_string(),
        unit_id: binding.run.unit_id.clone(),
        unit_run_id: binding.run.id.clone(),
        logical_work_item_id: binding.projection_binding.logical_work_item_id.clone(),
        work_item_revision_id: binding.run.work_item_revision_id.clone(),
        code_review_report_id: format!("report_{}", binding.run.id),
        verdict: ReviewVerdict::Approve,
        finding_digest: vec![CompactFindingDigest {
            defect_class: None,
            reason_code: None,
            severity: "info".to_string(),
            message_digest: "digest".to_string(),
        }],
        evidence_refs: vec!["evidence".to_string()],
        diff_refs: Vec::new(),
        raw_report_hash: "raw".to_string(),
    }
}

fn request() -> ReviewRequest {
    ReviewRequest {
        id: "request".to_string(),
        attempt_id: "attempt".to_string(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::Unknown,
        remote: String::new(),
        base_branch: "main".to_string(),
        branch_name: "branch".to_string(),
        commit_sha: "final".to_string(),
        push_status: PushStatus::NotPushed,
        external_url: None,
        manual_instructions: Vec::new(),
        push_error: None,
        created_at: "ignored".to_string(),
        updated_at: "ignored".to_string(),
    }
}

fn facts(bindings: &[AuthoritativeGroupReviewerBinding]) -> GroupGitFacts {
    GroupGitFacts { diff_stat: String::new(), completion_diffs: bindings.iter().map(|binding| CompletionDiff { unit_run_id: binding.run.id.clone(), base_commit: "base".to_string(), completion_commit: binding.run.completion_commit.clone().expect("commit"), patch: format!("diff --git a/src/{0}/file.rs b/src/{0}/file.rs\n@@ -0,0 +1 @@\n+fn {0}() {{}}\n", binding.run.unit_id), file_stats: vec![DiffFileStat { path: format!("src/{}/file.rs", binding.run.unit_id), insertions: 1, deletions: 0 }], hunks: Vec::new() }).collect(), final_diff: String::new() }
}

#[test]
fn group_review_material_content_hash_is_stable_across_input_order() {
    let first = binding(2, "b", Vec::new(), Vec::new());
    let second = binding(1, "a", Vec::new(), Vec::new());
    let forward = vec![first.clone(), second.clone()];
    let reverse = vec![second.clone(), first.clone()];
    let forward_snapshots = forward.iter().map(snapshot).collect::<Vec<_>>();
    let reverse_snapshots = reverse.iter().map(snapshot).collect::<Vec<_>>();
    let forward_result = compile_group_review_material(
        &forward,
        &forward_snapshots,
        &request(),
        &facts(&forward),
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    let reverse_result = compile_group_review_material(
        &reverse,
        &reverse_snapshots,
        &request(),
        &facts(&reverse),
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    assert_eq!(forward_result.content_hash, reverse_result.content_hash);
}

#[test]
fn material_partitions_twenty_units_in_shards_of_at_most_four() {
    let bindings = (0..20)
        .map(|index| binding(index, &format!("{index:02}"), Vec::new(), Vec::new()))
        .collect::<Vec<_>>();
    let snapshots = bindings.iter().map(snapshot).collect::<Vec<_>>();
    let result = compile_group_review_material(
        &bindings,
        &snapshots,
        &request(),
        &facts(&bindings),
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    assert!(result.partition_result.shards.len() >= 5);
    assert!(
        result
            .partition_result
            .shards
            .iter()
            .all(|shard| shard.ordered_unit_run_ids.len() <= 4)
    );
}

#[test]
fn material_keeps_handoff_dependency_in_same_shard() {
    let producer = binding(
        1,
        "producer",
        Vec::new(),
        vec![PromisedOutputContract {
            contract_id: "api".to_string(),
            capabilities: vec!["read".to_string()],
        }],
    );
    let consumer = binding(
        2,
        "consumer",
        vec![RequiredInputContract {
            contract_id: "api".to_string(),
            provider_logical_work_item_id: "work_producer".to_string(),
            required_capabilities: vec!["read".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
        Vec::new(),
    );
    let bindings = vec![producer, consumer];
    let snapshots = bindings.iter().map(snapshot).collect::<Vec<_>>();
    let result = compile_group_review_material(
        &bindings,
        &snapshots,
        &request(),
        &facts(&bindings),
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    assert!(
        result
            .partition_result
            .shards
            .iter()
            .any(|shard| shard.ordered_unit_run_ids.len() == 2)
    );
}

#[test]
fn material_indexes_hunks_and_marks_forbidden_scope() {
    let mut unit = binding(1, "one", Vec::new(), Vec::new());
    unit.projection_binding
        .projection
        .scope_policy
        .forbidden_scopes = vec!["src/one/".to_string()];
    let snapshots = vec![snapshot(&unit)];
    let result = compile_group_review_material(
        &[unit.clone()],
        &snapshots,
        &request(),
        &facts(&[unit]),
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    assert_eq!(result.diff_index.files.len(), 1);
    assert!(result.diff_index.files[0].forbidden_scope_hit);
    assert_eq!(
        result.diff_index.files[0].owner_unit_run_ids,
        vec!["run_one"]
    );
    assert!(!result.diff_index.hunks.is_empty());
}
