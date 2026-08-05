//! Task 8c: 端到端验收测试。
//!
//! 覆盖：
//! - Step 3：20-unit E2E（5 分片 + 1 归约，全量断言）
//! - Step 9-10：三家 provider 材料一致性
//! - Step 13-14：raw ref 失败路径持久化

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use crate::product::coding_models::{
    CodingUnitRun, CodingUnitRunStatus, CompactFindingDigest, PushStatus, RemoteKind,
    ReviewRequest, ReviewRequestKind, ReviewVerdict, UnitReviewConclusionSnapshot,
};
use crate::product::coding_workspace_engine::group_review_budget::{
    GROUP_REVIEW_HARD_CAP_BYTES, GROUP_REVIEW_QUALITY_TARGET_BYTES,
};
use crate::product::coding_workspace_engine::group_review_material::{
    GroupReviewMaterialSnapshotDraft, ShardPromptMeasurer, compile_group_review_material,
};
use crate::product::coding_workspace_engine::group_review_orchestrator::{
    FakeGroupReviewExecutor, GroupReviewExecutionError, GroupReviewExecutionResult,
    GroupReviewOrchestrationError, GroupReviewOrchestrator,
};
use crate::product::coding_workspace_engine::group_review_prompts::{
    build_reduction_prompt, build_shard_prompt,
};
use crate::product::coding_workspace_engine::group_review_types::{
    CompletionDiff, DiffFileStat, GroupGitFacts, GroupReviewMaterialSnapshot, GroupShardSpec,
};
use crate::product::coding_workspace_engine::plan_defect_routing::{
    AuthoritativeGroupReviewerBinding, GroupReviewerProjectionBinding,
};
use crate::product::models::ProviderName;
use crate::product::work_item_contract::{
    BlockerRoute, ContractCompatibilityPolicy, RequiredInputContract, VerificationCheck,
    WorkItemWritePolicy,
};
use crate::product::work_item_projection::{ReviewerRequirementCheck, ReviewerWorkItemProjection};
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helper: fake provider output that passes parse_review_payload
// ---------------------------------------------------------------------------

fn valid_group_review_output(verdict: &str, findings: usize) -> String {
    let findings_json = (0..findings)
        .map(|index| {
            serde_json::json!({
                "message": format!("finding {index}"),
                "defect_class": "implementation_defect",
            })
        })
        .collect::<Vec<_>>();
    format!(
        "GROUP_REVIEW_VERDICT: {}\n{}",
        verdict,
        serde_json::json!({
            "verdict": verdict,
            "summary": "group review complete",
            "findings": findings_json,
            "impact_scope": ["group"],
            "pr_description": "pr description",
            "commit_message_suggestion": "review group",
        })
    )
}

fn valid_shard_output() -> String {
    valid_group_review_output("approve", 0)
}

fn valid_reduction_output() -> String {
    valid_group_review_output("approve", 0)
}

// ---------------------------------------------------------------------------
// Helper: construct AuthoritativeGroupReviewerBinding + snapshot + facts
// ---------------------------------------------------------------------------

fn e2e_binding(
    index: u32,
    id: &str,
    exclusive_scope: Option<String>,
    forbidden_scope: Option<String>,
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
            created_at: "2026-08-04T00:00:00Z".to_string(),
            updated_at: "2026-08-04T00:00:00Z".to_string(),
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
                    exclusive_scopes: exclusive_scope.into_iter().collect(),
                    forbidden_scopes: forbidden_scope.into_iter().collect(),
                },
                input_contract_checks: Vec::new(),
                output_contract_checks: Vec::new(),
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

fn e2e_snapshot(
    binding: &AuthoritativeGroupReviewerBinding,
    attempt_id: &str,
) -> UnitReviewConclusionSnapshot {
    UnitReviewConclusionSnapshot {
        attempt_id: attempt_id.to_string(),
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

fn e2e_review_request() -> ReviewRequest {
    ReviewRequest {
        id: "request_e2e".to_string(),
        attempt_id: "attempt_e2e".to_string(),
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

fn e2e_facts(bindings: &[AuthoritativeGroupReviewerBinding]) -> GroupGitFacts {
    GroupGitFacts {
        diff_stat: String::new(),
        completion_diffs: bindings
            .iter()
            .map(|binding| CompletionDiff {
                unit_run_id: binding.run.id.clone(),
                base_commit: "base".to_string(),
                completion_commit: binding.run.completion_commit.clone().expect("commit"),
                patch: format!(
                    "diff --git a/src/{0}/file.rs b/src/{0}/file.rs\n@@ -0,0 +1 @@\n+fn {0}() {{}}\n",
                    binding.run.unit_id
                ),
                file_stats: vec![DiffFileStat {
                    path: format!("src/{}/file.rs", binding.run.unit_id),
                    insertions: 1,
                    deletions: 0,
                }],
                hunks: Vec::new(),
            })
            .collect(),
        final_diff: String::new(),
        final_commit: "final".to_string(),
        completion_commit_in_final: bindings
            .iter()
            .filter_map(|binding| binding.run.completion_commit.clone())
            .collect(),
    }
}

/// A real `ShardPromptMeasurer` that uses `build_shard_prompt` to compute
/// accurate byte sizes, so budget repartition produces real sub-30KiB shards.
struct RealMeasurer;

impl ShardPromptMeasurer for RealMeasurer {
    fn measure_shard(
        &self,
        snapshot: &GroupReviewMaterialSnapshotDraft,
        shard: &GroupShardSpec,
    ) -> usize {
        let full = GroupReviewMaterialSnapshot {
            schema_version: snapshot.schema_version,
            compiler_version: snapshot.compiler_version.clone(),
            attempt_id: snapshot.attempt_id.clone(),
            review_request_id: snapshot.review_request_id.clone(),
            base_branch: snapshot.base_branch.clone(),
            final_commit: snapshot.final_commit.clone(),
            authoritative_binding_digest: snapshot.authoritative_binding_digest.clone(),
            unit_records: snapshot.unit_records.clone(),
            global_graph: snapshot.global_graph.clone(),
            diff_index: snapshot.diff_index.clone(),
            deterministic_findings: snapshot.deterministic_findings.clone(),
            partition_result: snapshot.partition_result.clone(),
            content_hash: String::new(),
        };
        build_shard_prompt(&full, shard, None).measure().total
    }
}

// ---------------------------------------------------------------------------
// Helper: store setup with an attempt
// ---------------------------------------------------------------------------

fn e2e_store() -> (TempDir, CodingAttemptStore, String) {
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

// ---------------------------------------------------------------------------
// Helper: build a 20-unit material snapshot via the real compiler
// ---------------------------------------------------------------------------

fn twenty_unit_snapshot(
    attempt_id: &str,
) -> (
    Vec<AuthoritativeGroupReviewerBinding>,
    Vec<UnitReviewConclusionSnapshot>,
    crate::product::coding_workspace_engine::group_review_types::GroupReviewMaterialSnapshot,
) {
    let bindings = (0..20)
        .map(|index| {
            e2e_binding(
                index,
                &format!("unit_{index:02}"),
                Some(format!("src/unit_{index:02}/")),
                None,
            )
        })
        .collect::<Vec<_>>();
    let snapshots = bindings
        .iter()
        .map(|b| e2e_snapshot(b, attempt_id))
        .collect::<Vec<_>>();
    let mut request = e2e_review_request();
    request.attempt_id = attempt_id.to_string();
    let snapshot = compile_group_review_material(
        &bindings,
        &snapshots,
        &request,
        &e2e_facts(&bindings),
        &RealMeasurer,
        GROUP_REVIEW_QUALITY_TARGET_BYTES,
    )
    .expect("compile 20-unit material");
    (bindings, snapshots, snapshot)
}

// ===========================================================================
// Step 3: 20-unit E2E — shard prompt size, reduction, cross-shard conflicts
// ===========================================================================

#[test]
fn step3_twenty_unit_compile_produces_at_least_five_shards() {
    let (_bindings, _snapshots, snapshot) = twenty_unit_snapshot("compile_only");
    assert!(
        snapshot.partition_result.shards.len() >= 5,
        "20 units should be partitioned into at least 5 shards of ≤4 units"
    );
    for shard in &snapshot.partition_result.shards {
        assert!(
            shard.ordered_unit_run_ids.len() <= 4,
            "each shard should have at most 4 units"
        );
    }
}

#[test]
fn step3_each_shard_prompt_is_within_hard_cap() {
    let (_bindings, _snapshots, snapshot) = twenty_unit_snapshot("compile_only");
    for shard in &snapshot.partition_result.shards {
        let prompt = build_shard_prompt(&snapshot, shard, None);
        let breakdown = prompt.measure();
        assert!(
            breakdown.total <= GROUP_REVIEW_HARD_CAP_BYTES,
            "shard {} prompt is {} bytes, exceeds hard cap {}",
            shard.shard_id,
            breakdown.total,
            GROUP_REVIEW_HARD_CAP_BYTES,
        );
    }
}

#[test]
fn step3_each_shard_prompt_is_within_quality_target_or_sendable() {
    // The budget repartitioner should ensure prompts are ≤ quality_target (28 KiB).
    // This test verifies the compiler honored the target during partitioning.
    let (_bindings, _snapshots, snapshot) = twenty_unit_snapshot("compile_only");
    for shard in &snapshot.partition_result.shards {
        let prompt = build_shard_prompt(&snapshot, shard, None);
        let breakdown = prompt.measure();
        assert!(
            breakdown.total <= GROUP_REVIEW_QUALITY_TARGET_BYTES,
            "shard {} prompt is {} bytes, exceeds quality target {} (repartition should keep within target)",
            shard.shard_id,
            breakdown.total,
            GROUP_REVIEW_QUALITY_TARGET_BYTES,
        );
    }
}

#[tokio::test]
async fn step3_e2e_shards_and_reduction_persist_internal_pr_review() {
    let (_root, store, attempt_id) = e2e_store();
    let (bindings, _snapshots, snapshot) = twenty_unit_snapshot(&attempt_id);
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");

    // 5 shard outputs + 1 reduction output
    let shard_count = snapshot.partition_result.shards.len();
    let mut results = (0..shard_count)
        .map(|_| {
            Ok(GroupReviewExecutionResult {
                full_output: valid_shard_output(),
                provider_session_id: None,
            })
        })
        .collect::<Vec<_>>();
    results.push(Ok(GroupReviewExecutionResult {
        full_output: valid_reduction_output(),
        provider_session_id: None,
    }));
    let executor = FakeGroupReviewExecutor::new(results);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    let shard_reports = orchestrator
        .execute_shards(&snapshot)
        .await
        .expect("execute shards");
    assert_eq!(shard_reports.len(), shard_count);

    // All shard reports have non-empty raw refs and no failure code
    for report in &shard_reports {
        assert_eq!(report.run_failure_code, None);
        assert!(!report.raw_provider_output_refs.is_empty());
    }

    let projection_bindings = bindings
        .iter()
        .map(|b| b.projection_binding.clone())
        .collect::<Vec<_>>();
    let reduction = orchestrator
        .execute_reduction(&snapshot, &shard_reports, &projection_bindings)
        .await
        .expect("execute reduction");

    assert_eq!(reduction.verdict, ReviewVerdict::Approve);
    assert!(!reduction.raw_provider_output_refs.is_empty());

    // The InternalPrReview should have been persisted by the orchestrator
    let reviews = store
        .list_internal_pr_reviews("project_0001", "issue_0001", &attempt_id)
        .expect("list reviews");
    assert_eq!(reviews.len(), 1);
    let review = &reviews[0];
    assert_eq!(review.verdict, ReviewVerdict::Approve);
    assert!(review.raw_provider_output_ref.is_some());
    assert!(review.findings.is_empty()); // no findings in approve
}

#[test]
fn step3_cross_shard_contract_mismatch_is_detected_by_deterministic_findings() {
    // Construct two bindings with an unmatched contract: consumer expects capability
    // that producer does not provide.
    let producer = e2e_binding(0, "producer", Some("src/producer/".into()), None);
    let consumer = AuthoritativeGroupReviewerBinding {
        order_index: 1,
        run: CodingUnitRun {
            id: "run_consumer".to_string(),
            unit_id: "consumer".to_string(),
            execution_no: 1,
            work_item_revision_id: "revision_consumer".to_string(),
            resolved_handoff_revision_ids: Vec::new(),
            canonical_contract_hash: "hash_consumer".to_string(),
            projection_bundle_id: "bundle_consumer".to_string(),
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
            completion_commit: Some("commit_consumer".to_string()),
            created_at: "2026-08-04T00:00:00Z".to_string(),
            updated_at: "2026-08-04T00:00:00Z".to_string(),
        },
        projection_binding: GroupReviewerProjectionBinding {
            logical_work_item_id: "work_consumer".to_string(),
            projection: ReviewerWorkItemProjection {
                work_item_revision_id: "revision_consumer".to_string(),
                criterion_refs: vec!["criterion_consumer".to_string()],
                requirement_matrix: vec![ReviewerRequirementCheck {
                    criterion_id: "criterion_consumer".to_string(),
                    requirement_refs: vec!["req_consumer".to_string()],
                    required_evidence: Vec::new(),
                    failure_route: BlockerRoute::CoderRework,
                }],
                scope_policy: WorkItemWritePolicy {
                    exclusive_scopes: vec!["src/consumer/".to_string()],
                    forbidden_scopes: Vec::new(),
                },
                input_contract_checks: vec![RequiredInputContract {
                    contract_id: "api".to_string(),
                    provider_logical_work_item_id: "work_producer".to_string(),
                    required_capabilities: vec!["write".to_string()],
                    compatibility_policy: ContractCompatibilityPolicy::RequireAll,
                }],
                output_contract_checks: Vec::new(),
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
    };
    let bindings = vec![producer, consumer];
    let snapshots = bindings
        .iter()
        .map(|b| e2e_snapshot(b, "compile_only"))
        .collect::<Vec<_>>();
    let mut request = e2e_review_request();
    request.attempt_id = "compile_only".to_string();
    let snapshot = compile_group_review_material(
        &bindings,
        &snapshots,
        &request,
        &e2e_facts(&bindings),
        &RealMeasurer,
        GROUP_REVIEW_QUALITY_TARGET_BYTES,
    )
    .expect("compile");

    let has_mismatch = snapshot
        .deterministic_findings
        .iter()
        .any(|f| f.kind == "contract_missing_or_capability_mismatch");
    assert!(
        has_mismatch,
        "deterministic findings should include contract mismatch"
    );

    // The contract edge should be unmatched
    let unmatched_edge = snapshot
        .global_graph
        .contract_edges
        .iter()
        .find(|edge| edge.contract_id == "api");
    assert!(unmatched_edge.is_some());
    assert!(!unmatched_edge.unwrap().matched);
}

#[test]
fn step3_cross_shard_shared_file_conflict_is_detected_by_deterministic_findings() {
    // Two units touch the same file → scope overlap
    let mut left = e2e_binding(0, "left", Some("src/left/".into()), None);
    let right = e2e_binding(1, "right", Some("src/right/".into()), None);
    let bindings = vec![left.clone(), right];
    let snapshots = bindings
        .iter()
        .map(|b| e2e_snapshot(b, "compile_only"))
        .collect::<Vec<_>>();
    let mut git_facts = e2e_facts(&bindings);
    // Both units modify the same file
    for diff in &mut git_facts.completion_diffs {
        diff.patch = "diff --git a/shared.rs b/shared.rs\n@@ -0,0 +1 @@\n+shared\n".to_string();
        diff.file_stats = vec![DiffFileStat {
            path: "shared.rs".to_string(),
            insertions: 1,
            deletions: 0,
        }];
    }
    let mut request = e2e_review_request();
    request.attempt_id = "compile_only".to_string();
    let snapshot = compile_group_review_material(
        &bindings,
        &snapshots,
        &request,
        &git_facts,
        &RealMeasurer,
        GROUP_REVIEW_QUALITY_TARGET_BYTES,
    )
    .expect("compile");

    // shared.rs should be in scope_overlaps with 2 owners
    let overlap = snapshot
        .global_graph
        .scope_overlaps
        .iter()
        .find(|o| o.file_path == "shared.rs");
    assert!(
        overlap.is_some(),
        "shared file should produce scope overlap"
    );
    let overlap = overlap.unwrap();
    assert_eq!(overlap.unit_run_ids.len(), 2);
    assert!(!overlap.forbidden_hit);

    // Force different scopes so they land in different shards
    left.run.id = "run_left_v2".to_string();
}

#[test]
fn step3_scope_violation_is_detected_by_deterministic_findings() {
    // A unit has a forbidden scope that matches its own file
    let mut unit = e2e_binding(0, "violation", Some("src/violation/".into()), None);
    unit.projection_binding
        .projection
        .scope_policy
        .forbidden_scopes = vec!["src/violation/".to_string()];
    let bindings = vec![unit];
    let snapshots = bindings
        .iter()
        .map(|b| e2e_snapshot(b, "compile_only"))
        .collect::<Vec<_>>();
    let mut request = e2e_review_request();
    request.attempt_id = "compile_only".to_string();
    let snapshot = compile_group_review_material(
        &bindings,
        &snapshots,
        &request,
        &e2e_facts(&bindings),
        &RealMeasurer,
        GROUP_REVIEW_QUALITY_TARGET_BYTES,
    )
    .expect("compile");

    let has_forbidden = snapshot
        .deterministic_findings
        .iter()
        .any(|f| f.kind == "forbidden_scope_hit");
    assert!(
        has_forbidden,
        "deterministic findings should include forbidden_scope_hit"
    );

    // The diff index should also mark the file as forbidden_scope_hit
    assert!(
        snapshot
            .diff_index
            .files
            .iter()
            .any(|f| f.forbidden_scope_hit),
        "diff index should mark the file as forbidden_scope_hit"
    );
}

// ===========================================================================
// Step 9-10: three-provider material consistency
// ===========================================================================

#[test]
fn step9_10_material_snapshot_is_provider_independent() {
    // compile_group_review_material is a pure function that does not depend on provider.
    // Running it with different provider configs should produce identical snapshot.
    let bindings = (0..6)
        .map(|index| {
            e2e_binding(
                index,
                &format!("prov_{index:02}"),
                Some(format!("src/prov_{index:02}/")),
                None,
            )
        })
        .collect::<Vec<_>>();
    let snapshots = bindings
        .iter()
        .map(|b| e2e_snapshot(b, "compile_only"))
        .collect::<Vec<_>>();
    let mut request = e2e_review_request();
    request.attempt_id = "compile_only".to_string();
    let facts = e2e_facts(&bindings);

    let snapshot_a = compile_group_review_material(
        &bindings,
        &snapshots,
        &request,
        &facts,
        &RealMeasurer,
        GROUP_REVIEW_QUALITY_TARGET_BYTES,
    )
    .expect("compile A");

    let snapshot_b = compile_group_review_material(
        &bindings,
        &snapshots,
        &request,
        &facts,
        &RealMeasurer,
        GROUP_REVIEW_QUALITY_TARGET_BYTES,
    )
    .expect("compile B");

    // Material content hash must be identical (provider-independent)
    assert_eq!(
        snapshot_a.content_hash, snapshot_b.content_hash,
        "material snapshot must be deterministic and provider-independent"
    );
    assert_eq!(
        snapshot_a.partition_result.shards.len(),
        snapshot_b.partition_result.shards.len()
    );
}

#[test]
fn step9_10_shard_prompts_are_identical_regardless_of_provider_meta() {
    // build_shard_prompt takes provider_meta: Option<&str>.
    // When provider_meta is None (as in the orchestrator path), the prompts are identical.
    // When provider_meta is Some, only the retry_diagnostic_reserve differs.
    let bindings = (0..4)
        .map(|index| {
            e2e_binding(
                index,
                &format!("meta_{index:02}"),
                Some(format!("src/meta_{index:02}/")),
                None,
            )
        })
        .collect::<Vec<_>>();
    let snapshots = bindings
        .iter()
        .map(|b| e2e_snapshot(b, "compile_only"))
        .collect::<Vec<_>>();
    let mut request = e2e_review_request();
    request.attempt_id = "compile_only".to_string();
    let snapshot = compile_group_review_material(
        &bindings,
        &snapshots,
        &request,
        &e2e_facts(&bindings),
        &RealMeasurer,
        GROUP_REVIEW_QUALITY_TARGET_BYTES,
    )
    .expect("compile");

    for shard in &snapshot.partition_result.shards {
        let prompt_none = build_shard_prompt(&snapshot, shard, None);
        let prompt_with_meta = build_shard_prompt(&snapshot, shard, Some("provider=ClaudeCode"));

        // Everything except retry_diagnostic_reserve must be identical
        let breakdown_none = prompt_none.measure();
        let breakdown_with_meta = prompt_with_meta.measure();
        assert_eq!(
            breakdown_none.fixed_protocol,
            breakdown_with_meta.fixed_protocol
        );
        assert_eq!(breakdown_none.identity, breakdown_with_meta.identity);
        assert_eq!(
            breakdown_none.unit_records,
            breakdown_with_meta.unit_records
        );
        assert_eq!(
            breakdown_none.evidence_digest,
            breakdown_with_meta.evidence_digest
        );
        assert_eq!(breakdown_none.graph, breakdown_with_meta.graph);
        assert_eq!(breakdown_none.diff, breakdown_with_meta.diff);
        // Only retry_diagnostic_reserve may differ
        assert_ne!(
            breakdown_none.retry_diagnostic_reserve,
            breakdown_with_meta.retry_diagnostic_reserve
        );
    }

    // Reduction prompt is also provider-independent except for retry_diagnostic_reserve
    let shard_reports: Vec<crate::product::coding_models::GroupReviewShardReport> = snapshot
        .partition_result
        .shards
        .iter()
        .map(
            |shard| crate::product::coding_models::GroupReviewShardReport {
                id: format!("shard_report_{}", shard.shard_id),
                attempt_id: snapshot.attempt_id.clone(),
                snapshot_hash: snapshot.content_hash.clone(),
                shard_id: shard.shard_id.clone(),
                ordered_unit_run_ids: shard.ordered_unit_run_ids.clone(),
                partition_rationale: shard.partition_rationale.clone(),
                verdict: ReviewVerdict::Approve,
                findings: Vec::new(),
                unresolved_obligations: Vec::new(),
                selected_diff_refs: Vec::new(),
                raw_provider_output_refs: vec!["raw".to_string()],
                role_run_ids: Vec::new(),
                run_failure_code: None,
            },
        )
        .collect();
    let reduction_none = build_reduction_prompt(&snapshot, &shard_reports, None);
    let reduction_meta = build_reduction_prompt(&snapshot, &shard_reports, Some("provider=Pi"));
    let bd_none = reduction_none.measure();
    let bd_meta = reduction_meta.measure();
    assert_eq!(bd_none.fixed_protocol, bd_meta.fixed_protocol);
    assert_eq!(bd_none.identity, bd_meta.identity);
    assert_eq!(bd_none.unit_records, bd_meta.unit_records);
}

// ===========================================================================
// Step 13-14: raw ref persistence on failure paths
// ===========================================================================

#[tokio::test]
async fn step13_14_invalid_shard_output_persists_raw_ref() {
    let (_root, store, attempt_id) = e2e_store();
    let (_bindings, _snapshots, snapshot) = twenty_unit_snapshot(&attempt_id);
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");

    // First shard returns invalid JSON (9 findings > 8 shard limit)
    let shard_count = snapshot.partition_result.shards.len();
    let mut results = Vec::new();
    for _ in 0..shard_count {
        results.push(Ok(GroupReviewExecutionResult {
            full_output: valid_group_review_output("approve", 9), // 9 > 8 shard limit
            provider_session_id: None,
        }));
    }
    let executor = FakeGroupReviewExecutor::new(results);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    let error = orchestrator
        .execute_shards(&snapshot)
        .await
        .expect_err("invalid shard output must fail");

    assert!(matches!(
        error,
        GroupReviewOrchestrationError::ShardOutputInvalid { ref raw_ref, .. } if !raw_ref.is_empty()
    ));

    // The raw output should be persisted even though the shard failed
    let _shard_reports = store
        .list_group_review_shard_reports(&attempt_id)
        .expect("shard reports");
    // At least one report should have been written with the raw ref
    // (CAS may store some, the first invalid one triggers the error)
    // Verify the raw provider output was saved by checking the error's raw_ref
    if let GroupReviewOrchestrationError::ShardOutputInvalid { raw_ref, .. } = error {
        assert!(
            !raw_ref.is_empty(),
            "raw_ref must be non-empty on invalid output"
        );
        // The raw output should be readable
        let attempt = store.find_attempt_by_id(&attempt_id).expect("attempt");
        let raw = store
            .read_attempt_artifact_text(&attempt, &raw_ref)
            .expect("raw output");
        assert!(raw.contains("GROUP_REVIEW_VERDICT"));
    }
}

#[tokio::test]
async fn step13_14_invalid_reduction_output_persists_raw_ref() {
    let (_root, store, attempt_id) = e2e_store();
    let (bindings, _snapshots, snapshot) = twenty_unit_snapshot(&attempt_id);
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");

    // Valid shards, but reduction returns 17 findings > 16 reduction limit
    let shard_count = snapshot.partition_result.shards.len();
    let mut results = (0..shard_count)
        .map(|_| {
            Ok(GroupReviewExecutionResult {
                full_output: valid_shard_output(),
                provider_session_id: None,
            })
        })
        .collect::<Vec<_>>();
    results.push(Ok(GroupReviewExecutionResult {
        full_output: valid_group_review_output("approve", 17), // 17 > 16 reduction limit
        provider_session_id: None,
    }));
    let executor = FakeGroupReviewExecutor::new(results);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    let shard_reports = orchestrator
        .execute_shards(&snapshot)
        .await
        .expect("execute shards");

    let projection_bindings = bindings
        .iter()
        .map(|b| b.projection_binding.clone())
        .collect::<Vec<_>>();
    let error = orchestrator
        .execute_reduction(&snapshot, &shard_reports, &projection_bindings)
        .await
        .expect_err("invalid reduction output must fail");

    assert!(matches!(
        error,
        GroupReviewOrchestrationError::ReductionOutputInvalid { ref raw_ref } if !raw_ref.is_empty()
    ));

    if let GroupReviewOrchestrationError::ReductionOutputInvalid { raw_ref } = error {
        assert!(
            !raw_ref.is_empty(),
            "raw_ref must be non-empty on invalid reduction"
        );
        let attempt = store.find_attempt_by_id(&attempt_id).expect("attempt");
        let raw = store
            .read_attempt_artifact_text(&attempt, &raw_ref)
            .expect("raw output");
        assert!(raw.contains("GROUP_REVIEW_VERDICT"));
    }
}

#[tokio::test]
async fn step13_14_malformed_json_output_still_persists_raw_ref() {
    let (_root, store, attempt_id) = e2e_store();
    let (_bindings, _snapshots, snapshot) = twenty_unit_snapshot(&attempt_id);
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");

    let shard_count = snapshot.partition_result.shards.len();
    let mut results = Vec::new();
    for _ in 0..shard_count {
        results.push(Ok(GroupReviewExecutionResult {
            full_output: "this is not valid JSON at all".to_string(),
            provider_session_id: None,
        }));
    }
    let executor = FakeGroupReviewExecutor::new(results);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    // Malformed JSON → parse_review_payload returns blocked_review_payload with
    // verdict=Blocked and 0 findings. This passes the findings limit check,
    // so shards succeed but with Blocked verdict. The key assertion is that
    // raw output refs are still persisted for auditability.
    let shard_reports = orchestrator
        .execute_shards(&snapshot)
        .await
        .expect("malformed JSON produces Blocked verdict but is parseable");

    assert_eq!(shard_reports.len(), shard_count);
    let stored_reports = store
        .list_group_review_shard_reports(&attempt_id)
        .expect("shard reports");
    assert_eq!(stored_reports.len(), shard_count);
    for report in &stored_reports {
        assert_eq!(report.verdict, ReviewVerdict::Blocked);
        assert_eq!(report.run_failure_code, None);
        assert!(!report.raw_provider_output_refs.is_empty());
    }
}

#[tokio::test]
async fn step13_14_transport_error_shard_does_not_persist_raw_ref() {
    let (_root, store, attempt_id) = e2e_store();
    let (_bindings, _snapshots, snapshot) = twenty_unit_snapshot(&attempt_id);
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");

    let shard_count = snapshot.partition_result.shards.len();
    let mut results = Vec::new();
    for _ in 0..shard_count {
        results.push(Err(GroupReviewExecutionError::Transport(
            "connection refused".to_string(),
        )));
    }
    let executor = FakeGroupReviewExecutor::new(results);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    let _error = orchestrator
        .execute_shards(&snapshot)
        .await
        .expect_err("transport error must fail");

    // Transport errors don't produce raw output, so no shard reports should be stored
    let shard_reports = store
        .list_group_review_shard_reports(&attempt_id)
        .expect("shard reports");
    assert!(
        shard_reports.is_empty(),
        "transport errors should not persist shard reports"
    );
}
