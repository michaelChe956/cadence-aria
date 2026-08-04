#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::Utc;
use sha2::{Digest, Sha256};

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::Semaphore;

use super::group_review_budget::{
    BudgetDecision, CapacityDecision, FindingsDecision, GROUP_REVIEW_QUALITY_TARGET_BYTES,
    REPAIR_PROMPT_BYTE_CAP, check_capacity, check_shard_findings, decide_budget,
    group_review_shard_concurrency,
};
use super::group_review_prompts::{build_repair_prompt, build_shard_prompt};
use super::group_review_types::{GroupReviewMaterialSnapshot, PromptBudgetBreakdown};
use super::plan_defect_routing::{GroupReviewerProjectionBinding, validate_group_reviewer_finding};
use super::review_parser::parse_review_payload;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CasOutcome, CodingExecutionStage, GroupReviewObligation, GroupReviewReductionReport,
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

        let existing_reports = self.matching_shard_reports(snapshot)?;
        if existing_reports.len() == snapshot.partition_result.shards.len() {
            return Ok(existing_reports);
        }

        let attempt = self.store.find_attempt_by_id(&snapshot.attempt_id)?;
        let existing_shard_ids = existing_reports
            .iter()
            .map(|report| report.shard_id.clone())
            .collect::<BTreeSet<_>>();
        let semaphore = Arc::new(Semaphore::new(group_review_shard_concurrency()));
        let mut executions = FuturesUnordered::new();
        let mut reports = existing_reports;
        let mut claimed_leases = ShardLeaseCleanup::new(self.store, &snapshot.attempt_id);
        for (shard, prompt) in prompts {
            if existing_shard_ids.contains(&shard.shard_id) {
                continue;
            }
            let Some(lease_id) = self.store.claim_group_review_lease(
                &snapshot.attempt_id,
                &snapshot.content_hash,
                "shard",
                &shard.shard_id,
            )?
            else {
                if let Some(report_id) = self.store.get_completed_group_review_result(
                    &snapshot.attempt_id,
                    &snapshot.content_hash,
                    "shard",
                    &shard.shard_id,
                )? && let Some(report) = self
                    .matching_shard_reports(snapshot)?
                    .into_iter()
                    .find(|report| report.id == report_id)
                {
                    reports.push(report);
                    continue;
                }
                return Err(GroupReviewOrchestrationError::ShardInProgress {
                    shard_id: shard.shard_id.clone(),
                });
            };
            claimed_leases.claim(lease_id.clone());
            let semaphore = semaphore.clone();
            executions.push(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .expect("group review semaphore is never closed");
                let result = self.executor.execute(&prompt).await?;
                Ok::<_, GroupReviewOrchestrationError>((shard, lease_id, result))
            });
        }

        while let Some(result) = executions.next().await {
            let (shard, lease_id, execution) = result?;
            match self.build_and_store_shard_report(snapshot, shard, &attempt, execution) {
                Ok(Some(report)) => {
                    self.store.release_group_review_lease(
                        &snapshot.attempt_id,
                        &lease_id,
                        &report.id,
                    )?;
                    claimed_leases.complete(&lease_id);
                    reports.push(report);
                }
                Ok(None) => {
                    self.store
                        .release_group_review_lease(&snapshot.attempt_id, &lease_id, "")?;
                    claimed_leases.complete(&lease_id);
                }
                Err(error) => return Err(error),
            }
        }
        reports.sort_by(|left, right| left.shard_id.cmp(&right.shard_id));
        Ok(reports)
    }

    pub(crate) async fn execute_with_retry(
        &self,
        prompt: &str,
        max_attempts: usize,
    ) -> Result<String, GroupReviewExecutionError> {
        let attempts = max_attempts.max(1);
        let mut last_transport_error = None;
        for _ in 0..attempts {
            match self.executor.execute(prompt).await {
                Ok(result) => return Ok(result.full_output),
                Err(error @ GroupReviewExecutionError::Transport(_)) => {
                    last_transport_error = Some(error);
                }
                Err(error @ GroupReviewExecutionError::UserCancelled)
                | Err(error @ GroupReviewExecutionError::Internal(_)) => return Err(error),
            }
        }
        Err(last_transport_error.expect("at least one transport error after exhausted retries"))
    }

    pub(crate) async fn execute_repair(
        &self,
        raw_output: &str,
        max_findings: usize,
    ) -> Result<RepairOutput, RepairError> {
        let prompt = build_repair_prompt(raw_output);
        if prompt.measure().total > REPAIR_PROMPT_BYTE_CAP {
            return Err(RepairError::InputTooLarge);
        }
        let repaired_output = self.executor.execute(&prompt.join()).await?.full_output;
        validate_repair_fidelity(raw_output, &repaired_output, max_findings)?;
        Ok(RepairOutput { repaired_output })
    }

    pub(crate) fn persist_repair_outputs(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        raw_output: &str,
        repair: &RepairOutput,
    ) -> Result<Vec<String>, ProductStoreError> {
        let raw_ref = self.store.save_provider_raw_output(
            attempt,
            CodingExecutionStage::InternalPrReview,
            "group_review_repair_raw",
            raw_output,
        )?;
        let repaired_ref = self.store.save_provider_raw_output(
            attempt,
            CodingExecutionStage::InternalPrReview,
            "group_review_repair_output",
            &repair.repaired_output,
        )?;
        Ok(vec![raw_ref, repaired_ref])
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
        let attempt = self.store.find_attempt_by_id(&snapshot.attempt_id)?;
        let active_snapshot = self
            .store
            .get_active_group_review_snapshot_hash(&snapshot.attempt_id)?;
        if active_snapshot.as_deref() == Some(snapshot.content_hash.as_str())
            && let Some(reduction) = self
                .store
                .list_group_review_reduction_reports(&snapshot.attempt_id)?
                .into_iter()
                .find(|report| {
                    report.snapshot_hash == snapshot.content_hash
                        && report.run_failure_code.is_none()
                })
        {
            self.persist_internal_pr_review_from_reduction(&attempt, snapshot, &reduction)?;
            return Ok(reduction);
        }
        let segments =
            super::group_review_prompts::build_reduction_prompt(snapshot, shard_reports, None);
        let breakdown = segments.measure();
        if decide_budget(breakdown.total, GROUP_REVIEW_QUALITY_TARGET_BYTES)
            == BudgetDecision::Overflow
        {
            return Err(GroupReviewOrchestrationError::MaterialOverflow { breakdown });
        }
        let Some(reduction_lease_id) = self.store.claim_group_review_lease(
            &snapshot.attempt_id,
            &snapshot.content_hash,
            "reduction",
            "all",
        )?
        else {
            return Err(GroupReviewOrchestrationError::ReductionInProgress);
        };
        let execution = match self.executor.execute(&segments.join()).await {
            Ok(execution) => execution,
            Err(error) => {
                self.store.release_group_review_lease(
                    &snapshot.attempt_id,
                    &reduction_lease_id,
                    "",
                )?;
                return Err(error.into());
            }
        };
        let raw_ref = match self.store.save_provider_raw_output(
            &attempt,
            CodingExecutionStage::InternalPrReview,
            "group_review_reduction",
            &execution.full_output,
        ) {
            Ok(raw_ref) => raw_ref,
            Err(error) => {
                self.store.release_group_review_lease(
                    &snapshot.attempt_id,
                    &reduction_lease_id,
                    "",
                )?;
                return Err(error.into());
            }
        };
        let payload = parse_review_payload(
            &execution.full_output,
            CodingExecutionStage::InternalPrReview,
        );
        if super::group_review_budget::check_reduction_findings(payload.findings.len())
            == FindingsDecision::FindingsExceeded
        {
            self.store
                .release_group_review_lease(&snapshot.attempt_id, &reduction_lease_id, "")?;
            return Err(GroupReviewOrchestrationError::ReductionOutputInvalid { raw_ref });
        }
        let findings = merge_findings(shard_reports, payload.findings);
        if findings.iter().any(|finding| {
            validate_group_reviewer_finding(finding, authoritative_bindings).is_err()
        }) {
            self.store
                .release_group_review_lease(&snapshot.attempt_id, &reduction_lease_id, "")?;
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
        let cas_outcome = match self
            .store
            .write_group_review_reduction_report_cas(&snapshot.attempt_id, reduction.clone())
        {
            Ok(outcome) => outcome,
            Err(error) => {
                self.store.release_group_review_lease(
                    &snapshot.attempt_id,
                    &reduction_lease_id,
                    "",
                )?;
                return Err(error.into());
            }
        };
        match cas_outcome {
            CasOutcome::Written => {}
            CasOutcome::StoredStale => {
                self.store.release_group_review_lease(
                    &snapshot.attempt_id,
                    &reduction_lease_id,
                    "",
                )?;
                return Err(GroupReviewOrchestrationError::ReductionStale);
            }
        }
        let persist_result =
            self.persist_internal_pr_review_from_reduction(&attempt, snapshot, &reduction);
        let release_result =
            self.store
                .release_group_review_lease(&snapshot.attempt_id, &reduction_lease_id, "");
        persist_result?;
        release_result?;
        Ok(reduction)
    }

    fn persist_internal_pr_review_from_reduction(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        snapshot: &GroupReviewMaterialSnapshot,
        reduction: &GroupReviewReductionReport,
    ) -> Result<(), ProductStoreError> {
        let existing = self.store.list_internal_pr_reviews(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let raw_ref = reduction.raw_provider_output_refs.first().ok_or_else(|| {
            ProductStoreError::NotFound {
                kind: "group_review_reduction_raw_output",
                id: reduction.id.clone(),
            }
        })?;
        if existing
            .iter()
            .any(|review| review.raw_provider_output_ref.as_deref() == Some(raw_ref.as_str()))
        {
            return Ok(());
        }
        let raw_output = self.store.read_attempt_artifact_text(attempt, raw_ref)?;
        let payload = parse_review_payload(&raw_output, CodingExecutionStage::InternalPrReview);
        let review = InternalPrReview {
            id: next_sequential_id("internal_review", existing.len()),
            attempt_id: snapshot.attempt_id.clone(),
            review_request_id: snapshot.review_request_id.clone(),
            verdict: reduction.verdict.clone(),
            findings: reduction.findings.clone(),
            impact_scope: reduction.impact_scope.clone(),
            pr_description: reduction.pr_description.clone(),
            commit_message_suggestion: reduction.commit_message_suggestion.clone(),
            tested_evidence_refs: payload.tested_evidence_refs,
            diff_refs: payload.diff_refs,
            summary: payload.summary,
            created_at: Utc::now().to_rfc3339(),
            raw_provider_output_ref: Some(raw_ref.clone()),
            role_run_id: None,
            run_no: None,
        };
        self.store.save_internal_pr_review(attempt, &review)
    }

    fn matching_shard_reports(
        &self,
        snapshot: &GroupReviewMaterialSnapshot,
    ) -> Result<Vec<GroupReviewShardReport>, ProductStoreError> {
        let expected = snapshot
            .partition_result
            .shards
            .iter()
            .map(|shard| shard.shard_id.as_str())
            .collect::<BTreeSet<_>>();
        let active_snapshot = self
            .store
            .get_active_group_review_snapshot_hash(&snapshot.attempt_id)?;
        let mut reports = self
            .store
            .list_group_review_shard_reports(&snapshot.attempt_id)?
            .into_iter()
            .filter(|report| {
                active_snapshot.as_deref() == Some(snapshot.content_hash.as_str())
                    && report.snapshot_hash == snapshot.content_hash
                    && report.run_failure_code.is_none()
                    && expected.contains(report.shard_id.as_str())
            })
            .collect::<Vec<_>>();
        reports.sort_by(|left, right| left.shard_id.cmp(&right.shard_id));
        Ok(reports)
    }

    fn build_and_store_shard_report(
        &self,
        snapshot: &GroupReviewMaterialSnapshot,
        shard: &super::group_review_types::GroupShardSpec,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        execution: GroupReviewExecutionResult,
    ) -> Result<Option<GroupReviewShardReport>, GroupReviewOrchestrationError> {
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
        match self
            .store
            .write_group_review_shard_report_cas(&snapshot.attempt_id, report.clone())?
        {
            CasOutcome::Written => Ok(Some(report)),
            CasOutcome::StoredStale => Ok(None),
        }
    }
}

struct ShardLeaseCleanup<'a> {
    store: &'a CodingAttemptStore,
    attempt_id: &'a str,
    lease_ids: Vec<String>,
}

impl<'a> ShardLeaseCleanup<'a> {
    fn new(store: &'a CodingAttemptStore, attempt_id: &'a str) -> Self {
        Self {
            store,
            attempt_id,
            lease_ids: Vec::new(),
        }
    }

    fn claim(&mut self, lease_id: String) {
        self.lease_ids.push(lease_id);
    }

    fn complete(&mut self, lease_id: &str) {
        self.lease_ids
            .retain(|claimed_lease_id| claimed_lease_id != lease_id);
    }
}

impl Drop for ShardLeaseCleanup<'_> {
    fn drop(&mut self) {
        for lease_id in &self.lease_ids {
            let _ = self
                .store
                .release_group_review_lease(self.attempt_id, lease_id, "");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepairOutput {
    pub(crate) repaired_output: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum RepairFidelityError {
    #[error("repair_missing_marker")]
    MissingMarker,
    #[error("repair_verdict_mismatch")]
    VerdictMismatch,
    #[error("repair_forbidden_approve")]
    ForbiddenApprove,
    #[error("repair_finding_not_subtraceable")]
    FindingNotSubtraceable,
    #[error("repair_evidence_not_subtraceable")]
    EvidenceNotSubtraceable,
    #[error("repair_target_not_subtraceable")]
    TargetNotSubtraceable,
    #[error("repair_too_many_findings")]
    TooManyFindings,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RepairError {
    #[error("repair_input_too_large")]
    InputTooLarge,
    #[error("repair_executor: {0}")]
    Executor(#[from] GroupReviewExecutionError),
    #[error("repair_fidelity: {0}")]
    Fidelity(#[from] RepairFidelityError),
}

pub(crate) fn validate_repair_fidelity(
    raw_output: &str,
    repaired_output: &str,
    max_findings: usize,
) -> Result<(), RepairFidelityError> {
    let raw_verdict = verdict_marker(raw_output).ok_or(RepairFidelityError::MissingMarker)?;
    let repaired_verdict =
        verdict_marker(repaired_output).ok_or(RepairFidelityError::MissingMarker)?;
    if repaired_verdict == ReviewVerdict::Approve {
        return Err(RepairFidelityError::ForbiddenApprove);
    }
    if repaired_verdict != raw_verdict {
        return Err(RepairFidelityError::VerdictMismatch);
    }
    let payload = parse_review_payload(repaired_output, CodingExecutionStage::InternalPrReview);
    if payload.findings.len() > max_findings {
        return Err(RepairFidelityError::TooManyFindings);
    }
    if payload.findings.iter().any(|finding| {
        !raw_output.contains(&finding.message)
            || finding
                .evidence
                .iter()
                .any(|evidence| !raw_output.contains(evidence))
            || finding.plan_defect_evidence.iter().any(|evidence| {
                !raw_output.contains(&evidence.source_ref)
                    || !raw_output.contains(&evidence.message)
                    || !raw_output.contains(&evidence.kind)
            })
    }) {
        if payload
            .findings
            .iter()
            .any(|finding| !raw_output.contains(&finding.message))
        {
            return Err(RepairFidelityError::FindingNotSubtraceable);
        }
        return Err(RepairFidelityError::EvidenceNotSubtraceable);
    }
    if payload.findings.iter().any(|finding| {
        finding.repair_target.as_ref().is_some_and(|target| {
            let kind = match target.kind {
                crate::product::models::RepairTargetKind::CurrentWorkItem => "current_work_item",
                crate::product::models::RepairTargetKind::UpstreamWorkItem => "upstream_work_item",
                crate::product::models::RepairTargetKind::Subgraph => "subgraph",
            };
            !raw_output.contains(kind)
                || target
                    .logical_work_item_ids
                    .iter()
                    .chain(target.work_item_revision_ids.iter())
                    .any(|id| !raw_output.contains(id))
        })
    }) {
        return Err(RepairFidelityError::TargetNotSubtraceable);
    }
    Ok(())
}

fn verdict_marker(output: &str) -> Option<ReviewVerdict> {
    output.lines().find_map(|line| {
        let value = line.trim().strip_prefix("GROUP_REVIEW_VERDICT:")?.trim();
        match value {
            "approve" => Some(ReviewVerdict::Approve),
            "request_changes" => Some(ReviewVerdict::RequestChanges),
            "blocked" => Some(ReviewVerdict::Blocked),
            _ => None,
        }
    })
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
    #[error("shard_in_progress: {shard_id}")]
    ShardInProgress { shard_id: String },
    #[error("reduction_in_progress")]
    ReductionInProgress,
    #[error("reduction_not_ready")]
    ReductionNotReady,
    #[error("reduction_output_invalid: {raw_ref}")]
    ReductionOutputInvalid { raw_ref: String },
    #[error("reduction_stale")]
    ReductionStale,
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
    let target_ids = target.map(|target| {
        let mut logical_work_item_ids = target.logical_work_item_ids.clone();
        let mut work_item_revision_ids = target.work_item_revision_ids.clone();
        logical_work_item_ids.sort();
        logical_work_item_ids.dedup();
        work_item_revision_ids.sort();
        work_item_revision_ids.dedup();
        (logical_work_item_ids, work_item_revision_ids)
    });
    let mut contract_refs = finding.contract_refs.clone();
    let mut capability_refs = finding.capability_refs.clone();
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
    let mut hunk_hashes = finding
        .plan_defect_evidence
        .iter()
        .map(|e| e.source_ref.as_str())
        .collect::<Vec<_>>();
    hunk_hashes.sort_unstable();
    hunk_hashes.dedup();
    let input = serde_json::json!({
        "defect_class": &finding.defect_class,
        "reason_code": &finding.reason_code,
        "repair_target_ids": target_ids,
        "contract_refs": contract_refs,
        "capability_refs": capability_refs,
        "path": path,
        "hunk_hashes": hunk_hashes,
    });
    let canonical = serde_json::to_vec(&input).expect("fingerprint fields serialize");
    hex::encode(Sha256::digest(canonical))
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
