#![allow(dead_code)]

use std::sync::Arc;

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::Semaphore;

use super::group_review_budget::{
    BudgetDecision, CapacityDecision, FindingsDecision, GROUP_REVIEW_QUALITY_TARGET_BYTES,
    check_capacity, check_shard_findings, decide_budget, group_review_shard_concurrency,
};
use super::group_review_prompts::build_shard_prompt;
use super::group_review_types::{GroupReviewMaterialSnapshot, PromptBudgetBreakdown};
use super::review_parser::parse_review_payload;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingExecutionStage, GroupReviewObligation, GroupReviewShardReport,
};
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
