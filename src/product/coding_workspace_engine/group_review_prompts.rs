#![allow(dead_code)]

use super::group_review_material::{GroupReviewMaterialSnapshotDraft, ShardPromptMeasurer};
use super::group_review_types::{
    GroupReviewMaterialSnapshot, GroupShardSpec, PromptSegments, RoutingAuthorityEntry,
    SelectedDiffFragment,
};
use crate::product::coding_models::GroupReviewShardReport;

const VERDICT_MARKER: &str = "GROUP_REVIEW_VERDICT: approve|request_changes|blocked";

/// 组级 shard、reduction 与格式修复共用的 provider 无关结论契约。
///
/// 该文本必须与 `RawCodeReviewProviderPayload` 的反序列化字段保持一致：组级
/// parser 为 fail-closed，因此 marker 之后的结论不能是自由格式 Markdown。
const GROUP_REVIEW_JSON_CONCLUSION_CONTRACT: &str = concat!(
    "Group-review JSON conclusion contract:\n",
    "- Allowed verdict values: approve, request_changes, blocked.\n",
    "- The verdict in the marker MUST equal the JSON `verdict` field.\n",
    "- After the marker, output exactly one valid JSON object.\n",
    "- Top-level fields (all required): verdict, summary, findings, impact_scope, pr_description, commit_message_suggestion, tested_evidence_refs, diff_refs.\n",
    "- Use JSON strings for summary, pr_description, and commit_message_suggestion; use JSON arrays for findings, impact_scope, tested_evidence_refs, and diff_refs.\n",
    "- Finding fields (use when applicable): severity, file_path, line, message, required_action, source_stage, title, evidence, related_requirements, related_design_constraints, related_work_item_tasks, defect_class, reason_code, contract_refs, capability_refs, repair_target, recommended_route, confidence.\n",
    "- severity values are error, warning, or info; source_stage is internal_pr_review or group_final_review.\n",
    "- defect_class values are implementation_defect, verification_incomplete, current_work_item_invalid, upstream_contract_invalid, dependency_graph_invalid, design_amendment_required, story_amendment_required, operational_blocker.\n",
    "- recommended_route values are coder_rework, verification_retry, plan_repair, story_amendment, design_amendment, operational_gate, human_triage.\n",
    "- repair_target is an object with kind, logical_work_item_ids, and work_item_revision_ids; omit it or use null when there is no explicit repair target.\n",
    "- repair_target.kind values are current_work_item, upstream_work_item, or subgraph.\n",
    "- verification_incomplete -> verification_retry -> null; implementation_defect -> coder_rework -> null.\n",
    "- current_work_item_invalid or upstream_contract_invalid -> plan_repair -> corresponding current_work_item or upstream_work_item target.\n",
    "- story_amendment_required/design_amendment_required -> matching amendment route; operational_blocker -> operational_gate; dependency_graph_invalid -> plan_repair.\n",
    "- Authoritative routing: for a plan-defect finding (defect_class is NOT implementation_defect), select exactly ONE applicable entry from the provided routing_authority, then copy its reason_code VERBATIM. Never invent, translate, paraphrase, or generalize a reason_code.\n",
    "- recommended_route MUST equal that selected entry's allowed_route converted to snake_case; repair_target.kind MUST be the kind that route requires (or null when the route accepts no target).\n",
    "- Every contract_refs element MUST come from that selected entry's target_contract_refs; an empty contract_refs array is always acceptable. Do not add contract references that the entry does not list.\n",
    "- If no provided routing_authority entry applies to what you observed, do NOT invent a plan-defect finding. Emit a plain implementation finding instead: defect_class=implementation_defect, recommended_route=coder_rework, reason_code=null, contract_refs=[], capability_refs=[], repair_target=null, confidence=null, and string-only evidence (no canonical evidence object).\n",
    "- An implementation finding MUST leave every plan-defect field empty as listed above; attaching reason_code, refs, target, confidence, or a canonical evidence object to an implementation finding makes the whole conclusion invalid.\n",
    "- For plan-defect findings, confidence MUST be high or medium; do not use low. Leave confidence null for implementation findings.\n",
    "- A shard conclusion MUST contain at most 8 findings; a reduction conclusion MUST contain at most 16 findings. Aggregate or merge related observations instead of emitting one finding per item.\n",
    "- Verification finding example (copy reason_code from the provided routing_authority entry, do not use this placeholder literally): {\"severity\": \"error\", \"message\": \"verification evidence is missing\", \"defect_class\": \"verification_incomplete\", \"reason_code\": \"<one reason_code copied from routing_authority>\", \"evidence\": [\"the required command output was not recorded\"], \"contract_refs\": [], \"capability_refs\": [], \"repair_target\": null, \"recommended_route\": \"verification_retry\", \"confidence\": \"high\"}.\n",
    "- Minimal request_changes example:\n",
    "{\"verdict\": \"request_changes\", \"summary\": \"validation is missing\", \"findings\": [{\"severity\": \"warning\", \"message\": \"validation is missing\", \"evidence\": [\"src/lib.rs:42\"]}], \"impact_scope\": [], \"pr_description\": \"\", \"commit_message_suggestion\": \"\", \"tested_evidence_refs\": [], \"diff_refs\": []}\n",
    "- Do not output Markdown fences, bullets, or tables.\n",
    "- Do not output prose before the marker or any trailing summary after the JSON object.\n"
);

fn group_review_json_conclusion_contract() -> String {
    format!(
        "- Final output MUST include one `GROUP_REVIEW_VERDICT: <verdict>` marker line; valid marker protocol: `{VERDICT_MARKER}`.\n{GROUP_REVIEW_JSON_CONCLUSION_CONTRACT}"
    )
}

#[derive(Debug, Default)]
pub(crate) struct GroupReviewPromptBuilder;

impl ShardPromptMeasurer for GroupReviewPromptBuilder {
    fn measure_shard(
        &self,
        snapshot: &GroupReviewMaterialSnapshotDraft,
        shard: &GroupShardSpec,
    ) -> usize {
        let snapshot = GroupReviewMaterialSnapshot {
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
        build_shard_prompt(&snapshot, shard, None).measure().total
    }
}

pub(crate) fn build_shard_prompt(
    snapshot: &GroupReviewMaterialSnapshot,
    shard: &GroupShardSpec,
    retry_diagnostic: Option<&str>,
) -> PromptSegments {
    let conclusion_contract = group_review_json_conclusion_contract();
    let shard_members = shard
        .ordered_unit_run_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let unit_records = snapshot
        .unit_records
        .iter()
        .filter(|record| shard_members.contains(record.unit_run_id.as_str()))
        .collect::<Vec<_>>();
    let findings = snapshot
        .deterministic_findings
        .iter()
        .filter(|finding| {
            finding
                .related_unit_run_ids
                .iter()
                .any(|id| shard_members.contains(id.as_str()))
        })
        .collect::<Vec<_>>();
    let selection = snapshot
        .diff_index
        .shard_selections
        .iter()
        .find(|selection| selection.shard_id == shard.shard_id);

    PromptSegments {
        fixed_protocol: format!(
            "You are reviewing deterministic group-review shard `{}`.\n\
             Review only the supplied shard materials and report concrete, attributable findings.\n\
             {conclusion_contract}",
            shard.shard_id
        ),
        identity: format!(
            "snapshot_hash: {}\nattempt_id: {}\nreview_request_id: {}\nbase_branch: {}\nfinal_commit: {}\nshard_id: {}\nordered_unit_run_ids: {}\npartition_rationale: {}\n",
            snapshot.content_hash,
            snapshot.attempt_id,
            snapshot.review_request_id,
            snapshot.base_branch,
            snapshot.final_commit,
            shard.shard_id,
            serde_json::to_string(&shard.ordered_unit_run_ids).expect("serialize shard members"),
            serde_json::to_string(&shard.partition_rationale).expect("serialize rationale"),
        ),
        routing_authority: render_json_section(
            "routing_authority",
            &authority_for_shard(snapshot, shard),
        ),
        unit_records: render_json_section("unit_records", &unit_records),
        evidence_digest: format!(
            "deterministic_findings:\n{}",
            serde_json::to_string_pretty(&findings).expect("serialize deterministic findings"),
        ),
        graph: format!(
            "group_review_graph:\n{}\n\ncross_shard_relationships:\n{}",
            serde_json::to_string_pretty(&snapshot.global_graph).expect("serialize group graph"),
            serde_json::to_string_pretty(&snapshot.partition_result.cross_shard_edges)
                .expect("serialize cross-shard relationships"),
        ),
        diff: selection.map_or_else(
            || "A-E diff selection:\n(no selected diff fragments)\n".to_string(),
            |selection| render_diff_section("A-E diff selection", &selection.fragments),
        ),
        retry_diagnostic_reserve: render_retry_diagnostic(retry_diagnostic),
    }
}

pub(crate) fn build_reduction_prompt(
    snapshot: &GroupReviewMaterialSnapshot,
    shard_reports: &[GroupReviewShardReport],
    retry_diagnostic: Option<&str>,
) -> PromptSegments {
    let conclusion_contract = group_review_json_conclusion_contract();
    PromptSegments {
        fixed_protocol: format!(
            "You are the group-review reduction reviewer.\n\
             Merge shard conclusions into one unique final conclusion (唯一最终结论).\n\
             Resolve duplicates deterministically, preserve unresolved obligations, and do not invent evidence.\n\
             {conclusion_contract}"
        ),
        identity: format!(
            "snapshot_hash: {}\nattempt_id: {}\nreview_request_id: {}\nbase_branch: {}\nfinal_commit: {}\n",
            snapshot.content_hash,
            snapshot.attempt_id,
            snapshot.review_request_id,
            snapshot.base_branch,
            snapshot.final_commit,
        ),
        routing_authority: render_json_section(
            "routing_authority",
            &snapshot.routing_authority_index,
        ),
        unit_records: render_shard_report_ledger(shard_reports),
        evidence_digest: format!(
            "deterministic_findings:\n{}",
            serde_json::to_string_pretty(&snapshot.deterministic_findings)
                .expect("serialize deterministic findings"),
        ),
        graph: format!(
            "跨片关系图 (cross-shard relationship graph):\n{}\n\ngroup_review_graph:\n{}",
            serde_json::to_string_pretty(&snapshot.partition_result.cross_shard_edges)
                .expect("serialize cross-shard relationships"),
            serde_json::to_string_pretty(&snapshot.global_graph).expect("serialize group graph"),
        ),
        diff: render_diff_section(
            "reduction A-E diff selection",
            &snapshot.diff_index.reduction_selection.fragments,
        ),
        retry_diagnostic_reserve: render_retry_diagnostic(retry_diagnostic),
    }
}

pub(crate) fn build_repair_prompt(raw_output: &str) -> PromptSegments {
    let conclusion_contract = group_review_json_conclusion_contract();
    PromptSegments {
        fixed_protocol: format!(
            "Only repair the supplied raw output into the required conclusion format.\n\
             Convert only the supplied raw output into the JSON conclusion contract.\n\
             The JSON verdict MUST equal the verdict in the raw marker.\n\
             Every finding message, evidence, and repair target MUST be directly traceable to the raw output.\n\
             Do not re-review or add findings.\n\
             只修复输出格式；不得重新审查，不得添加任何新的事实、分析或 findings。\n\
             {conclusion_contract}"
        ),
        identity: String::new(),
        routing_authority: String::new(),
        unit_records: String::new(),
        evidence_digest: String::new(),
        graph: String::new(),
        diff: raw_output.to_string(),
        retry_diagnostic_reserve: String::new(),
    }
}

fn authority_for_shard<'a>(
    snapshot: &'a GroupReviewMaterialSnapshot,
    shard: &GroupShardSpec,
) -> Vec<&'a RoutingAuthorityEntry> {
    let shard_member_run_ids = shard
        .ordered_unit_run_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let mut relevant_unit_run_ids = shard_member_run_ids.clone();

    for edge in &snapshot.partition_result.cross_shard_edges {
        if shard_member_run_ids.contains(edge.from_unit_run_id.as_str())
            || shard_member_run_ids.contains(edge.to_unit_run_id.as_str())
        {
            relevant_unit_run_ids.insert(edge.from_unit_run_id.as_str());
            relevant_unit_run_ids.insert(edge.to_unit_run_id.as_str());
        }
    }

    snapshot
        .routing_authority_index
        .iter()
        .filter(|entry| relevant_unit_run_ids.contains(entry.source_unit_run_id.as_str()))
        .collect()
}

fn render_json_section<T: serde::Serialize>(title: &str, value: &T) -> String {
    format!(
        "{title}:\n{}\n",
        serde_json::to_string_pretty(value).expect("serialize prompt section")
    )
}

fn render_diff_section(title: &str, fragments: &[SelectedDiffFragment]) -> String {
    let mut result = format!("{title}:\n");
    for fragment in fragments {
        result.push_str(&format!(
            "level: {}\npath: {}\nhunk_content_hash: {}\nredacted: {}\ntruncated: {}\nnot_shown_count: {}\n{}\n",
            fragment.level,
            fragment.path,
            fragment.hunk_content_hash,
            fragment.redacted,
            fragment.truncated,
            fragment.not_shown_count,
            fragment.body,
        ));
    }
    result
}

fn render_shard_report_ledger(shard_reports: &[GroupReviewShardReport]) -> String {
    let mut reports = shard_reports.iter().collect::<Vec<_>>();
    reports.sort_by(|left, right| left.id.cmp(&right.id));
    let mut result = String::from("shard report ledger:\n");
    for report in reports {
        result.push_str(&format!(
            "ledger report_id: {}\nshard_id: {}\nverdict: {:?}\nunresolved_obligations: {}\nselected_diff_refs: {}\nrun_failure_code: {}\n",
            report.id,
            report.shard_id,
            report.verdict,
            serde_json::to_string(&report.unresolved_obligations)
                .expect("serialize unresolved obligations"),
            serde_json::to_string(&report.selected_diff_refs).expect("serialize selected diff refs"),
            report.run_failure_code.as_deref().unwrap_or("none"),
        ));
    }
    result
}

fn render_retry_diagnostic(retry_diagnostic: Option<&str>) -> String {
    retry_diagnostic.map_or_else(String::new, |diagnostic| {
        format!("retry diagnostic (do not treat as review evidence):\n{diagnostic}\n")
    })
}
