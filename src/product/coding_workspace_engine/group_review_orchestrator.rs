#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::Utc;
use sha2::{Digest, Sha256};

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::{Semaphore, mpsc};

use super::{
    CodingProviderStreamRun, CodingWorkspaceEngine, CodingWorkspaceEngineError,
    provider_type_for_name, role_permission_mode_for_attempt, streaming_input_from_adapter,
};

use super::group_review_budget::{
    BudgetDecision, CapacityDecision, FindingsDecision, GROUP_REVIEW_QUALITY_TARGET_BYTES,
    REPAIR_PROMPT_BYTE_CAP, check_capacity, check_shard_findings, decide_budget,
    group_review_shard_concurrency,
};
use super::group_review_errors::{
    GROUP_REVIEW_FAILURE_REASON_CODES, GroupReviewExecutionError, GroupReviewOrchestrationError,
};
use super::group_review_failure_reports::{
    ShardLeaseCleanup, persist_reduction_failure_report, persist_shard_failure_report,
};
use super::group_review_prompts::{authority_for_shard, build_repair_prompt, build_shard_prompt};
#[cfg(test)]
pub(crate) use super::group_review_repair::RepairFidelityError;
pub(crate) use super::group_review_repair::{RepairError, RepairOutput, validate_repair_fidelity};
use super::group_review_types::GroupReviewMaterialSnapshot;
use super::plan_defect_routing::{
    GroupReviewerProjectionBinding, validate_group_review_finding_against_snapshot_authority,
};
use super::review_parser::{CodeReviewProviderPayload, parse_group_review_payload};
use crate::cross_cutting::provider_adapter::{DEFAULT_PROVIDER_TIMEOUT_SECS, ProviderAdapterError};
use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateBlockedGateInput};
use crate::product::coding_models::{
    CasOutcome, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingGateDiagnostic, CodingProviderRole, CodingRoleRun, GroupReviewObligation,
    GroupReviewReductionReport, GroupReviewShardReport, InternalPrReview, ReviewFinding,
    ReviewVerdict,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::id::next_sequential_id;
use crate::product::json_store::ProductStoreError;
use crate::product::models::ProviderName;
use crate::protocol::contracts::{AdapterInput, AdapterRole};

#[cfg(test)]
use std::collections::VecDeque;
#[cfg(test)]
use std::sync::Mutex;

const GROUP_REVIEW_MAX_ATTEMPTS: usize = 3;

pub(crate) struct GroupReviewOrchestrator<'a> {
    executor: &'a dyn GroupReviewExecutor,
    store: &'a CodingAttemptStore,
    role_run_id: Option<String>,
}

impl<'a> GroupReviewOrchestrator<'a> {
    pub(crate) fn new(
        executor: &'a dyn GroupReviewExecutor,
        store: &'a CodingAttemptStore,
    ) -> Self {
        Self {
            executor,
            store,
            role_run_id: None,
        }
    }

    pub(crate) fn with_role_run_id(mut self, role_run_id: &str) -> Self {
        self.role_run_id = Some(role_run_id.to_string());
        self
    }

    pub(crate) fn create_failure_gate(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        node_id: &str,
        error: &GroupReviewOrchestrationError,
    ) -> Result<crate::product::coding_models::CodingGateRequired, ProductStoreError> {
        let (reason_code, phase, actual_value, limit, raw_provider_output_ref) = match error {
            GroupReviewOrchestrationError::CapacityExceeded => (
                "capacity_exceeded",
                "shard",
                Some(
                    self.store
                        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                        .len()
                        .to_string(),
                ),
                Some("20".to_string()),
                None,
            ),
            GroupReviewOrchestrationError::MaterialOverflow { breakdown } => (
                "material_overflow",
                "shard",
                Some(breakdown.total.to_string()),
                Some("30720".to_string()),
                None,
            ),
            GroupReviewOrchestrationError::IdentityMissing => {
                ("identity_missing", "rebuild", None, None, None)
            }
            GroupReviewOrchestrationError::ShardOutputInvalid { raw_ref, .. } => (
                "shard_output_invalid",
                "shard",
                Some("invalid".to_string()),
                Some("8".to_string()),
                (!raw_ref.is_empty()).then(|| raw_ref.clone()),
            ),
            GroupReviewOrchestrationError::ReductionOutputInvalid { raw_ref } => (
                "reduction_output_invalid",
                "reduction",
                Some("invalid".to_string()),
                Some("16".to_string()),
                (!raw_ref.is_empty()).then(|| raw_ref.clone()),
            ),
            GroupReviewOrchestrationError::ShardTransportExhausted { .. } => (
                GROUP_REVIEW_FAILURE_REASON_CODES[3],
                "shard",
                None,
                None,
                None,
            ),
            GroupReviewOrchestrationError::ReductionTransportExhausted => (
                GROUP_REVIEW_FAILURE_REASON_CODES[4],
                "reduction",
                None,
                None,
                None,
            ),
            _ => {
                return Err(ProductStoreError::Io(
                    "group_review_failure_gate_unsupported_error".to_string(),
                ));
            }
        };
        let attempt = if attempt.status == CodingAttemptStatus::Blocked {
            attempt.clone()
        } else {
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )?
        };
        self.store.create_group_review_failure_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::InternalPrReview,
                node_id: Some(node_id.to_string()),
                role: Some(CodingProviderRole::InternalReviewer),
                title: "组级审查失败".to_string(),
                description: format!("组级审查在 {phase} 环节失败: {reason_code}"),
                reason_code: Some(reason_code.to_string()),
                evidence_refs: raw_provider_output_ref.iter().cloned().collect(),
                raw_provider_output_ref,
                available_actions: Vec::new(),
            },
            CodingGateDiagnostic {
                actual_value,
                limit,
                phase: phase.to_string(),
                run_failure_code: reason_code.to_string(),
            },
        )
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
        let prompts = match snapshot
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
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(prompts) => prompts,
            Err(error) => return Err(error),
        };

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
            let failure_attempt = attempt.clone();
            executions.push(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .expect("group review semaphore is never closed");
                let result = match self
                    .execute_with_retry_result(&prompt, GROUP_REVIEW_MAX_ATTEMPTS)
                    .await
                {
                    Ok(result) => result,
                    Err(GroupReviewExecutionError::Transport(_)) => {
                        persist_shard_failure_report(
                            self.store,
                            snapshot,
                            shard,
                            &failure_attempt,
                            self.role_run_id.as_deref().unwrap_or(""),
                            "shard_transport_exhausted",
                        )?;
                        return Err(GroupReviewOrchestrationError::ShardTransportExhausted {
                            shard_id: shard.shard_id.clone(),
                        });
                    }
                    Err(GroupReviewExecutionError::ProviderProtocol(_)) => {
                        persist_shard_failure_report(
                            self.store,
                            snapshot,
                            shard,
                            &failure_attempt,
                            self.role_run_id.as_deref().unwrap_or(""),
                            "shard_output_invalid",
                        )?;
                        return Err(GroupReviewOrchestrationError::ShardOutputInvalid {
                            shard_id: shard.shard_id.clone(),
                            raw_ref: String::new(),
                        });
                    }
                    Err(error) => return Err(error.into()),
                };
                Ok::<_, GroupReviewOrchestrationError>((shard, lease_id, result))
            });
        }

        while let Some(result) = executions.next().await {
            let (shard, lease_id, execution) = result?;
            match self
                .build_and_store_shard_report(snapshot, shard, &attempt, execution)
                .await
            {
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
        Ok(self
            .execute_with_retry_result(prompt, max_attempts)
            .await?
            .full_output)
    }

    pub(crate) async fn execute_with_retry_result(
        &self,
        prompt: &str,
        max_attempts: usize,
    ) -> Result<GroupReviewExecutionResult, GroupReviewExecutionError> {
        let attempts = max_attempts.max(1);
        let mut last_transport_error = None;
        for _ in 0..attempts {
            match self.executor.execute(prompt).await {
                Ok(result) => return Ok(result),
                Err(error @ GroupReviewExecutionError::Transport(_)) => {
                    last_transport_error = Some(error);
                }
                Err(error @ GroupReviewExecutionError::UserCancelled)
                | Err(error @ GroupReviewExecutionError::ProviderProtocol(_))
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
        self.persist_repair_outputs_from_raw_ref(attempt, raw_ref, repair)
    }

    fn persist_repair_outputs_from_raw_ref(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        raw_ref: String,
        repair: &RepairOutput,
    ) -> Result<Vec<String>, ProductStoreError> {
        let repaired_ref = self.store.save_provider_raw_output(
            attempt,
            CodingExecutionStage::InternalPrReview,
            "group_review_repair_output",
            &repair.repaired_output,
        )?;
        Ok(vec![raw_ref, repaired_ref])
    }

    async fn parse_or_repair_group_review_output(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        raw_output: &str,
        raw_ref: String,
        max_findings: usize,
    ) -> Result<ParsedGroupReviewOutput, ProductStoreError> {
        match parse_group_review_payload(raw_output, CodingExecutionStage::InternalPrReview) {
            Ok(payload) => Ok(ParsedGroupReviewOutput {
                payload,
                raw_provider_output_refs: vec![raw_ref],
                output_invalid: false,
            }),
            Err(_) => match self.execute_repair(raw_output, max_findings).await {
                Ok(repair) => match parse_group_review_payload(
                    &repair.repaired_output,
                    CodingExecutionStage::InternalPrReview,
                ) {
                    Ok(payload) => Ok(ParsedGroupReviewOutput {
                        payload,
                        raw_provider_output_refs: self
                            .persist_repair_outputs_from_raw_ref(attempt, raw_ref, &repair)?,
                        output_invalid: false,
                    }),
                    Err(_) => Ok(ParsedGroupReviewOutput::invalid(raw_ref)),
                },
                Err(_) => Ok(ParsedGroupReviewOutput::invalid(raw_ref)),
            },
        }
    }

    pub(crate) async fn execute_reduction(
        &self,
        snapshot: &GroupReviewMaterialSnapshot,
        shard_reports: &[GroupReviewShardReport],
        _authoritative_bindings: &[GroupReviewerProjectionBinding],
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
            let Some(recovery_lease_id) = self.store.claim_group_review_lease(
                &snapshot.attempt_id,
                &snapshot.content_hash,
                "reduction",
                "all",
            )?
            else {
                return Err(GroupReviewOrchestrationError::ReductionInProgress);
            };
            if reduction.findings.iter().any(|finding| {
                validate_group_review_finding_against_snapshot_authority(
                    finding,
                    &snapshot.routing_authority_index,
                    None,
                )
                .is_err()
            }) {
                let raw_ref = reduction
                    .raw_provider_output_refs
                    .last()
                    .cloned()
                    .unwrap_or_default();
                let mut invalid_reduction = reduction;
                invalid_reduction.findings.clear();
                invalid_reduction.run_failure_code = Some("reduction_output_invalid".to_string());
                let persist_result = self.store.write_group_review_reduction_report_cas(
                    &snapshot.attempt_id,
                    invalid_reduction,
                );
                let release_result = self.store.release_group_review_lease(
                    &snapshot.attempt_id,
                    &recovery_lease_id,
                    "",
                );
                let cas_outcome = persist_result?;
                release_result?;
                if cas_outcome == CasOutcome::StoredStale {
                    return Err(GroupReviewOrchestrationError::ReductionStale);
                }
                return Err(GroupReviewOrchestrationError::ReductionOutputInvalid { raw_ref });
            }
            let persist_result =
                self.persist_internal_pr_review_from_reduction(&attempt, snapshot, &reduction);
            let release_result =
                self.store
                    .release_group_review_lease(&snapshot.attempt_id, &recovery_lease_id, "");
            let outcome = persist_result?;
            release_result?;
            return match outcome {
                PersistOutcome::Persisted => Ok(reduction),
                PersistOutcome::SkippedStale => Err(GroupReviewOrchestrationError::ReductionStale),
            };
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
        let execution = match self
            .execute_with_retry_result(&segments.join(), GROUP_REVIEW_MAX_ATTEMPTS)
            .await
        {
            Ok(execution) => execution,
            Err(GroupReviewExecutionError::Transport(_)) => {
                let persist_result = persist_reduction_failure_report(
                    self.store,
                    snapshot,
                    shard_reports,
                    &attempt,
                    self.role_run_id.as_deref().unwrap_or(""),
                    "reduction_transport_exhausted",
                );
                let release_result = self.store.release_group_review_lease(
                    &snapshot.attempt_id,
                    &reduction_lease_id,
                    "",
                );
                persist_result?;
                release_result?;
                return Err(GroupReviewOrchestrationError::ReductionTransportExhausted);
            }
            Err(GroupReviewExecutionError::ProviderProtocol(_)) => {
                let persist_result = persist_reduction_failure_report(
                    self.store,
                    snapshot,
                    shard_reports,
                    &attempt,
                    self.role_run_id.as_deref().unwrap_or(""),
                    "reduction_output_invalid",
                );
                let release_result = self.store.release_group_review_lease(
                    &snapshot.attempt_id,
                    &reduction_lease_id,
                    "",
                );
                persist_result?;
                release_result?;
                return Err(GroupReviewOrchestrationError::ReductionOutputInvalid {
                    raw_ref: String::new(),
                });
            }
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
        let parsed = self
            .parse_or_repair_group_review_output(
                &attempt,
                &execution.full_output,
                raw_ref.clone(),
                16,
            )
            .await?;
        let payload = parsed.payload;
        let findings_exceeded =
            super::group_review_budget::check_reduction_findings(payload.findings.len())
                == FindingsDecision::FindingsExceeded;
        let finding_contract_invalid = payload.findings.iter().any(|finding| {
            validate_group_review_finding_against_snapshot_authority(
                finding,
                &snapshot.routing_authority_index,
                None,
            )
            .is_err()
        });
        let output_invalid = parsed.output_invalid || findings_exceeded || finding_contract_invalid;
        let findings = if output_invalid {
            Vec::new()
        } else {
            merge_findings(shard_reports, payload.findings)
        };
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
            raw_provider_output_refs: parsed.raw_provider_output_refs,
            role_run_ids: execution.role_run_id.into_iter().collect(),
            run_failure_code: output_invalid.then(|| "reduction_output_invalid".to_string()),
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
        if output_invalid {
            self.store
                .release_group_review_lease(&snapshot.attempt_id, &reduction_lease_id, "")?;
            return Err(GroupReviewOrchestrationError::ReductionOutputInvalid { raw_ref });
        }
        let persist_result =
            self.persist_internal_pr_review_from_reduction(&attempt, snapshot, &reduction);
        let release_result =
            self.store
                .release_group_review_lease(&snapshot.attempt_id, &reduction_lease_id, "");
        let outcome = persist_result?;
        release_result?;
        match outcome {
            PersistOutcome::Persisted => Ok(reduction),
            PersistOutcome::SkippedStale => Err(GroupReviewOrchestrationError::ReductionStale),
        }
    }

    pub(crate) fn persist_internal_pr_review_from_reduction(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        snapshot: &GroupReviewMaterialSnapshot,
        reduction: &GroupReviewReductionReport,
    ) -> Result<PersistOutcome, ProductStoreError> {
        let existing = self.store.list_internal_pr_reviews(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let raw_ref = reduction.raw_provider_output_refs.last().ok_or_else(|| {
            ProductStoreError::NotFound {
                kind: "group_review_reduction_raw_output",
                id: reduction.id.clone(),
            }
        })?;
        if existing
            .iter()
            .any(|review| review.raw_provider_output_ref.as_deref() == Some(raw_ref.as_str()))
        {
            return Ok(PersistOutcome::Persisted);
        }
        let raw_output = self.store.read_attempt_artifact_text(attempt, raw_ref)?;
        let payload =
            parse_group_review_payload(&raw_output, CodingExecutionStage::InternalPrReview)
                .map_err(|error| {
                    ProductStoreError::Json(format!(
                        "group_review_reduction_raw_output_invalid: {error}"
                    ))
                })?;
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
        match self.store.save_internal_pr_review_if_active(
            &snapshot.attempt_id,
            &reduction.snapshot_hash,
            &review,
        )? {
            true => Ok(PersistOutcome::Persisted),
            false => Ok(PersistOutcome::SkippedStale),
        }
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

    async fn build_and_store_shard_report(
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
        let parsed = self
            .parse_or_repair_group_review_output(
                attempt,
                &execution.full_output,
                raw_ref.clone(),
                8,
            )
            .await?;
        let payload = parsed.payload;
        let allowed_source_unit_run_ids = authority_for_shard(snapshot, shard)
            .into_iter()
            .map(|entry| entry.source_unit_run_id.clone())
            .collect::<Vec<_>>();
        let finding_authority_invalid = payload.findings.iter().any(|finding| {
            validate_group_review_finding_against_snapshot_authority(
                finding,
                &snapshot.routing_authority_index,
                Some(&allowed_source_unit_run_ids),
            )
            .is_err()
        });
        let output_invalid = parsed.output_invalid
            || check_shard_findings(payload.findings.len()) == FindingsDecision::FindingsExceeded
            || finding_authority_invalid;

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
            findings: if output_invalid {
                Vec::new()
            } else {
                payload.findings
            },
            unresolved_obligations: Vec::<GroupReviewObligation>::new(),
            selected_diff_refs,
            raw_provider_output_refs: parsed.raw_provider_output_refs,
            role_run_ids: execution.role_run_id.into_iter().collect(),
            run_failure_code: output_invalid.then(|| "shard_output_invalid".to_string()),
        };
        match self
            .store
            .write_group_review_shard_report_cas(&snapshot.attempt_id, report.clone())?
        {
            CasOutcome::Written if output_invalid => {
                Err(GroupReviewOrchestrationError::ShardOutputInvalid {
                    shard_id: shard.shard_id.clone(),
                    raw_ref,
                })
            }
            CasOutcome::Written => Ok(Some(report)),
            CasOutcome::StoredStale => Err(GroupReviewOrchestrationError::ShardStaleAudit),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistOutcome {
    Persisted,
    SkippedStale,
}

struct ParsedGroupReviewOutput {
    payload: CodeReviewProviderPayload,
    raw_provider_output_refs: Vec<String>,
    output_invalid: bool,
}

impl ParsedGroupReviewOutput {
    fn invalid(raw_ref: String) -> Self {
        Self {
            payload: CodeReviewProviderPayload {
                verdict: ReviewVerdict::Blocked,
                summary: "组级审查输出无法解析".to_string(),
                findings: Vec::new(),
                impact_scope: Vec::new(),
                pr_description: String::new(),
                commit_message_suggestion: String::new(),
                tested_evidence_refs: Vec::new(),
                diff_refs: Vec::new(),
            },
            raw_provider_output_refs: vec![raw_ref],
            output_invalid: true,
        }
    }
}

pub(crate) struct RealGroupReviewExecutor<'a> {
    engine: &'a CodingWorkspaceEngine,
    attempt: CodingExecutionAttempt,
    provider: &'a dyn StreamingProviderAdapter,
    node_id: String,
    role_run: CodingRoleRun,
    provider_name: ProviderName,
}

impl<'a> RealGroupReviewExecutor<'a> {
    pub(crate) fn new(
        engine: &'a CodingWorkspaceEngine,
        attempt: CodingExecutionAttempt,
        provider: &'a dyn StreamingProviderAdapter,
        node_id: String,
        role_run: CodingRoleRun,
        provider_name: ProviderName,
    ) -> Self {
        Self {
            engine,
            attempt,
            provider,
            node_id,
            role_run,
            provider_name,
        }
    }
}

#[async_trait::async_trait]
impl GroupReviewExecutor for RealGroupReviewExecutor<'_> {
    async fn execute(
        &self,
        prompt: &str,
    ) -> Result<GroupReviewExecutionResult, GroupReviewExecutionError> {
        let worktree_path = self.attempt.worktree_path.clone().ok_or_else(|| {
            GroupReviewExecutionError::Internal(format!(
                "coding_attempt_missing_worktree: {}",
                self.attempt.id
            ))
        })?;
        let input = AdapterInput {
            provider_type: provider_type_for_name(&self.provider_name),
            role: AdapterRole::Reviewer,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            provider_stream_log_dir: Some(
                self.engine.attempt_provider_stream_log_dir(&self.attempt),
            ),
            prompt: prompt.to_string(),
            context_files: Vec::new(),
            output_schema: "coding_workspace_internal_pr_review_json".to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };
        let permission_mode = role_permission_mode_for_attempt(
            &self.engine.store,
            &self.attempt,
            CodingProviderRole::InternalReviewer,
        )
        .map_err(map_group_review_engine_error)?;
        let mut provider_input =
            streaming_input_from_adapter(&input, worktree_path, permission_mode);
        provider_input.workspace_session_id = Some(self.attempt.id.clone());
        provider_input.resume_provider_session_id = None;
        let (command_tx, mut command_rx) = mpsc::channel::<CodingRunnerCommand>(1);
        drop(command_tx);
        let full_output = self
            .engine
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &self.attempt,
                node_id: &self.node_id,
                role_run: Some(&self.role_run),
                provider: self.provider,
                legacy_input: &input,
                input: provider_input,
                provider_name: &self.provider_name,
                provider_role: CodingProviderRole::InternalReviewer,
                command_rx: &mut command_rx,
                allow_legacy_stream_fallback: true,
                timeout: None,
                timeout_reason_code: None,
                suppress_failure_side_effects: true,
                validated_input: None,
            })
            .await
            .map_err(map_group_review_engine_error)?;
        Ok(GroupReviewExecutionResult {
            full_output,
            role_run_id: Some(self.role_run.id.clone()),
        })
    }
}

fn map_group_review_engine_error(error: CodingWorkspaceEngineError) -> GroupReviewExecutionError {
    match error {
        CodingWorkspaceEngineError::Aborted => GroupReviewExecutionError::UserCancelled,
        CodingWorkspaceEngineError::ProviderProtocol(message) => {
            GroupReviewExecutionError::ProviderProtocol(message)
        }
        CodingWorkspaceEngineError::ProviderStream(message)
            if message == "provider_choice_unresolved" =>
        {
            GroupReviewExecutionError::ProviderProtocol(message)
        }
        CodingWorkspaceEngineError::ProviderStream(message)
        | CodingWorkspaceEngineError::ProviderAdapter(ProviderAdapterError {
            details: message,
            ..
        }) => GroupReviewExecutionError::Transport(message),
        error => GroupReviewExecutionError::Internal(error.to_string()),
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
    pub role_run_id: Option<String>,
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
