#![allow(dead_code)]

use super::group_review_material::{GroupReviewMaterialSnapshotDraft, ShardPromptMeasurer};
use super::group_review_types::{
    GroupReviewMaterialSnapshot, GroupShardSpec, PromptSegments, SelectedDiffFragment,
};
use crate::product::coding_models::GroupReviewShardReport;

const VERDICT_MARKER: &str = "GROUP_REVIEW_VERDICT: approve|request_changes|blocked";

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
             Output MUST include the exact marker line `{VERDICT_MARKER}`.\n\
             Review only the supplied shard materials and report concrete, attributable findings.\n",
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
    PromptSegments {
        fixed_protocol: format!(
            "You are the group-review reduction reviewer.\n\
             Merge shard conclusions into one unique final conclusion (唯一最终结论).\n\
             Resolve duplicates deterministically, preserve unresolved obligations, and do not invent evidence.\n\
             Output MUST include the exact marker line `{VERDICT_MARKER}`.\n"
        ),
        identity: format!(
            "snapshot_hash: {}\nattempt_id: {}\nreview_request_id: {}\nbase_branch: {}\nfinal_commit: {}\n",
            snapshot.content_hash,
            snapshot.attempt_id,
            snapshot.review_request_id,
            snapshot.base_branch,
            snapshot.final_commit,
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
    PromptSegments {
        fixed_protocol: format!(
            "Only repair the supplied raw output into the required conclusion format.\n\
             Output MUST include the exact marker line `{VERDICT_MARKER}`.\n\
             只修复输出格式；不得重新审查，不得添加任何新的事实、分析或 findings。\n"
        ),
        identity: String::new(),
        unit_records: String::new(),
        evidence_digest: String::new(),
        graph: String::new(),
        diff: raw_output.to_string(),
        retry_diagnostic_reserve: String::new(),
    }
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
