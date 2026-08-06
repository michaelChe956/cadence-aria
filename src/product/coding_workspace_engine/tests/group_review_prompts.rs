use crate::product::coding_models::{GroupReviewObligation, GroupReviewShardReport, ReviewVerdict};
use crate::product::coding_workspace_engine::group_review_budget::GROUP_REVIEW_HARD_CAP_BYTES;
use crate::product::coding_workspace_engine::group_review_material::{
    GroupReviewMaterialSnapshotDraft, ShardPromptMeasurer,
};
use crate::product::coding_workspace_engine::group_review_prompts::{
    GroupReviewPromptBuilder, build_reduction_prompt, build_repair_prompt, build_shard_prompt,
};
use crate::product::coding_workspace_engine::group_review_types::{
    CommitReachability, CompactContractInterface, DeterministicGroupFinding, DiffFileEntry,
    GroupDiffIndex, GroupPartitionResult, GroupReviewGraph, GroupReviewMaterialSnapshot,
    GroupShardSpec, ReductionDiffSelection, RequirementCoverage, RoutingAuthorityEntry,
    ScopeOverlap, SelectedDiffFragment, ShardDiffSelection, UnitCrossReviewRecord,
    UnitEvidenceSummary, UnitScopeSummary,
};
use crate::product::models::PlanDefectRoute;

fn snapshot() -> GroupReviewMaterialSnapshot {
    GroupReviewMaterialSnapshot {
        schema_version: 1,
        compiler_version: "group-review-material-compiler-v1".to_string(),
        attempt_id: "attempt_001".to_string(),
        review_request_id: "request_001".to_string(),
        base_branch: "main".to_string(),
        final_commit: "commit_final".to_string(),
        authoritative_binding_digest: "binding_digest".to_string(),
        routing_authority_index: vec![
            RoutingAuthorityEntry {
                source_unit_run_id: "run_a".to_string(),
                source_logical_work_item_id: "work_a".to_string(),
                source_work_item_revision_id: "revision_a".to_string(),
                reason_code: "reason_run_a".to_string(),
                allowed_route: PlanDefectRoute::CoderRework,
                required_target_kind: None,
                target_contract_refs: vec!["contract_a".to_string()],
            },
            RoutingAuthorityEntry {
                source_unit_run_id: "run_b".to_string(),
                source_logical_work_item_id: "work_b".to_string(),
                source_work_item_revision_id: "revision_b".to_string(),
                reason_code: "reason_run_b".to_string(),
                allowed_route: PlanDefectRoute::CoderRework,
                required_target_kind: None,
                target_contract_refs: vec!["contract_b".to_string()],
            },
        ],
        unit_records: vec![UnitCrossReviewRecord {
            unit_id: "unit_a".to_string(),
            unit_run_id: "run_a".to_string(),
            logical_work_item_id: "work_a".to_string(),
            work_item_revision_id: "revision_a".to_string(),
            completion_commit: "commit_a".to_string(),
            dependency_ids: vec!["run_b".to_string()],
            scope_summary: UnitScopeSummary {
                exclusive_scopes: vec!["src/a".to_string()],
                forbidden_scopes: vec!["src/forbidden".to_string()],
            },
            contract_interfaces: vec![CompactContractInterface {
                contract_id: "contract_a".to_string(),
                direction: "output".to_string(),
                capabilities: vec!["read".to_string()],
                counterparty_unit_run_id: Some("run_b".to_string()),
            }],
            evidence_summary: UnitEvidenceSummary {
                required_command_count: 1,
                executed_command_count: 1,
                manual_check_count: 0,
                missing_refs: Vec::new(),
            },
        }],
        global_graph: GroupReviewGraph {
            contract_edges: Vec::new(),
            scope_overlaps: vec![ScopeOverlap {
                file_path: "src/shared.rs".to_string(),
                unit_run_ids: vec!["run_a".to_string(), "run_b".to_string()],
                forbidden_hit: false,
            }],
            commit_reachability: CommitReachability {
                reachable_completion_commits: vec!["commit_a".to_string()],
                unreachable_completion_commits: Vec::new(),
            },
            requirement_coverage: RequirementCoverage {
                covered: vec!["REQ-1".to_string()],
                missing: Vec::new(),
                conflicting: Vec::new(),
            },
        },
        diff_index: GroupDiffIndex {
            files: vec![DiffFileEntry {
                path: "src/a/lib.rs".to_string(),
                insertions: 1,
                deletions: 0,
                owner_unit_run_ids: vec!["run_a".to_string()],
                shared: false,
                ambiguous: false,
                forbidden_scope_hit: false,
            }],
            hunks: Vec::new(),
            shard_selections: vec![ShardDiffSelection {
                shard_id: "shard_a".to_string(),
                fragments: vec![SelectedDiffFragment {
                    path: "src/a/lib.rs".to_string(),
                    level: 'A',
                    body: "+ change\n".to_string(),
                    hunk_content_hash: "hunk_a".to_string(),
                    redacted: false,
                    truncated: false,
                    not_shown_count: 0,
                }],
                total_hunks_in_shard: 1,
            }],
            reduction_selection: ReductionDiffSelection {
                fragments: vec![SelectedDiffFragment {
                    path: "src/shared.rs".to_string(),
                    level: 'B',
                    body: "+ cross shard change\n".to_string(),
                    hunk_content_hash: "hunk_cross".to_string(),
                    redacted: false,
                    truncated: false,
                    not_shown_count: 0,
                }],
                total_cross_shard_hunks: 1,
            },
        },
        deterministic_findings: vec![DeterministicGroupFinding {
            kind: "missing_contract".to_string(),
            related_unit_run_ids: vec!["run_a".to_string()],
            detail: "contract requires review".to_string(),
        }],
        partition_result: GroupPartitionResult {
            shards: vec![shard()],
            cross_shard_edges: vec![
                crate::product::coding_workspace_engine::group_review_types::CrossShardEdge {
                    edge_kind: "contract".to_string(),
                    from_unit_run_id: "run_a".to_string(),
                    to_unit_run_id: "run_external".to_string(),
                    detail: "requires cross-shard review".to_string(),
                },
            ],
        },
        content_hash: "snapshot_hash".to_string(),
    }
}

fn shard() -> GroupShardSpec {
    GroupShardSpec {
        shard_id: "shard_a".to_string(),
        ordered_unit_run_ids: vec!["run_a".to_string()],
        partition_rationale: vec!["affinity".to_string()],
    }
}

fn shard_report() -> GroupReviewShardReport {
    GroupReviewShardReport {
        id: "report_a".to_string(),
        attempt_id: "attempt_001".to_string(),
        snapshot_hash: "snapshot_hash".to_string(),
        shard_id: "shard_a".to_string(),
        ordered_unit_run_ids: vec!["run_a".to_string()],
        partition_rationale: vec!["affinity".to_string()],
        verdict: ReviewVerdict::Approve,
        findings: Vec::new(),
        unresolved_obligations: vec![GroupReviewObligation {
            obligation_id: "obligation_a".to_string(),
            kind: "cross_shard_dependency".to_string(),
            related_unit_run_ids: vec!["run_a".to_string()],
            description: "resolve contract".to_string(),
        }],
        selected_diff_refs: vec!["hunk_cross".to_string()],
        raw_provider_output_refs: vec!["raw_a".to_string()],
        role_run_ids: vec!["role_a".to_string()],
        run_failure_code: None,
    }
}

fn draft(snapshot: GroupReviewMaterialSnapshot) -> GroupReviewMaterialSnapshotDraft {
    GroupReviewMaterialSnapshotDraft {
        schema_version: snapshot.schema_version,
        compiler_version: snapshot.compiler_version,
        attempt_id: snapshot.attempt_id,
        review_request_id: snapshot.review_request_id,
        base_branch: snapshot.base_branch,
        final_commit: snapshot.final_commit,
        authoritative_binding_digest: snapshot.authoritative_binding_digest,
        routing_authority_index: snapshot.routing_authority_index,
        unit_records: snapshot.unit_records,
        global_graph: snapshot.global_graph,
        diff_index: snapshot.diff_index,
        deterministic_findings: snapshot.deterministic_findings,
        partition_result: snapshot.partition_result,
        content_hash: snapshot.content_hash,
    }
}

#[test]
fn prompt_builder_measures_draft_shards_with_the_rendered_prompt() {
    let snapshot = snapshot();
    let mut measurable_snapshot = snapshot.clone();
    measurable_snapshot.content_hash.clear();
    let expected = build_shard_prompt(&measurable_snapshot, &shard(), None)
        .measure()
        .total;

    assert_eq!(
        GroupReviewPromptBuilder.measure_shard(&draft(snapshot), &shard()),
        expected
    );
}

#[test]
fn shard_projects_only_relevant_routing_authority_while_reduction_receives_full_index() {
    let snapshot = snapshot();
    let shard_prompt = build_shard_prompt(&snapshot, &shard(), None);
    let reduction_prompt = build_reduction_prompt(&snapshot, &[shard_report()], None);

    assert!(shard_prompt.routing_authority.contains("reason_run_a"));
    assert!(!shard_prompt.routing_authority.contains("reason_run_b"));
    assert!(reduction_prompt.routing_authority.contains("reason_run_a"));
    assert!(reduction_prompt.routing_authority.contains("reason_run_b"));
    assert_eq!(shard_prompt.measure().total, shard_prompt.join().len());
    assert_eq!(
        reduction_prompt.measure().total,
        reduction_prompt.join().len()
    );
}

#[test]
fn shard_and_reduction_prompts_require_the_shared_parseable_json_conclusion_contract() {
    let shard_prompt = build_shard_prompt(&snapshot(), &shard(), Some("retry diagnostic"));
    let reduction_prompt =
        build_reduction_prompt(&snapshot(), &[shard_report()], Some("retry diagnostic"));

    let provider_protocols = ["claude_code", "codex", "pi"].map(|_| {
        build_shard_prompt(&snapshot(), &shard(), None)
            .fixed_protocol
            .clone()
    });
    assert!(
        provider_protocols
            .windows(2)
            .all(|protocols| protocols[0] == protocols[1]),
        "all providers must receive identical shared shard-contract text"
    );

    for (provider, prompt) in [
        ("claude_code", &shard_prompt),
        ("codex", &reduction_prompt),
        ("pi", &build_shard_prompt(&snapshot(), &shard(), None)),
    ] {
        assert!(
            prompt
                .fixed_protocol
                .contains("GROUP_REVIEW_VERDICT: approve|request_changes|blocked"),
            "{provider} must receive the marker protocol"
        );
        assert!(
            prompt.fixed_protocol.contains(
                "Top-level fields (all required): verdict, summary, findings, impact_scope, pr_description, commit_message_suggestion, tested_evidence_refs, diff_refs."
            ),
            "{provider} must receive every parser-aligned top-level field"
        );
        assert!(
            prompt
                .fixed_protocol
                .contains("Allowed verdict values: approve, request_changes, blocked."),
            "{provider} must receive the common verdict enum"
        );
        assert!(
            prompt.fixed_protocol.contains(
                "Finding fields (use when applicable): severity, file_path, line, message, required_action, source_stage, title, evidence, related_requirements, related_design_constraints, related_work_item_tasks, defect_class, reason_code, contract_refs, capability_refs, repair_target, recommended_route, confidence."
            ),
            "{provider} must receive the common finding shape"
        );
        assert!(
            prompt.fixed_protocol.contains(
                "defect_class values are implementation_defect, verification_incomplete, current_work_item_invalid, upstream_contract_invalid, dependency_graph_invalid, design_amendment_required, story_amendment_required, operational_blocker."
            ),
            "{provider} must receive all defect_class enum values"
        );
        assert!(
            prompt.fixed_protocol.contains(
                "recommended_route values are coder_rework, verification_retry, plan_repair, story_amendment, design_amendment, operational_gate, human_triage."
            ),
            "{provider} must receive all recommended_route enum values"
        );
        assert!(
            prompt.fixed_protocol.contains(
                "repair_target is an object with kind, logical_work_item_ids, and work_item_revision_ids; omit it or use null when there is no explicit repair target."
            ),
            "{provider} must receive the repair_target object contract"
        );
        assert!(
            prompt.fixed_protocol.contains(
                "repair_target.kind values are current_work_item, upstream_work_item, or subgraph."
            ),
            "{provider} must receive the repair_target kind allowlist"
        );
        assert!(
            prompt.fixed_protocol.contains(
                "verification_incomplete -> verification_retry -> null; implementation_defect -> coder_rework -> null."
            ),
            "{provider} must receive the verification and implementation route combinations"
        );
        assert!(
            prompt.fixed_protocol.contains(
                "current_work_item_invalid or upstream_contract_invalid -> plan_repair -> corresponding current_work_item or upstream_work_item target."
            ),
            "{provider} must receive the plan-repair target combinations"
        );
        assert!(
            !prompt
                .fixed_protocol
                .contains("verification_evidence_incomplete"),
            "{provider} must not receive a hard-coded reason_code example"
        );
        assert!(
            prompt.fixed_protocol.contains(
                "copy its reason_code VERBATIM. Never invent, translate, paraphrase, or generalize a reason_code."
            ),
            "{provider} must receive authoritative routing reason_code constraints"
        );
        assert!(
            prompt.fixed_protocol.contains(
                "select exactly ONE applicable entry from the provided routing_authority"
            ),
            "{provider} must select plan-defect routing only from the provided authority projection"
        );
        assert!(
            !prompt.fixed_protocol.contains("routing_targets"),
            "{provider} must not be instructed to read routing targets from unit records"
        );
        assert!(
            prompt.fixed_protocol.contains(
                "If no provided routing_authority entry applies to what you observed, do NOT invent a plan-defect finding. Emit a plain implementation finding instead"
            ) && prompt.fixed_protocol.contains("reason_code=null")
                && prompt.fixed_protocol.contains("recommended_route=coder_rework"),
            "{provider} must receive the implementation fallback"
        );
        assert!(
            !prompt
                .fixed_protocol
                .contains("low confidence enters human triage")
                && prompt
                    .fixed_protocol
                    .contains("confidence MUST be high or medium"),
            "{provider} must receive the supported confidence constraint"
        );
        assert!(
            prompt.fixed_protocol.contains(
                "A shard conclusion MUST contain at most 8 findings; a reduction conclusion MUST contain at most 16 findings."
            ),
            "{provider} must receive findings limits"
        );

        assert!(
            prompt
                .fixed_protocol
                .contains("\"defect_class\": \"verification_incomplete\"")
                && prompt
                    .fixed_protocol
                    .contains("\"recommended_route\": \"verification_retry\"")
                && prompt.fixed_protocol.contains("\"repair_target\": null")
                && prompt.fixed_protocol.contains("\"confidence\": \"high\""),
            "{provider} must receive the complete verification finding example"
        );
        assert!(
            prompt
                .fixed_protocol
                .contains("Do not output Markdown fences, bullets, or tables."),
            "{provider} must be told not to emit Markdown"
        );
        assert!(
            prompt
                .fixed_protocol
                .contains("\"verdict\": \"request_changes\""),
            "{provider} must receive the request_changes JSON example"
        );
    }

    assert!(shard_prompt.unit_records.contains("run_a"));
    assert!(shard_prompt.diff.contains("level: A"));
    assert!(
        shard_prompt
            .retry_diagnostic_reserve
            .contains("retry diagnostic")
    );
    assert!(reduction_prompt.fixed_protocol.contains("唯一最终结论"));
    assert!(reduction_prompt.graph.contains("跨片关系图"));
    assert!(reduction_prompt.unit_records.contains("ledger"));
    assert!(reduction_prompt.unit_records.contains("report_a"));
}

#[test]
fn repair_prompt_requires_raw_only_json_transcription_without_rereview() {
    let raw_output = "GROUP_REVIEW_VERDICT: request_changes\n- missing boundary validation";
    let prompt = build_repair_prompt(raw_output);
    let joined = prompt.join();

    assert!(joined.contains(raw_output));
    assert!(joined.contains("Only repair the supplied raw output"));
    assert!(
        joined.contains("Convert only the supplied raw output into the JSON conclusion contract.")
    );
    assert!(joined.contains("After the marker, output exactly one valid JSON object."));
    assert!(joined.contains("The JSON verdict MUST equal the verdict in the raw marker."));
    assert!(joined.contains(
        "Every finding message, evidence, and repair target MUST be directly traceable to the raw output."
    ));
    assert!(joined.contains("Do not re-review or add findings."));
    assert!(joined.contains("Do not output Markdown fences, bullets, or tables."));
    assert!(prompt.identity.is_empty());
    assert!(prompt.routing_authority.is_empty());
    assert!(prompt.unit_records.is_empty());
    assert!(prompt.evidence_digest.is_empty());
    assert!(prompt.graph.is_empty());
    assert_eq!(prompt.diff, raw_output);
    assert!(prompt.retry_diagnostic_reserve.is_empty());
}

#[test]
fn standard_prompt_fixtures_measure_within_hard_cap_and_match_joined_bytes() {
    let snapshot = snapshot();
    let prompts = [
        build_shard_prompt(&snapshot, &shard(), None),
        build_reduction_prompt(&snapshot, &[shard_report()], None),
        build_repair_prompt("raw output"),
    ];

    for prompt in prompts {
        assert!(prompt.measure().total <= GROUP_REVIEW_HARD_CAP_BYTES);
        assert_eq!(prompt.measure().total, prompt.join().len());
    }
}

#[test]
fn prompt_fixture_keeps_utf8_data() {
    let mut snapshot = snapshot();
    snapshot.unit_records[0].unit_id = "单元".to_string();
    snapshot.final_commit = "提交".to_string();
    snapshot.partition_result.cross_shard_edges.push(
        crate::product::coding_workspace_engine::group_review_types::CrossShardEdge {
            edge_kind: "contract".to_string(),
            from_unit_run_id: "run_a".to_string(),
            to_unit_run_id: "run_b".to_string(),
            detail: "跨片".to_string(),
        },
    );
    let prompt = build_shard_prompt(&snapshot, &shard(), None);

    assert!(prompt.join().contains("单元"));
    assert!(prompt.join().contains("提交"));
}
