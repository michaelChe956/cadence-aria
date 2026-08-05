use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CasOutcome, CodingExecutionAttempt, GroupReviewObligation, GroupReviewReductionReport,
    GroupReviewShardReport, ReviewVerdict,
};

use super::group_review_errors::GroupReviewOrchestrationError;
use super::group_review_types::{GroupReviewMaterialSnapshot, GroupShardSpec};

pub(crate) fn persist_shard_failure_report(
    store: &CodingAttemptStore,
    snapshot: &GroupReviewMaterialSnapshot,
    shard: &GroupShardSpec,
    attempt: &CodingExecutionAttempt,
    role_run_id: &str,
    run_failure_code: &str,
) -> Result<(), GroupReviewOrchestrationError> {
    let selected_diff_refs = snapshot
        .diff_index
        .shard_selections
        .iter()
        .find(|selection| selection.shard_id == shard.shard_id)
        .map(|selection| {
            selection
                .fragments
                .iter()
                .map(|fragment| fragment.hunk_content_hash.clone())
                .collect()
        })
        .unwrap_or_default();
    let report = GroupReviewShardReport {
        id: format!("group_review_shard_{}", shard.shard_id),
        attempt_id: snapshot.attempt_id.clone(),
        snapshot_hash: snapshot.content_hash.clone(),
        shard_id: shard.shard_id.clone(),
        ordered_unit_run_ids: shard.ordered_unit_run_ids.clone(),
        partition_rationale: shard.partition_rationale.clone(),
        verdict: ReviewVerdict::Blocked,
        findings: Vec::new(),
        unresolved_obligations: Vec::<GroupReviewObligation>::new(),
        selected_diff_refs,
        raw_provider_output_refs: Vec::new(),
        role_run_ids: vec![role_run_id.to_string()],
        run_failure_code: Some(run_failure_code.to_string()),
    };
    match store.write_group_review_shard_report_cas(&attempt.id, report)? {
        CasOutcome::Written => Ok(()),
        CasOutcome::StoredStale => Err(GroupReviewOrchestrationError::ShardStaleAudit),
    }
}

pub(crate) fn persist_reduction_failure_report(
    store: &CodingAttemptStore,
    snapshot: &GroupReviewMaterialSnapshot,
    shard_reports: &[GroupReviewShardReport],
    attempt: &CodingExecutionAttempt,
    role_run_id: &str,
    run_failure_code: &str,
) -> Result<(), GroupReviewOrchestrationError> {
    let report = GroupReviewReductionReport {
        id: "group_review_reduction_0001".to_string(),
        attempt_id: snapshot.attempt_id.clone(),
        snapshot_hash: snapshot.content_hash.clone(),
        shard_report_ids: shard_reports
            .iter()
            .map(|report| report.id.clone())
            .collect(),
        verdict: ReviewVerdict::Blocked,
        findings: Vec::new(),
        impact_scope: Vec::new(),
        pr_description: String::new(),
        commit_message_suggestion: String::new(),
        provenance: Vec::new(),
        raw_provider_output_refs: Vec::new(),
        role_run_ids: vec![role_run_id.to_string()],
        run_failure_code: Some(run_failure_code.to_string()),
    };
    match store.write_group_review_reduction_report_cas(&attempt.id, report)? {
        CasOutcome::Written => Ok(()),
        CasOutcome::StoredStale => Err(GroupReviewOrchestrationError::ReductionStale),
    }
}
