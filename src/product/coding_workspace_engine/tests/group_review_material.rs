use crate::product::coding_models::{
    CodingUnitRun, CodingUnitRunStatus, CompactFindingDigest, PushStatus, RemoteKind,
    ReviewRequest, ReviewRequestKind, ReviewRequestOwnerKind, ReviewVerdict,
    UnitReviewConclusionSnapshot,
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
use crate::product::models::{PlanDefectRoute, RepairTargetKind};
use crate::product::work_item_contract::{
    BlockerRoute, BlockerRule, ContractCompatibilityPolicy, PromisedOutputContract,
    RequiredInputContract, VerificationCheck, WorkItemWritePolicy,
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
        owner_kind: ReviewRequestOwnerKind::Attempt,
        pointer_publication_id: None,
        created_at: "ignored".to_string(),
        updated_at: "ignored".to_string(),
    }
}

fn facts(bindings: &[AuthoritativeGroupReviewerBinding]) -> GroupGitFacts {
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

#[test]
fn group_review_material_content_hash_is_stable_across_input_order() {
    let mut first = binding(2, "b", Vec::new(), Vec::new());
    first.projection_binding.projection.blocker_routing = vec![BlockerRule {
        reason_code: "reason_b".to_string(),
        route: BlockerRoute::PlanRepairUpstream,
        target_contract_refs: vec!["contract_z".to_string(), "contract_a".to_string()],
    }];
    let mut second = binding(1, "a", Vec::new(), Vec::new());
    second.projection_binding.projection.blocker_routing = vec![BlockerRule {
        reason_code: "reason_a".to_string(),
        route: BlockerRoute::VerificationRetry,
        target_contract_refs: Vec::new(),
    }];
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
    assert_eq!(forward_result.routing_authority_index.len(), 2);
    assert_eq!(
        forward_result.routing_authority_index[0].source_unit_run_id,
        "run_a"
    );
    assert_eq!(
        forward_result.routing_authority_index[1].target_contract_refs,
        ["contract_a", "contract_z"]
    );
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
fn material_only_marks_paths_forbidden_by_their_owning_unit() {
    let mut owner = binding(1, "owner", Vec::new(), Vec::new());
    owner
        .projection_binding
        .projection
        .scope_policy
        .exclusive_scopes = vec!["package.json".to_string()];
    let mut other = binding(2, "other", Vec::new(), Vec::new());
    other
        .projection_binding
        .projection
        .scope_policy
        .forbidden_scopes = vec!["package.json".to_string()];
    let bindings = vec![owner, other];
    let snapshots = bindings.iter().map(snapshot).collect::<Vec<_>>();
    let mut git_facts = facts(&bindings);
    git_facts.completion_diffs[0].patch =
        "diff --git a/package.json b/package.json\n@@ -0,0 +1 @@\n+{}\n".to_string();
    git_facts.completion_diffs[0].file_stats = vec![DiffFileStat {
        path: "package.json".to_string(),
        insertions: 1,
        deletions: 0,
    }];

    let result = compile_group_review_material(
        &bindings,
        &snapshots,
        &request(),
        &git_facts,
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");

    assert!(
        !result.diff_index.files[0].forbidden_scope_hit,
        "another unit's forbidden scope must not flag the owner's exclusive path"
    );
    assert!(
        result
            .deterministic_findings
            .iter()
            .all(|finding| finding.kind != "forbidden_scope_hit"),
        "deterministic checks must use the path owner, not every binding"
    );
}

#[test]
fn material_marks_paths_forbidden_by_their_owning_unit() {
    let mut owner = binding(1, "owner", Vec::new(), Vec::new());
    owner
        .projection_binding
        .projection
        .scope_policy
        .forbidden_scopes = vec!["package.json".to_string()];
    let bindings = vec![owner];
    let snapshots = bindings.iter().map(snapshot).collect::<Vec<_>>();
    let mut git_facts = facts(&bindings);
    git_facts.completion_diffs[0].patch =
        "diff --git a/package.json b/package.json\n@@ -0,0 +1 @@\n+{}\n".to_string();
    git_facts.completion_diffs[0].file_stats = vec![DiffFileStat {
        path: "package.json".to_string(),
        insertions: 1,
        deletions: 0,
    }];

    let result = compile_group_review_material(
        &bindings,
        &snapshots,
        &request(),
        &git_facts,
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");

    assert!(result.diff_index.files[0].forbidden_scope_hit);
    assert!(result.deterministic_findings.iter().any(|finding| {
        finding.kind == "forbidden_scope_hit"
            && finding.detail == "package.json"
            && finding.related_unit_run_ids == ["run_owner"]
    }));
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
        std::slice::from_ref(&unit),
        &snapshots,
        &request(),
        &facts(std::slice::from_ref(&unit)),
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
struct InspectingMeasurer;
impl ShardPromptMeasurer for InspectingMeasurer {
    fn measure_shard(
        &self,
        snapshot: &GroupReviewMaterialSnapshotDraft,
        _: &GroupShardSpec,
    ) -> usize {
        assert!(
            snapshot
                .diff_index
                .hunks
                .iter()
                .all(|hunk| !hunk.body.contains("super-secret"))
        );
        20_000
    }
}

#[test]
fn material_redacts_hunks_before_budget_measurement() {
    let unit = binding(1, "secret", Vec::new(), Vec::new());
    let snapshots = vec![snapshot(&unit)];
    let mut git_facts = facts(std::slice::from_ref(&unit));
    git_facts.completion_diffs[0].patch = "diff --git a/src/secret/file.rs b/src/secret/file.rs\n@@ -0,0 +1 @@\n+api_key=super-secret\n".to_string();
    git_facts.completion_diffs[0].file_stats = Vec::new();
    let result = compile_group_review_material(
        &[unit],
        &snapshots,
        &request(),
        &git_facts,
        &InspectingMeasurer,
        1,
    )
    .expect("redacted material can be measured");
    assert_eq!(result.diff_index.hunks[0].body, "[REDACTED]");
}

#[test]
fn material_persists_header_and_utf8_safe_truncated_fragment_body() {
    let mut unit = binding(1, "utf8", Vec::new(), Vec::new());
    unit.projection_binding
        .projection
        .scope_policy
        .forbidden_scopes = vec!["src/utf8/".to_string()];
    let snapshots = vec![snapshot(&unit)];
    let mut git_facts = facts(std::slice::from_ref(&unit));
    git_facts.completion_diffs[0].patch = format!(
        "diff --git a/src/utf8/file.rs b/src/utf8/file.rs\n@@ -0,0 +1 @@\n+{}\n",
        "界".repeat(5_000)
    );
    git_facts.completion_diffs[0].file_stats = Vec::new();
    let result = compile_group_review_material(
        &[unit],
        &snapshots,
        &request(),
        &git_facts,
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    let fragment = result.diff_index.shard_selections[0]
        .fragments
        .iter()
        .find(|fragment| fragment.level != 'E')
        .expect("fragment");
    assert!(
        fragment
            .body
            .starts_with("path: src/utf8/file.rs\nstat: +1 -0\n")
    );
    assert!(fragment.truncated);
    assert!(std::str::from_utf8(fragment.body.as_bytes()).is_ok());
}

#[test]
fn material_adds_final_only_b_file_hunk_to_selection() {
    let unit = binding(1, "one", Vec::new(), Vec::new());
    let snapshots = vec![snapshot(&unit)];
    let mut git_facts = facts(std::slice::from_ref(&unit));
    git_facts.final_diff =
        "diff --git a/final-only.rs b/final-only.rs\n@@ -0,0 +1 @@\n+fn final_only() {}\n"
            .to_string();
    let result = compile_group_review_material(
        &[unit],
        &snapshots,
        &request(),
        &git_facts,
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    assert!(
        result
            .diff_index
            .hunks
            .iter()
            .any(|hunk| hunk.path == "final-only.rs" && hunk.owner_unit_run_ids.is_empty())
    );
    assert!(
        result.diff_index.shard_selections[0]
            .fragments
            .iter()
            .any(|fragment| fragment.path == "final-only.rs"
                && fragment.level == 'B'
                && !fragment.body.is_empty())
    );
}

#[test]
fn material_splits_oversized_affinity_component_and_records_cross_edges() {
    let bindings = (0..5)
        .map(|index| {
            binding(
                index,
                &format!("{index}"),
                Vec::new(),
                vec![PromisedOutputContract {
                    contract_id: "shared".to_string(),
                    capabilities: vec!["x".to_string()],
                }],
            )
        })
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
    assert_eq!(result.partition_result.shards.len(), 2);
    assert!(
        result
            .partition_result
            .cross_shard_edges
            .iter()
            .any(|edge| edge.edge_kind == "contract_boundary")
    );
}

#[test]
fn material_uses_provider_identity_for_contract_matching() {
    let wrong = binding(
        1,
        "wrong",
        Vec::new(),
        vec![PromisedOutputContract {
            contract_id: "api".to_string(),
            capabilities: vec!["read".to_string()],
        }],
    );
    let right = binding(
        2,
        "right",
        Vec::new(),
        vec![PromisedOutputContract {
            contract_id: "api".to_string(),
            capabilities: vec!["read".to_string()],
        }],
    );
    let consumer = binding(
        3,
        "consumer",
        vec![RequiredInputContract {
            contract_id: "api".to_string(),
            provider_logical_work_item_id: "work_right".to_string(),
            required_capabilities: vec!["read".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
        Vec::new(),
    );
    let bindings = vec![wrong, right, consumer];
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
    let edge = result
        .global_graph
        .contract_edges
        .iter()
        .find(|edge| edge.contract_id == "api")
        .expect("edge");
    assert_eq!(edge.producer_unit_run_id, "run_right");
    assert!(edge.matched);
}

#[test]
fn material_moves_routing_authority_out_of_trimmed_unit_record() {
    let mut unit = binding(1, "trimmed", Vec::new(), Vec::new());
    unit.projection_binding.projection.blocker_routing = vec![BlockerRule {
        reason_code: "verification_command_failed".to_string(),
        route: BlockerRoute::VerificationRetry,
        target_contract_refs: vec!["contract-routing-authority".to_string()],
    }];
    unit.projection_binding.projection.input_contract_checks = (0..16)
        .map(|index| RequiredInputContract {
            contract_id: format!("contract-{index}-{}", "x".repeat(80)),
            provider_logical_work_item_id: format!("work-{index}"),
            required_capabilities: vec!["capability".repeat(12)],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        })
        .collect();
    unit.projection_binding.projection.scope_policy = WorkItemWritePolicy {
        exclusive_scopes: vec![format!("src/{}", "exclusive/".repeat(60))],
        forbidden_scopes: vec![format!("src/{}", "forbidden/".repeat(60))],
    };
    let snapshots = vec![snapshot(&unit)];
    let result = compile_group_review_material(
        std::slice::from_ref(&unit),
        &snapshots,
        &request(),
        &facts(std::slice::from_ref(&unit)),
        &FixedMeasurer,
        1_000,
    )
    .expect("non-authoritative fields should be trim-able");
    let record = result.unit_records.first().expect("unit record");

    assert!(record.contract_interfaces.len() < 16);
    assert_eq!(result.routing_authority_index.len(), 1);
    let authority = &result.routing_authority_index[0];
    assert_eq!(authority.source_unit_run_id, "run_trimmed");
    assert_eq!(authority.source_logical_work_item_id, "work_trimmed");
    assert_eq!(authority.source_work_item_revision_id, "revision_trimmed");
    assert_eq!(authority.reason_code, "verification_command_failed");
    assert_eq!(authority.allowed_route, PlanDefectRoute::VerificationRetry);
    assert_eq!(authority.required_target_kind, None);
    assert_eq!(
        authority.target_contract_refs,
        ["contract-routing-authority"]
    );
    assert!(serde_json::to_vec(record).unwrap().len() <= 1_200);
}

#[test]
fn material_keeps_realistic_full_routing_authority_without_exceeding_record_limit() {
    let mut unit = binding(1, "full-routing", Vec::new(), Vec::new());
    unit.projection_binding.projection.blocker_routing = vec![
        BlockerRule {
            reason_code: "verification_command_failed".to_string(),
            route: BlockerRoute::VerificationRetry,
            target_contract_refs: Vec::new(),
        },
        BlockerRule {
            reason_code: "acceptance_criteria_not_met".to_string(),
            route: BlockerRoute::CoderRework,
            target_contract_refs: Vec::new(),
        },
        BlockerRule {
            reason_code: "upstream_handoff_fields_missing".to_string(),
            route: BlockerRoute::PlanRepairUpstream,
            target_contract_refs: vec!["oc_compact_duration_module".to_string()],
        },
        BlockerRule {
            reason_code: "upstream_module_path_or_export_mismatch".to_string(),
            route: BlockerRoute::PlanRepairUpstream,
            target_contract_refs: vec!["oc_compact_duration_module".to_string()],
        },
        BlockerRule {
            reason_code: "upstream_module_behavior_contradicts_examples".to_string(),
            route: BlockerRoute::PlanRepairUpstream,
            target_contract_refs: vec!["oc_compact_duration_module".to_string()],
        },
        BlockerRule {
            reason_code: "web_test_glob_not_registered".to_string(),
            route: BlockerRoute::PlanRepairUpstream,
            target_contract_refs: vec!["oc_repo_test_glob_registration".to_string()],
        },
        BlockerRule {
            reason_code: "page_requires_write_outside_scope".to_string(),
            route: BlockerRoute::PlanRepairCurrent,
            target_contract_refs: vec!["oc_compact_duration_module".to_string()],
        },
        BlockerRule {
            reason_code: "out_of_contract_input_semantics_undefined".to_string(),
            route: BlockerRoute::StoryAmendment,
            target_contract_refs: vec!["oc_compact_duration_module".to_string()],
        },
        BlockerRule {
            reason_code: "module_loading_strategy_infeasible_without_build".to_string(),
            route: BlockerRoute::DesignAmendment,
            target_contract_refs: vec!["oc_compact_duration_module".to_string()],
        },
    ];
    let snapshots = vec![snapshot(&unit)];

    let result = compile_group_review_material(
        std::slice::from_ref(&unit),
        &snapshots,
        &request(),
        &facts(std::slice::from_ref(&unit)),
        &FixedMeasurer,
        1_000,
    )
    .expect("full authoritative routing must remain reviewable");

    let record = result.unit_records.first().expect("unit record");
    assert!(serde_json::to_vec(record).unwrap().len() <= 1_200);
    assert_eq!(result.routing_authority_index.len(), 9);
    assert!(
        result
            .routing_authority_index
            .iter()
            .any(|target| target.reason_code == "module_loading_strategy_infeasible_without_build")
    );
    let current_repair = result
        .routing_authority_index
        .iter()
        .find(|target| target.reason_code == "page_requires_write_outside_scope")
        .expect("current work item repair authority");
    assert_eq!(current_repair.allowed_route, PlanDefectRoute::PlanRepair);
    assert_eq!(
        current_repair.required_target_kind,
        Some(RepairTargetKind::CurrentWorkItem)
    );
}

#[test]
fn material_keeps_long_routing_authority_outside_unit_record() {
    let mut unit = binding(1, "routing-oversized", Vec::new(), Vec::new());
    unit.projection_binding.projection.blocker_routing = vec![BlockerRule {
        reason_code: "routing-authority".repeat(50),
        route: BlockerRoute::VerificationRetry,
        target_contract_refs: vec!["contract-ref".repeat(70)],
    }];
    let snapshots = vec![snapshot(&unit)];
    let result = compile_group_review_material(
        std::slice::from_ref(&unit),
        &snapshots,
        &request(),
        &facts(std::slice::from_ref(&unit)),
        &FixedMeasurer,
        1_000,
    )
    .expect("long routing authority must not consume unit record bytes");

    let record = result.unit_records.first().expect("unit record");
    assert!(serde_json::to_vec(record).unwrap().len() <= 1_200);
    assert_eq!(result.routing_authority_index.len(), 1);
    assert_eq!(
        result.routing_authority_index[0].reason_code,
        "routing-authority".repeat(50)
    );
    assert_eq!(
        result.routing_authority_index[0].target_contract_refs,
        ["contract-ref".repeat(70)]
    );
}

#[test]
fn material_rejects_uncompressible_oversized_record() {
    let mut unit = binding(1, "oversized", Vec::new(), Vec::new());
    unit.run.id = "x".repeat(900);
    let snapshots = vec![snapshot(&unit)];
    let error = compile_group_review_material(
        std::slice::from_ref(&unit),
        &snapshots,
        &request(),
        &facts(std::slice::from_ref(&unit)),
        &FixedMeasurer,
        1_000,
    )
    .expect_err("oversized identity must fail");
    assert!(
        matches!(error, crate::product::coding_workspace_engine::group_review_types::GroupMaterialError::Internal(message) if message == "unit_cross_review_record_exceeds_size_limit")
    );
}

#[test]
fn material_is_stable_when_completion_diff_input_is_reversed() {
    let first = binding(1, "a", Vec::new(), Vec::new());
    let second = binding(2, "b", Vec::new(), Vec::new());
    let bindings = vec![first, second];
    let snapshots = bindings.iter().map(snapshot).collect::<Vec<_>>();
    let forward = facts(&bindings);
    let mut reverse = facts(&bindings);
    reverse.completion_diffs.reverse();
    let first_result = compile_group_review_material(
        &bindings,
        &snapshots,
        &request(),
        &forward,
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    let second_result = compile_group_review_material(
        &bindings,
        &snapshots,
        &request(),
        &reverse,
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    assert_eq!(first_result.content_hash, second_result.content_hash);
}

#[test]
fn material_exposes_compiler_version() {
    let unit = binding(1, "version", Vec::new(), Vec::new());
    let snapshots = vec![snapshot(&unit)];
    let result = compile_group_review_material(
        std::slice::from_ref(&unit),
        &snapshots,
        &request(),
        &facts(std::slice::from_ref(&unit)),
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    assert_eq!(result.compiler_version, "group-review-material-compiler-v1");
}
#[test]
fn material_redacts_sensitive_hunk_header_before_measurement_and_selection() {
    let unit = binding(1, "header", Vec::new(), Vec::new());
    let snapshots = vec![snapshot(&unit)];
    let mut git_facts = facts(std::slice::from_ref(&unit));
    git_facts.completion_diffs[0].patch = "diff --git a/src/header/file.rs b/src/header/file.rs\n@@ -1 +1 @@ api_key=header-secret\n+safe\n".to_string();
    git_facts.completion_diffs[0].file_stats = Vec::new();
    let result = compile_group_review_material(
        &[unit],
        &snapshots,
        &request(),
        &git_facts,
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    assert!(!result.diff_index.hunks[0].header.contains("header-secret"));
}

#[test]
fn material_charges_a_file_header_once_across_multiple_hunks() {
    let mut unit = binding(1, "many", Vec::new(), Vec::new());
    unit.projection_binding
        .projection
        .scope_policy
        .forbidden_scopes = vec!["src/many/".to_string()];
    let snapshots = vec![snapshot(&unit)];
    let mut git_facts = facts(std::slice::from_ref(&unit));
    git_facts.completion_diffs[0].patch = format!(
        "diff --git a/src/many/file.rs b/src/many/file.rs\n@@ -0,0 +1 @@\n+{}\n@@ -2,0 +3 @@\n+{}\n",
        "a".repeat(7_000),
        "b".repeat(7_000)
    );
    git_facts.completion_diffs[0].file_stats = Vec::new();
    let result = compile_group_review_material(
        std::slice::from_ref(&unit),
        &snapshots,
        &request(),
        &git_facts,
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    let fragments = result.diff_index.shard_selections[0]
        .fragments
        .iter()
        .filter(|fragment| fragment.path == "src/many/file.rs")
        .collect::<Vec<_>>();
    assert_eq!(
        fragments
            .iter()
            .filter(|fragment| fragment.body.starts_with("path: src/many/file.rs\n"))
            .count(),
        1
    );
    assert!(
        fragments
            .iter()
            .map(|fragment| fragment.body.len())
            .sum::<usize>()
            <= 10_500
    );
}

#[test]
fn material_keeps_shared_file_pair_together_across_four_unit_boundary() {
    let fillers = (0..3)
        .map(|index| binding(index, &format!("filler-{index}"), Vec::new(), Vec::new()))
        .collect::<Vec<_>>();
    let left = binding(3, "shared-left", Vec::new(), Vec::new());
    let right = binding(4, "shared-right", Vec::new(), Vec::new());
    let mut bindings = fillers;
    bindings.extend([left, right]);
    let snapshots = bindings.iter().map(snapshot).collect::<Vec<_>>();
    let mut git_facts = facts(&bindings);
    for diff in git_facts.completion_diffs.iter_mut().filter(|diff| {
        diff.unit_run_id == "run_shared-left" || diff.unit_run_id == "run_shared-right"
    }) {
        diff.patch = "diff --git a/shared.rs b/shared.rs\n@@ -0,0 +1 @@\n+shared\n".to_string();
        diff.file_stats = vec![DiffFileStat {
            path: "shared.rs".to_string(),
            insertions: 1,
            deletions: 0,
        }];
    }
    let result = compile_group_review_material(
        &bindings,
        &snapshots,
        &request(),
        &git_facts,
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    assert!(result.partition_result.shards.iter().any(|shard| {
        shard
            .ordered_unit_run_ids
            .contains(&"run_shared-left".to_string())
            && shard
                .ordered_unit_run_ids
                .contains(&"run_shared-right".to_string())
    }));

    let no_affinity = (0..5)
        .map(|index| binding(index, &format!("plain-{index}"), Vec::new(), Vec::new()))
        .collect::<Vec<_>>();
    let no_affinity_snapshots = no_affinity.iter().map(snapshot).collect::<Vec<_>>();
    let control = compile_group_review_material(
        &no_affinity,
        &no_affinity_snapshots,
        &request(),
        &facts(&no_affinity),
        &FixedMeasurer,
        1_000,
    )
    .expect("control compile");
    assert!(control.partition_result.shards.iter().any(|shard| {
        shard.ordered_unit_run_ids
            == vec!["run_plain-0", "run_plain-1", "run_plain-2", "run_plain-3"]
    }));
    assert!(
        control
            .partition_result
            .shards
            .iter()
            .any(|shard| { shard.ordered_unit_run_ids == vec!["run_plain-4"] })
    );
}

#[test]
fn material_keeps_contract_boundary_pair_together_across_four_unit_boundary() {
    let fillers = (0..3)
        .map(|index| binding(index, &format!("filler-{index}"), Vec::new(), Vec::new()))
        .collect::<Vec<_>>();
    let left = binding(
        3,
        "contract-left",
        Vec::new(),
        vec![PromisedOutputContract {
            contract_id: "same".to_string(),
            capabilities: vec![],
        }],
    );
    let right = binding(
        4,
        "contract-right",
        Vec::new(),
        vec![PromisedOutputContract {
            contract_id: "same".to_string(),
            capabilities: vec![],
        }],
    );
    let mut bindings = fillers;
    bindings.extend([left, right]);
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
    assert!(result.partition_result.shards.iter().any(|shard| {
        shard
            .ordered_unit_run_ids
            .contains(&"run_contract-left".to_string())
            && shard
                .ordered_unit_run_ids
                .contains(&"run_contract-right".to_string())
    }));

    let control = (0..5)
        .map(|index| binding(index, &format!("plain-{index}"), Vec::new(), Vec::new()))
        .collect::<Vec<_>>();
    let control_snapshots = control.iter().map(snapshot).collect::<Vec<_>>();
    let control_result = compile_group_review_material(
        &control,
        &control_snapshots,
        &request(),
        &facts(&control),
        &FixedMeasurer,
        1_000,
    )
    .expect("control compile");
    assert!(control_result.partition_result.shards.iter().any(|shard| {
        shard.ordered_unit_run_ids
            == vec!["run_plain-0", "run_plain-1", "run_plain-2", "run_plain-3"]
    }));
    assert!(
        control_result
            .partition_result
            .shards
            .iter()
            .any(|shard| { shard.ordered_unit_run_ids == vec!["run_plain-4"] })
    );
}

struct SplittingMeasurer;
impl ShardPromptMeasurer for SplittingMeasurer {
    fn measure_shard(
        &self,
        snapshot: &GroupReviewMaterialSnapshotDraft,
        shard: &GroupShardSpec,
    ) -> usize {
        assert_eq!(snapshot.content_hash, "");
        shard.ordered_unit_run_ids.len() * 100
    }
}

#[test]
fn material_budget_repartitions_before_finalizing_hash() {
    let bindings = (0..4)
        .map(|index| binding(index, &format!("budget-{index}"), Vec::new(), Vec::new()))
        .collect::<Vec<_>>();
    let snapshots = bindings.iter().map(snapshot).collect::<Vec<_>>();
    let result = compile_group_review_material(
        &bindings,
        &snapshots,
        &request(),
        &facts(&bindings),
        &SplittingMeasurer,
        150,
    )
    .expect("compile");
    assert!(result.partition_result.shards.len() > 1);
    assert!(!result.content_hash.is_empty());
}

#[test]
fn material_assigns_highest_diff_level_when_forbidden_and_shared() {
    let mut left = binding(1, "left", Vec::new(), Vec::new());
    left.projection_binding
        .projection
        .scope_policy
        .forbidden_scopes = vec!["shared.rs".to_string()];
    let right = binding(2, "right", Vec::new(), Vec::new());
    let bindings = vec![left, right];
    let snapshots = bindings.iter().map(snapshot).collect::<Vec<_>>();
    let mut git_facts = facts(&bindings);
    for diff in &mut git_facts.completion_diffs {
        diff.patch = "diff --git a/shared.rs b/shared.rs\n@@ -0,0 +1 @@\n+shared\n".to_string();
        diff.file_stats = vec![DiffFileStat {
            path: "shared.rs".to_string(),
            insertions: 1,
            deletions: 0,
        }];
    }
    let result = compile_group_review_material(
        &bindings,
        &snapshots,
        &request(),
        &git_facts,
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    assert!(
        result.diff_index.shard_selections[0]
            .fragments
            .iter()
            .any(|fragment| fragment.path == "shared.rs" && fragment.level == 'A')
    );
}

#[test]
fn material_includes_cross_shard_shared_file_as_b_in_reduction_selection() {
    let bindings = (0..5)
        .map(|index| binding(index, &format!("cross-{index}"), Vec::new(), Vec::new()))
        .collect::<Vec<_>>();
    let snapshots = bindings.iter().map(snapshot).collect::<Vec<_>>();
    let mut git_facts = facts(&bindings);
    for diff in &mut git_facts.completion_diffs {
        diff.patch = "diff --git a/shared.rs b/shared.rs\n@@ -0,0 +1 @@\n+shared\n".to_string();
        diff.file_stats = vec![DiffFileStat {
            path: "shared.rs".to_string(),
            insertions: 1,
            deletions: 0,
        }];
    }
    let result = compile_group_review_material(
        &bindings,
        &snapshots,
        &request(),
        &git_facts,
        &FixedMeasurer,
        1_000,
    )
    .expect("compile");
    assert_eq!(result.partition_result.shards.len(), 2);
    assert!(
        result
            .diff_index
            .reduction_selection
            .fragments
            .iter()
            .any(|fragment| fragment.path == "shared.rs" && fragment.level == 'B')
    );
}
