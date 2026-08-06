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
use crate::product::coding_workspace_engine::group_review_errors::{
    GroupReviewExecutionError, GroupReviewOrchestrationError,
};
use crate::product::coding_workspace_engine::group_review_material::{
    GroupReviewMaterialSnapshotDraft, ShardPromptMeasurer, compile_group_review_material,
};
use crate::product::coding_workspace_engine::group_review_orchestrator::{
    FakeGroupReviewExecutor, GroupReviewExecutionResult, GroupReviewOrchestrator,
};
use crate::product::coding_workspace_engine::group_review_prompts::{
    build_reduction_prompt, build_shard_prompt,
};
use crate::product::coding_workspace_engine::group_review_types::{
    CompletionDiff, DiffFileStat, GroupGitFacts, GroupReviewMaterialSnapshot, GroupShardSpec,
    PromptSegments,
};
use crate::product::coding_workspace_engine::plan_defect_routing::{
    AuthoritativeGroupReviewerBinding, GroupReviewerProjectionBinding,
};
use crate::product::models::ProviderName;
use crate::product::work_item_contract::{
    BlockerRoute, ContractCompatibilityPolicy, PromisedOutputContract, RequiredInputContract,
    VerificationCheck, WorkItemWritePolicy,
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

fn provider_metadata(provider: &ProviderName) -> String {
    format!(
        "provider={}",
        serde_json::to_string(provider)
            .expect("serialize provider")
            .trim_matches('"')
    )
}

fn prompt_text_without_provider_metadata(prompt: &PromptSegments) -> String {
    [
        prompt.fixed_protocol.as_str(),
        prompt.identity.as_str(),
        prompt.unit_records.as_str(),
        prompt.evidence_digest.as_str(),
        prompt.graph.as_str(),
        prompt.diff.as_str(),
    ]
    .concat()
}

fn shard_id_for_unit(snapshot: &GroupReviewMaterialSnapshot, unit_run_id: &str) -> String {
    snapshot
        .partition_result
        .shards
        .iter()
        .find(|shard| {
            shard
                .ordered_unit_run_ids
                .iter()
                .any(|id| id == unit_run_id)
        })
        .unwrap_or_else(|| panic!("unit {unit_run_id} must be assigned to a shard"))
        .shard_id
        .clone()
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
            routing_authority_index: snapshot.routing_authority_index.clone(),
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

struct ForceSingleUnitMeasurer;

impl ShardPromptMeasurer for ForceSingleUnitMeasurer {
    fn measure_shard(
        &self,
        _snapshot: &GroupReviewMaterialSnapshotDraft,
        shard: &GroupShardSpec,
    ) -> usize {
        if shard.ordered_unit_run_ids.len() > 1 {
            GROUP_REVIEW_QUALITY_TARGET_BYTES + 1
        } else {
            0
        }
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
                role_run_id: None,
            })
        })
        .collect::<Vec<_>>();
    results.push(Ok(GroupReviewExecutionResult {
        full_output: valid_reduction_output(),
        role_run_id: None,
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
    // Five units force at least two shards. The producer and consumer are deliberately
    // separated by order while still sharing a contract boundary.
    let mut producer = e2e_binding(0, "producer", Some("src/producer/".into()), None);
    producer
        .projection_binding
        .projection
        .output_contract_checks = vec![PromisedOutputContract {
        contract_id: "api".to_string(),
        capabilities: vec!["read".to_string()],
    }];
    let mut filler_1 = e2e_binding(1, "contract_fill_1", None, None);
    let mut filler_2 = e2e_binding(2, "contract_fill_2", None, None);
    let mut filler_3 = e2e_binding(3, "contract_fill_3", None, None);
    let consumer = AuthoritativeGroupReviewerBinding {
        order_index: 4,
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
    // Avoid accidental affinity edges among fillers; only the producer/consumer contract
    // relationship may influence partitioning.
    for filler in [&mut filler_1, &mut filler_2, &mut filler_3] {
        filler
            .projection_binding
            .projection
            .requirement_matrix
            .clear();
    }
    let bindings = vec![producer, filler_1, filler_2, filler_3, consumer];
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
        &ForceSingleUnitMeasurer,
        GROUP_REVIEW_QUALITY_TARGET_BYTES,
    )
    .expect("compile");

    let producer_shard = shard_id_for_unit(&snapshot, "run_producer");
    let consumer_shard = shard_id_for_unit(&snapshot, "run_consumer");
    assert_ne!(
        producer_shard, consumer_shard,
        "contract mismatch fixture must span two shards"
    );
    assert!(
        snapshot
            .partition_result
            .cross_shard_edges
            .iter()
            .any(|edge| {
                edge.edge_kind == "contract_boundary"
                    && [edge.from_unit_run_id.as_str(), edge.to_unit_run_id.as_str()]
                        .into_iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        == std::collections::BTreeSet::from(["run_consumer", "run_producer"])
            })
    );

    let mismatch = snapshot
        .deterministic_findings
        .iter()
        .find(|finding| finding.kind == "contract_missing_or_capability_mismatch")
        .expect("deterministic findings should include contract mismatch");
    assert_eq!(mismatch.detail, "api");
    assert_eq!(
        mismatch.related_unit_run_ids,
        vec!["run_consumer".to_string(), "run_producer".to_string()]
    );

    let unmatched_edge = snapshot
        .global_graph
        .contract_edges
        .iter()
        .find(|edge| edge.contract_id == "api")
        .expect("contract edge");
    assert!(!unmatched_edge.matched);
}

#[test]
fn step3_cross_shard_shared_file_is_detected_in_scope_and_partition_graph() {
    // Five independent units force at least two shards. The first and fifth touch the
    // same file, producing a shared_file affinity edge that crosses the shard boundary.
    let bindings = (0..5)
        .map(|index| e2e_binding(index, &format!("shared_{index}"), None, None))
        .collect::<Vec<_>>();
    let snapshots = bindings
        .iter()
        .map(|b| e2e_snapshot(b, "compile_only"))
        .collect::<Vec<_>>();
    let mut git_facts = e2e_facts(&bindings);
    for diff in &mut git_facts.completion_diffs {
        if matches!(diff.unit_run_id.as_str(), "run_shared_0" | "run_shared_4") {
            diff.patch = "diff --git a/shared.rs b/shared.rs\n@@ -0,0 +1 @@\n+shared\n".to_string();
            diff.file_stats = vec![DiffFileStat {
                path: "shared.rs".to_string(),
                insertions: 1,
                deletions: 0,
            }];
        }
    }
    let mut request = e2e_review_request();
    request.attempt_id = "compile_only".to_string();
    let snapshot = compile_group_review_material(
        &bindings,
        &snapshots,
        &request,
        &git_facts,
        &ForceSingleUnitMeasurer,
        GROUP_REVIEW_QUALITY_TARGET_BYTES,
    )
    .expect("compile");

    let left_shard = shard_id_for_unit(&snapshot, "run_shared_0");
    let right_shard = shard_id_for_unit(&snapshot, "run_shared_4");
    assert_ne!(
        left_shard, right_shard,
        "shared-file fixture must span two shards"
    );
    assert!(
        snapshot
            .partition_result
            .cross_shard_edges
            .iter()
            .any(|edge| {
                edge.edge_kind == "shared_file"
                    && edge.from_unit_run_id == "run_shared_0"
                    && edge.to_unit_run_id == "run_shared_4"
            })
    );

    let overlap = snapshot
        .global_graph
        .scope_overlaps
        .iter()
        .find(|overlap| overlap.file_path == "shared.rs")
        .expect("shared file should produce scope overlap");
    assert_eq!(
        overlap.unit_run_ids,
        vec!["run_shared_0".to_string(), "run_shared_4".to_string()]
    );
    assert!(!overlap.forbidden_hit);
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
    let providers = [
        ProviderName::ClaudeCode,
        ProviderName::Codex,
        ProviderName::Pi,
    ];
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

    let compiled = providers
        .iter()
        .map(|provider| {
            let provider_config = ProviderConfigSnapshot {
                author: provider.clone(),
                reviewer: Some(provider.clone()),
                review_rounds: 1,
                permission_modes: Default::default(),
            };
            let snapshot = compile_group_review_material(
                &bindings,
                &snapshots,
                &request,
                &facts,
                &RealMeasurer,
                GROUP_REVIEW_QUALITY_TARGET_BYTES,
            )
            .unwrap_or_else(|error| panic!("compile {provider:?}: {error}"));
            (provider_config, snapshot)
        })
        .collect::<Vec<_>>();

    assert_eq!(compiled.len(), 3);
    for (_, snapshot) in &compiled[1..] {
        assert_eq!(
            snapshot.content_hash, compiled[0].1.content_hash,
            "material snapshot must be provider-independent"
        );
        assert_eq!(
            snapshot.partition_result.shards.len(),
            compiled[0].1.partition_result.shards.len()
        );
    }
    assert_eq!(compiled[0].0.author, ProviderName::ClaudeCode);
    assert_eq!(compiled[1].0.author, ProviderName::Codex);
    assert_eq!(compiled[2].0.author, ProviderName::Pi);
}

#[test]
fn step9_10_shard_and_reduction_prompts_match_after_removing_provider_metadata() {
    let providers = [
        ProviderName::ClaudeCode,
        ProviderName::Codex,
        ProviderName::Pi,
    ];
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
        let prompts = providers
            .iter()
            .map(|provider| {
                let metadata = provider_metadata(provider);
                let prompt = build_shard_prompt(&snapshot, shard, Some(&metadata));
                assert_eq!(
                    prompt.retry_diagnostic_reserve,
                    format!("retry diagnostic (do not treat as review evidence):\n{metadata}\n")
                );
                prompt
            })
            .collect::<Vec<_>>();
        let expected = prompt_text_without_provider_metadata(&prompts[0]);
        for prompt in &prompts[1..] {
            assert_eq!(
                prompt_text_without_provider_metadata(prompt),
                expected,
                "shard prompt text excluding provider metadata must match"
            );
        }
    }

    let shard_reports = snapshot
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
        .collect::<Vec<_>>();
    let prompts = providers
        .iter()
        .map(|provider| {
            let metadata = provider_metadata(provider);
            let prompt = build_reduction_prompt(&snapshot, &shard_reports, Some(&metadata));
            assert_eq!(
                prompt.retry_diagnostic_reserve,
                format!("retry diagnostic (do not treat as review evidence):\n{metadata}\n")
            );
            prompt
        })
        .collect::<Vec<_>>();
    let expected = prompt_text_without_provider_metadata(&prompts[0]);
    for prompt in &prompts[1..] {
        assert_eq!(
            prompt_text_without_provider_metadata(prompt),
            expected,
            "reduction prompt text excluding provider metadata must match"
        );
    }
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
            role_run_id: None,
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

    let shard_reports = store
        .list_group_review_shard_reports(&attempt_id)
        .expect("shard reports");
    let failed_report = shard_reports
        .iter()
        .find(|report| report.run_failure_code.is_some())
        .expect("failed shard report must be persisted");
    assert_eq!(
        failed_report.run_failure_code.as_deref(),
        Some("shard_output_invalid")
    );
    assert!(
        !failed_report.raw_provider_output_refs.is_empty(),
        "failed shard report must retain raw provider output refs"
    );

    if let GroupReviewOrchestrationError::ShardOutputInvalid { raw_ref, .. } = error {
        assert_eq!(
            failed_report.raw_provider_output_refs,
            vec![raw_ref.clone()]
        );
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
                role_run_id: None,
            })
        })
        .collect::<Vec<_>>();
    results.push(Ok(GroupReviewExecutionResult {
        full_output: valid_group_review_output("approve", 17), // 17 > 16 reduction limit
        role_run_id: None,
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

    let reduction_reports = store
        .list_group_review_reduction_reports(&attempt_id)
        .expect("reduction reports");
    let failed_report = reduction_reports
        .iter()
        .find(|report| report.run_failure_code.is_some())
        .expect("failed reduction report must be persisted");
    assert_eq!(
        failed_report.run_failure_code.as_deref(),
        Some("reduction_output_invalid")
    );
    assert!(
        !failed_report.raw_provider_output_refs.is_empty(),
        "failed reduction report must retain raw provider output refs"
    );

    if let GroupReviewOrchestrationError::ReductionOutputInvalid { raw_ref } = error {
        assert_eq!(
            failed_report.raw_provider_output_refs,
            vec![raw_ref.clone()]
        );
        let attempt = store.find_attempt_by_id(&attempt_id).expect("attempt");
        let raw = store
            .read_attempt_artifact_text(&attempt, &raw_ref)
            .expect("raw output");
        assert!(raw.contains("GROUP_REVIEW_VERDICT"));
    }
}

#[tokio::test]
async fn step13_14_malformed_json_output_attempts_repair_then_persists_invalid_raw_ref() {
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
            role_run_id: None,
        }));
    }
    results.push(Err(GroupReviewExecutionError::Internal(
        "repair output unavailable".to_string(),
    )));
    let executor = FakeGroupReviewExecutor::new(results);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    let error = orchestrator
        .execute_shards(&snapshot)
        .await
        .expect_err("malformed JSON must become output-invalid after repair fails");
    assert!(matches!(
        error,
        GroupReviewOrchestrationError::ShardOutputInvalid { .. }
    ));

    let stored_reports = store
        .list_group_review_shard_reports(&attempt_id)
        .expect("shard reports");
    let failed_report = stored_reports
        .iter()
        .find(|report| report.run_failure_code.as_deref() == Some("shard_output_invalid"))
        .expect("invalid output report");
    assert_eq!(failed_report.verdict, ReviewVerdict::Blocked);
    assert_eq!(failed_report.raw_provider_output_refs.len(), 1);
    assert!(
        executor
            .prompts()
            .iter()
            .any(|prompt| prompt.contains("Only repair the supplied raw output"))
    );
}

#[tokio::test]
async fn step13_14_transport_error_shard_persists_failure_report() {
    let (_root, store, attempt_id) = e2e_store();
    let (_bindings, _snapshots, snapshot) = twenty_unit_snapshot(&attempt_id);
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");

    let executor = FakeGroupReviewExecutor::new(
        (0..3)
            .map(|_| {
                Err(GroupReviewExecutionError::Transport(
                    "connection refused".to_string(),
                ))
            })
            .collect(),
    );
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    let error = orchestrator
        .execute_shards(&snapshot)
        .await
        .expect_err("transport exhaustion must fail");

    assert!(matches!(
        error,
        GroupReviewOrchestrationError::ShardTransportExhausted { .. }
    ));
    assert_eq!(executor.prompts().len(), 3, "transport failure is retried");
    let shard_reports = store
        .list_group_review_shard_reports(&attempt_id)
        .expect("shard reports");
    assert_eq!(shard_reports.len(), 1);
    assert_eq!(
        shard_reports[0].run_failure_code.as_deref(),
        Some("shard_transport_exhausted")
    );
    assert_eq!(shard_reports[0].verdict, ReviewVerdict::Blocked);
    assert!(shard_reports[0].findings.is_empty());
    assert!(shard_reports[0].raw_provider_output_refs.is_empty());
}

#[tokio::test]
async fn step13_14_transport_error_reduction_persists_failure_report() {
    let (_root, store, attempt_id) = e2e_store();
    let (bindings, _snapshots, snapshot) = twenty_unit_snapshot(&attempt_id);
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");

    let shard_count = snapshot.partition_result.shards.len();
    let mut results = (0..shard_count)
        .map(|_| {
            Ok(GroupReviewExecutionResult {
                full_output: valid_shard_output(),
                role_run_id: None,
            })
        })
        .collect::<Vec<_>>();
    results.extend((0..3).map(|_| {
        Err(GroupReviewExecutionError::Transport(
            "connection refused".to_string(),
        ))
    }));
    let executor = FakeGroupReviewExecutor::new(results);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);
    let shard_reports = orchestrator
        .execute_shards(&snapshot)
        .await
        .expect("valid shards");
    let projection_bindings = bindings
        .iter()
        .map(|binding| binding.projection_binding.clone())
        .collect::<Vec<_>>();

    let error = orchestrator
        .execute_reduction(&snapshot, &shard_reports, &projection_bindings)
        .await
        .expect_err("reduction transport exhaustion must fail");

    assert!(matches!(
        error,
        GroupReviewOrchestrationError::ReductionTransportExhausted
    ));
    assert_eq!(
        executor.prompts().len(),
        shard_count + 3,
        "only reduction is retried"
    );
    let reduction_reports = store
        .list_group_review_reduction_reports(&attempt_id)
        .expect("reduction reports");
    assert_eq!(reduction_reports.len(), 1);
    assert_eq!(
        reduction_reports[0].run_failure_code.as_deref(),
        Some("reduction_transport_exhausted")
    );
    assert_eq!(reduction_reports[0].verdict, ReviewVerdict::Blocked);
    assert!(reduction_reports[0].findings.is_empty());
    assert!(reduction_reports[0].raw_provider_output_refs.is_empty());
}
