#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::Utc;
use sha2::{Digest, Sha256};

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::Semaphore;

use super::group_review_budget::{
    BudgetDecision, CapacityDecision, FindingsDecision, GROUP_REVIEW_QUALITY_TARGET_BYTES,
    check_capacity, check_shard_findings, decide_budget, group_review_shard_concurrency,
};
use super::group_review_prompts::build_shard_prompt;
use super::group_review_types::{GroupReviewMaterialSnapshot, PromptBudgetBreakdown};
use super::plan_defect_routing::{GroupReviewerProjectionBinding, validate_group_reviewer_finding};
use super::review_parser::parse_review_payload;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingExecutionStage, GroupReviewObligation, GroupReviewReductionReport,
    GroupReviewShardReport, InternalPrReview, ReviewFinding, ReviewVerdict,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::ProductStoreError;

#[cfg(test)]
use std::collections::VecDeque;
#[cfg(test)]
use std::sync::Mutex;

pub(crate) struct GroupReviewOrchestrator<'a> {
    executor: &'a dyn GroupReviewExecutor,
    store: &'a CodingAttemptStore,
}

impl<'a> GroupReviewOrchestrator<'a> {
    pub(crate) fn new(
        executor: &'a dyn GroupReviewExecutor,
        store: &'a CodingAttemptStore,
    ) -> Self {
        Self { executor, store }
    }

    pub(crate) async fn execute_shards(
        &self,
        snapshot: &GroupReviewMaterialSnapshot,
    ) -> Result<Vec<GroupReviewShardReport>, GroupReviewOrchestrationError> {
        if check_capacity(snapshot.unit_records.len()) == CapacityDecision::CapacityExceeded {
            return Err(GroupReviewOrchestrationError::CapacityExceeded);
        }

        // Do all material validation before starting a provider future: a later overflow must
        // not permit an earlier shard to have already contacted the provider.
        let prompts = snapshot
            .partition_result
            .shards
            .iter()
            .map(|shard| {
                let segments = build_shard_prompt(snapshot, shard, None);
                let breakdown = segments.measure();
                if decide_budget(breakdown.total, GROUP_REVIEW_QUALITY_TARGET_BYTES)
                    == BudgetDecision::Overflow
                {
                    return Err(GroupReviewOrchestrationError::MaterialOverflow { breakdown });
                }
                Ok((shard, segments.join()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let attempt = self.store.find_attempt_by_id(&snapshot.attempt_id)?;
        let semaphore = Arc::new(Semaphore::new(group_review_shard_concurrency()));
        let mut executions = FuturesUnordered::new();
        for (shard, prompt) in prompts {
            let semaphore = semaphore.clone();
            executions.push(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .expect("group review semaphore is never closed");
                let result = self.executor.execute(&prompt).await?;
                Ok::<_, GroupReviewOrchestrationError>((shard, result))
            });
        }

        let mut reports = Vec::new();
        while let Some(result) = executions.next().await {
            let (shard, execution) = result?;
            // Persist raw outputs and reports after the concurrently-executed provider calls.
            // The existing raw-output store sequences files per purpose, so serial persistence
            // prevents independent shards from selecting the same sequence number.
            reports.push(self.build_and_store_shard_report(snapshot, shard, &attempt, execution)?);
        }
        reports.sort_by(|left, right| left.shard_id.cmp(&right.shard_id));
        Ok(reports)
    }

    pub(crate) async fn execute_reduction(
        &self,
        snapshot: &GroupReviewMaterialSnapshot,
        shard_reports: &[GroupReviewShardReport],
        authoritative_bindings: &[GroupReviewerProjectionBinding],
    ) -> Result<GroupReviewReductionReport, GroupReviewOrchestrationError> {
        if !has_all_shard_reports(snapshot, shard_reports) {
            return Err(GroupReviewOrchestrationError::ReductionNotReady);
        }
        let segments =
            super::group_review_prompts::build_reduction_prompt(snapshot, shard_reports, None);
        let breakdown = segments.measure();
        if decide_budget(breakdown.total, GROUP_REVIEW_QUALITY_TARGET_BYTES)
            == BudgetDecision::Overflow
        {
            return Err(GroupReviewOrchestrationError::MaterialOverflow { breakdown });
        }
        let attempt = self.store.find_attempt_by_id(&snapshot.attempt_id)?;
        let execution = self.executor.execute(&segments.join()).await?;
        let raw_ref = self.store.save_provider_raw_output(
            &attempt,
            CodingExecutionStage::InternalPrReview,
            "group_review_reduction",
            &execution.full_output,
        )?;
        let payload = parse_review_payload(
            &execution.full_output,
            CodingExecutionStage::InternalPrReview,
        );
        if super::group_review_budget::check_reduction_findings(payload.findings.len())
            == FindingsDecision::FindingsExceeded
        {
            return Err(GroupReviewOrchestrationError::ReductionOutputInvalid { raw_ref });
        }
        let findings = merge_findings(shard_reports, payload.findings);
        if findings.iter().any(|finding| {
            validate_group_reviewer_finding(finding, authoritative_bindings).is_err()
        }) {
            return Err(GroupReviewOrchestrationError::ReductionOutputInvalid { raw_ref });
        }
        let verdict = reduce_verdict(
            shard_reports
                .iter()
                .map(|report| report.verdict.clone())
                .chain(std::iter::once(payload.verdict)),
        );
        let reduction = GroupReviewReductionReport {
            id: "group_review_reduction_0001".to_string(),
            attempt_id: snapshot.attempt_id.clone(),
            snapshot_hash: snapshot.content_hash.clone(),
            shard_report_ids: shard_reports
                .iter()
                .map(|report| report.id.clone())
                .collect(),
            verdict: verdict.clone(),
            findings: findings.clone(),
            impact_scope: payload.impact_scope.clone(),
            pr_description: payload.pr_description.clone(),
            commit_message_suggestion: payload.commit_message_suggestion.clone(),
            provenance: Vec::new(),
            raw_provider_output_refs: vec![raw_ref.clone()],
            role_run_ids: execution.provider_session_id.into_iter().collect(),
            run_failure_code: None,
        };
        let _ = self
            .store
            .write_group_review_reduction_report_cas(&snapshot.attempt_id, reduction.clone())?;
        let existing = self.store.list_internal_pr_reviews(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let review = InternalPrReview {
            id: next_sequential_id("internal_review", existing.len()),
            attempt_id: snapshot.attempt_id.clone(),
            review_request_id: snapshot.review_request_id.clone(),
            verdict,
            findings,
            impact_scope: payload.impact_scope,
            pr_description: payload.pr_description,
            commit_message_suggestion: payload.commit_message_suggestion,
            tested_evidence_refs: payload.tested_evidence_refs,
            diff_refs: payload.diff_refs,
            summary: payload.summary,
            created_at: Utc::now().to_rfc3339(),
            raw_provider_output_ref: Some(raw_ref),
            role_run_id: None,
            run_no: None,
        };
        self.store.save_internal_pr_review(&attempt, &review)?;
        Ok(reduction)
    }

    fn build_and_store_shard_report(
        &self,
        snapshot: &GroupReviewMaterialSnapshot,
        shard: &super::group_review_types::GroupShardSpec,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        execution: GroupReviewExecutionResult,
    ) -> Result<GroupReviewShardReport, GroupReviewOrchestrationError> {
        let raw_ref = self.store.save_provider_raw_output(
            attempt,
            CodingExecutionStage::InternalPrReview,
            &format!("group_review_{}", shard.shard_id),
            &execution.full_output,
        )?;
        let payload = parse_review_payload(
            &execution.full_output,
            CodingExecutionStage::InternalPrReview,
        );
        if check_shard_findings(payload.findings.len()) == FindingsDecision::FindingsExceeded {
            return Err(GroupReviewOrchestrationError::ShardOutputInvalid {
                shard_id: shard.shard_id.clone(),
                raw_ref,
            });
        }

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
            verdict: payload.verdict,
            findings: payload.findings,
            unresolved_obligations: Vec::<GroupReviewObligation>::new(),
            selected_diff_refs,
            raw_provider_output_refs: vec![raw_ref],
            role_run_ids: execution.provider_session_id.into_iter().collect(),
            run_failure_code: None,
        };
        let _ = self
            .store
            .write_group_review_shard_report_cas(&snapshot.attempt_id, report.clone())?;
        Ok(report)
    }
}

#[async_trait::async_trait]
pub(crate) trait GroupReviewExecutor: Send + Sync {
    async fn execute(
        &self,
        prompt: &str,
    ) -> Result<GroupReviewExecutionResult, GroupReviewExecutionError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GroupReviewExecutionResult {
    pub full_output: String,
    pub provider_session_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GroupReviewExecutionError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("user_cancelled")]
    UserCancelled,
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GroupReviewOrchestrationError {
    #[error("capacity_exceeded")]
    CapacityExceeded,
    #[error("material_overflow")]
    MaterialOverflow { breakdown: PromptBudgetBreakdown },
    #[error("shard_output_invalid: {shard_id}")]
    ShardOutputInvalid { shard_id: String, raw_ref: String },
    #[error("reduction_not_ready")]
    ReductionNotReady,
    #[error("reduction_output_invalid: {raw_ref}")]
    ReductionOutputInvalid { raw_ref: String },
    #[error("store: {0}")]
    Store(#[from] ProductStoreError),
    #[error("executor: {0}")]
    Executor(#[from] GroupReviewExecutionError),
}

#[cfg(test)]
pub(crate) struct FakeGroupReviewExecutor {
    results: Mutex<VecDeque<Result<GroupReviewExecutionResult, GroupReviewExecutionError>>>,
    prompts: Mutex<Vec<String>>,
}

#[cfg(test)]
impl FakeGroupReviewExecutor {
    pub(crate) fn new(
        results: Vec<Result<GroupReviewExecutionResult, GroupReviewExecutionError>>,
    ) -> Self {
        Self {
            results: Mutex::new(results.into()),
            prompts: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn push_result(
        &self,
        result: Result<GroupReviewExecutionResult, GroupReviewExecutionError>,
    ) {
        self.results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(result);
    }

    pub(crate) fn prompts(&self) -> Vec<String> {
        self.prompts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl GroupReviewExecutor for FakeGroupReviewExecutor {
    async fn execute(
        &self,
        prompt: &str,
    ) -> Result<GroupReviewExecutionResult, GroupReviewExecutionError> {
        self.prompts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(prompt.to_string());
        self.results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
            .unwrap_or_else(|| {
                Err(GroupReviewExecutionError::Internal(
                    "fake_group_review_executor_exhausted".to_string(),
                ))
            })
    }
}

fn has_all_shard_reports(
    snapshot: &GroupReviewMaterialSnapshot,
    reports: &[GroupReviewShardReport],
) -> bool {
    let expected = snapshot
        .partition_result
        .shards
        .iter()
        .map(|shard| shard.shard_id.as_str())
        .collect::<BTreeSet<_>>();
    let valid_reports = reports
        .iter()
        .filter(|report| {
            report.attempt_id == snapshot.attempt_id
                && report.snapshot_hash == snapshot.content_hash
                && report.run_failure_code.is_none()
        })
        .collect::<Vec<_>>();
    let actual = valid_reports
        .iter()
        .map(|report| report.shard_id.as_str())
        .collect::<BTreeSet<_>>();
    expected == actual && valid_reports.len() == expected.len()
}

pub(crate) fn reduce_verdict(verdicts: impl IntoIterator<Item = ReviewVerdict>) -> ReviewVerdict {
    verdicts
        .into_iter()
        .max_by_key(|verdict| match verdict {
            ReviewVerdict::Approve => 0,
            ReviewVerdict::RequestChanges => 1,
            ReviewVerdict::Blocked => 2,
        })
        .unwrap_or(ReviewVerdict::Approve)
}

pub(crate) fn finding_fingerprint(finding: &ReviewFinding) -> String {
    let target = finding.repair_target.as_ref();
    let mut target_ids = target
        .map(|target| {
            target
                .logical_work_item_ids
                .iter()
                .chain(target.work_item_revision_ids.iter())
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut contract_refs = finding.contract_refs.clone();
    let mut capability_refs = finding.capability_refs.clone();
    target_ids.sort();
    target_ids.dedup();
    contract_refs.sort();
    contract_refs.dedup();
    capability_refs.sort();
    capability_refs.dedup();
    let path = finding
        .file_path
        .as_deref()
        .unwrap_or("")
        .replace('\\', "/")
        .to_ascii_lowercase();
    let hunk_hash = finding
        .plan_defect_evidence
        .iter()
        .map(|e| e.source_ref.as_str())
        .collect::<Vec<_>>();
    let input = format!(
        "{:?}|{}|{}|{}|{}|{}|{}",
        finding.defect_class,
        finding.reason_code.as_deref().unwrap_or(""),
        target_ids.join(","),
        contract_refs.join(","),
        capability_refs.join(","),
        path,
        hunk_hash.join(",")
    );
    hex::encode(Sha256::digest(input.as_bytes()))
}

pub(crate) fn merge_findings(
    shard_reports: &[GroupReviewShardReport],
    reduction_findings: Vec<ReviewFinding>,
) -> Vec<ReviewFinding> {
    let mut merged = BTreeMap::<String, ReviewFinding>::new();
    for finding in shard_reports
        .iter()
        .flat_map(|report| report.findings.iter().cloned())
        .chain(reduction_findings)
    {
        let fingerprint = finding_fingerprint(&finding);
        match merged.get_mut(&fingerprint) {
            Some(existing) => {
                if severity_rank(&finding.severity) > severity_rank(&existing.severity) {
                    existing.severity = finding.severity;
                }
                existing.evidence.extend(finding.evidence);
                existing.evidence.sort();
                existing.evidence.dedup();
                existing
                    .plan_defect_evidence
                    .extend(finding.plan_defect_evidence);
                existing
                    .plan_defect_evidence
                    .sort_by(|a, b| a.source_ref.cmp(&b.source_ref));
                existing
                    .plan_defect_evidence
                    .dedup_by(|a, b| a.source_ref == b.source_ref);
            }
            None => {
                merged.insert(fingerprint, finding);
            }
        }
    }
    merged.into_values().collect()
}

fn severity_rank(severity: &crate::product::coding_models::FindingSeverity) -> u8 {
    match severity {
        crate::product::coding_models::FindingSeverity::Info => 0,
        crate::product::coding_models::FindingSeverity::Warning => 1,
        crate::product::coding_models::FindingSeverity::Error => 2,
    }
}
