use crate::product::coding_models::{GroupReviewObligation, GroupReviewShardReport, ReviewVerdict};
use crate::product::coding_workspace_engine::group_review_budget::GROUP_REVIEW_HARD_CAP_BYTES;
use crate::product::coding_workspace_engine::group_review_material::{
    GroupReviewMaterialSnapshotDraft, ShardPromptMeasurer,
};
use crate::product::coding_workspace_engine::group_review_prompts::{
    GroupReviewPromptBuilder, build_reduction_prompt, build_repair_prompt, build_shard_prompt,
};
use crate::product::coding_workspace_engine::group_review_types::{
    CommitReachability, CompactContractInterface, CompactRoutingTarget, DeterministicGroupFinding,
    DiffFileEntry, GroupDiffIndex, GroupPartitionResult, GroupReviewGraph,
    GroupReviewMaterialSnapshot, GroupShardSpec, ReductionDiffSelection, RequirementCoverage,
    ScopeOverlap, SelectedDiffFragment, ShardDiffSelection, UnitCrossReviewRecord,
    UnitEvidenceSummary, UnitScopeSummary,
};

fn snapshot() -> GroupReviewMaterialSnapshot {
    GroupReviewMaterialSnapshot {
        schema_version: 1,
        compiler_version: "group-review-material-compiler-v1".to_string(),
        attempt_id: "attempt_001".to_string(),
        review_request_id: "request_001".to_string(),
        base_branch: "main".to_string(),
        final_commit: "commit_final".to_string(),
        authoritative_binding_digest: "binding_digest".to_string(),
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
            routing_targets: vec![CompactRoutingTarget {
                reason_code: "contract_gap".to_string(),
                allowed_route: "coder_rework".to_string(),
                target_contract_refs: vec!["contract_a".to_string()],
            }],
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
            cross_shard_edges: Vec::new(),
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
fn shard_prompt_requires_verdict_marker_and_includes_unit_records_and_a_to_e_diff() {
    let prompt = build_shard_prompt(&snapshot(), &shard(), Some("retry diagnostic"));

    assert!(
        prompt
            .fixed_protocol
            .contains("GROUP_REVIEW_VERDICT: approve|request_changes|blocked")
    );
    assert!(prompt.unit_records.contains("run_a"));
    assert!(prompt.diff.contains("level: A"));
    assert!(prompt.retry_diagnostic_reserve.contains("retry diagnostic"));
}

#[test]
fn reduction_prompt_requires_merge_rules_cross_shard_graph_and_ledger_lines() {
    let prompt = build_reduction_prompt(&snapshot(), &[shard_report()], Some("retry diagnostic"));

    assert!(
        prompt
            .fixed_protocol
            .contains("GROUP_REVIEW_VERDICT: approve|request_changes|blocked")
    );
    assert!(prompt.fixed_protocol.contains("唯一最终结论"));
    assert!(prompt.graph.contains("跨片关系图"));
    assert!(prompt.unit_records.contains("ledger"));
    assert!(prompt.unit_records.contains("report_a"));
}

#[test]
fn repair_prompt_contains_only_raw_output_contract_and_no_rereview_instruction() {
    let raw_output = "unparseable GROUP_REVIEW_VERDICT output";
    let prompt = build_repair_prompt(raw_output);
    let joined = prompt.join();

    assert!(joined.contains(raw_output));
    assert!(joined.contains("只修复输出格式"));
    assert!(joined.contains("不得重新审查"));
    assert!(prompt.identity.is_empty());
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
