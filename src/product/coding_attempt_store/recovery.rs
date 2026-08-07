use std::fs;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage, CodingProviderRole,
    CodingRoleRun, CodingRoleRunEventType, CodingRoleRunRetryMetadata, CodingRoleRunStatus,
    CodingRoleRunTrigger,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

use super::locking::{ExclusiveFileLock, with_exclusive_lock};

pub(crate) const FAILED_CODE_REVIEW_RECOVERY_JOURNAL_FILE: &str =
    "failed-code-review-recovery.json";
const FAILED_CODE_REVIEW_RECOVERIES_DIR: &str = "failed-code-review-recoveries";
const COMPLETED_FAILED_CODE_REVIEW_RECOVERIES_DIR: &str = "completed";
const FAILED_CODE_REVIEW_RECOVERY_ARBITRATION_TARGET: &str =
    "failed-code-review-recovery-arbitration";

pub(crate) struct FailedCodeReviewRecoveryArbitrationGuard {
    _lock: ExclusiveFileLock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailedCodeReviewRecoveryPhase {
    Prepared,
    AttemptReopened,
    RetryRunCreated,
    AttemptRunning,
    GateResolved,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedCodeReviewRecoveryJournal {
    pub attempt_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub expected_gate_id: String,
    pub expected_failed_node_id: String,
    pub expected_stale_role_run_id: String,
    pub recovery_key: String,
    pub retry_role_run_id: Option<String>,
    pub phase: FailedCodeReviewRecoveryPhase,
    pub runner_started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl FailedCodeReviewRecoveryJournal {
    pub fn is_completed(&self) -> bool {
        self.phase == FailedCodeReviewRecoveryPhase::Completed
    }
}

impl super::CodingAttemptStore {
    pub(crate) fn acquire_failed_code_review_recovery_arbitration(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<FailedCodeReviewRecoveryArbitrationGuard, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let target = self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join(FAILED_CODE_REVIEW_RECOVERY_ARBITRATION_TARGET);
        Ok(FailedCodeReviewRecoveryArbitrationGuard {
            _lock: ExclusiveFileLock::acquire(&target)?,
        })
    }

    pub(crate) fn ensure_plan_repair_can_win_recovery_arbitration(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let Some(journal) = self.get_failed_code_review_recovery_journal(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?
        else {
            return Ok(());
        };
        if !journal.is_completed() {
            return Ok(());
        }
        let Some(retry_role_run_id) = journal.retry_role_run_id.as_deref() else {
            return Err(recovery_state_changed());
        };
        let retry = self.get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            retry_role_run_id,
        )?;
        let Some(node_id) = retry.node_id.as_deref() else {
            return Err(recovery_state_changed());
        };
        if retry.attempt_id != journal.attempt_id
            || retry.stage != CodingExecutionStage::CodeReview
            || retry.role != CodingProviderRole::CodeReviewer
            || !is_failed_review_manual_retry(&retry, &journal)
            || retry.supersedes_run_id.as_deref()
                != Some(journal.expected_stale_role_run_id.as_str())
            || !matches!(
                retry.status,
                CodingRoleRunStatus::Running
                    | CodingRoleRunStatus::Completed
                    | CodingRoleRunStatus::Blocked
            )
        {
            return Err(recovery_state_changed());
        }
        let matching_nodes = self
            .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .filter(|node| {
                node.id == node_id
                    && node.attempt_id == attempt.id
                    && node.stage == CodingExecutionStage::CodeReview
            })
            .count();
        let provider_started = self
            .list_role_run_events(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                retry_role_run_id,
            )?
            .iter()
            .any(|event| {
                event.event_type == CodingRoleRunEventType::ProviderStart
                    && event.node_id.as_deref() == Some(node_id)
            });
        if matching_nodes != 1 || !provider_started {
            return Err(recovery_state_changed());
        }
        self.archive_completed_failed_code_review_recovery_journal(&journal)
    }

    fn failed_code_review_recovery_journal_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        Ok(self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join(FAILED_CODE_REVIEW_RECOVERY_JOURNAL_FILE))
    }

    fn archived_failed_code_review_recovery_journal_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(gate_id)?;
        Ok(self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join(FAILED_CODE_REVIEW_RECOVERIES_DIR)
            .join(COMPLETED_FAILED_CODE_REVIEW_RECOVERIES_DIR)
            .join(format!("{gate_id}.json")))
    }

    pub fn get_failed_code_review_recovery_journal(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<FailedCodeReviewRecoveryJournal>, ProductStoreError> {
        let path =
            self.failed_code_review_recovery_journal_path(project_id, issue_id, attempt_id)?;
        if !super::path_is_regular_file(&path)? {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    pub fn get_archived_failed_code_review_recovery_journal(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
    ) -> Result<Option<FailedCodeReviewRecoveryJournal>, ProductStoreError> {
        let path = self.archived_failed_code_review_recovery_journal_path(
            project_id, issue_id, attempt_id, gate_id,
        )?;
        if !super::path_is_regular_file(&path)? {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    fn archive_completed_failed_code_review_recovery_journal(
        &self,
        journal: &FailedCodeReviewRecoveryJournal,
    ) -> Result<(), ProductStoreError> {
        if !journal.is_completed()
            || journal.runner_started_at.is_none()
            || journal.completed_at.is_none()
        {
            return Err(recovery_state_changed());
        }
        let current_path = self.failed_code_review_recovery_journal_path(
            &journal.project_id,
            &journal.issue_id,
            &journal.attempt_id,
        )?;
        if !super::path_is_regular_file(&current_path)? {
            return Err(recovery_state_changed());
        }
        let archived_path = self.archived_failed_code_review_recovery_journal_path(
            &journal.project_id,
            &journal.issue_id,
            &journal.attempt_id,
            &journal.expected_gate_id,
        )?;
        if super::path_is_regular_file(&archived_path)? {
            let archived: FailedCodeReviewRecoveryJournal = read_json(&archived_path)?;
            if archived != *journal {
                return Err(recovery_state_changed());
            }
            super::remove_file_if_exists(&current_path)?;
            return Ok(());
        }
        if let Some(parent) = archived_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                ProductStoreError::Io(format!("create {}: {error}", parent.display()))
            })?;
        }
        fs::rename(&current_path, &archived_path).map_err(|error| {
            ProductStoreError::Io(format!(
                "rename {} to {}: {error}",
                current_path.display(),
                archived_path.display()
            ))
        })
    }

    pub fn prepare_failed_code_review_recovery_journal(
        &self,
        attempt: &CodingExecutionAttempt,
        gate_id: &str,
        failed_node_id: &str,
        stale_role_run_id: &str,
    ) -> Result<FailedCodeReviewRecoveryJournal, ProductStoreError> {
        validate_relative_id(gate_id)?;
        validate_relative_id(failed_node_id)?;
        validate_relative_id(stale_role_run_id)?;
        let _arbitration = self.acquire_failed_code_review_recovery_arbitration(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let attempt_path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let journal_path = self.failed_code_review_recovery_journal_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        with_exclusive_lock(&attempt_path, || {
            let current = self.validate_attempt_lineage(attempt)?;
            if self.plan_amendment_blocks_failed_review_recovery(&current)? {
                return Err(ProductStoreError::Io(
                    "plan_amendment_blocks_provider_run".to_string(),
                ));
            }
            with_exclusive_lock(&journal_path, || {
                if let Some(existing) = self.get_failed_code_review_recovery_journal(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )? {
                    if existing.expected_gate_id == gate_id
                        && existing.expected_failed_node_id == failed_node_id
                        && existing.expected_stale_role_run_id == stale_role_run_id
                    {
                        return Ok(existing);
                    }
                    if !existing.is_completed() {
                        return Err(recovery_state_changed());
                    }
                    self.archive_completed_failed_code_review_recovery_journal(&existing)?;
                }

                let now = Utc::now().to_rfc3339();
                let journal = FailedCodeReviewRecoveryJournal {
                    attempt_id: current.id.clone(),
                    project_id: current.project_id.clone(),
                    issue_id: current.issue_id.clone(),
                    expected_gate_id: gate_id.to_string(),
                    expected_failed_node_id: failed_node_id.to_string(),
                    expected_stale_role_run_id: stale_role_run_id.to_string(),
                    recovery_key: format!(
                        "failed_code_review_recovery:{}:{gate_id}:{stale_role_run_id}",
                        current.id
                    ),
                    retry_role_run_id: None,
                    phase: FailedCodeReviewRecoveryPhase::Prepared,
                    runner_started_at: None,
                    completed_at: None,
                    created_at: now.clone(),
                    updated_at: now,
                };
                self.save_failed_code_review_recovery_journal(&journal)?;
                Ok(journal)
            })
        })
    }

    pub(crate) fn rollback_failed_code_review_recovery_for_plan_amendment_locked(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let Some(journal) = self.get_failed_code_review_recovery_journal(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?
        else {
            return Ok(());
        };
        if journal.is_completed() {
            return Err(recovery_state_changed());
        }
        let role_runs = self.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let matching_retries = role_runs
            .iter()
            .filter(|run| run.reason_code.as_deref() == Some(journal.recovery_key.as_str()))
            .collect::<Vec<_>>();
        if matching_retries.len() > 1 {
            return Err(recovery_state_changed());
        }
        let matching_retry = matching_retries.into_iter().next();
        let retry = if let Some(retry_role_run_id) = journal.retry_role_run_id.as_deref() {
            let retry = role_runs.iter().find(|run| run.id == retry_role_run_id);
            if retry.is_some_and(|run| {
                run.reason_code.as_deref() != Some(journal.recovery_key.as_str())
            }) || matching_retry.is_some_and(|run| run.id != retry_role_run_id)
            {
                return Err(recovery_state_changed());
            }
            retry
        } else {
            matching_retry
        };
        if let Some(retry) = retry.as_ref() {
            validate_retry_role_run(retry, &journal)?;
        }
        let retry_role_run_id = retry
            .map(|run| run.id.as_str())
            .or(journal.retry_role_run_id.as_deref());
        if let Some(retry_role_run_id) = retry_role_run_id {
            let mut stale = self.get_role_run(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &journal.expected_stale_role_run_id,
            )?;
            validate_stale_role_run(&stale, &journal, retry_role_run_id)?;
            stale.status = CodingRoleRunStatus::Failed;
            stale.superseded_by_run_id = None;
            if stale.completed_at.is_none() {
                stale.completed_at = Some(Utc::now().to_rfc3339());
            }
            self.save_role_run(&attempt.project_id, &attempt.issue_id, &stale)?;
        }
        if let Some(retry) = retry {
            super::remove_file_if_exists(&self.role_run_path(
                &attempt.project_id,
                &attempt.issue_id,
                retry,
            ))?;
        }
        let resolved_gate_path = self
            .blocked_gates_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("resolved")
            .join(format!("{}.json", journal.expected_gate_id));
        if journal.phase >= FailedCodeReviewRecoveryPhase::GateResolved
            || super::path_is_regular_file(&resolved_gate_path)?
        {
            self.reopen_failed_code_review_gate_for_plan_repair(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &journal.expected_gate_id,
            )?;
        }
        let path = self.failed_code_review_recovery_journal_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        super::remove_file_if_exists(&path)
    }

    pub fn advance_failed_code_review_recovery_journal(
        &self,
        expected: &FailedCodeReviewRecoveryJournal,
        phase: FailedCodeReviewRecoveryPhase,
        retry_role_run_id: Option<&str>,
    ) -> Result<FailedCodeReviewRecoveryJournal, ProductStoreError> {
        let _arbitration = self.acquire_failed_code_review_recovery_arbitration(
            &expected.project_id,
            &expected.issue_id,
            &expected.attempt_id,
        )?;
        let attempt = self.get_attempt(
            &expected.project_id,
            &expected.issue_id,
            &expected.attempt_id,
        )?;
        self.ensure_failed_review_recovery_write_allowed(&attempt)?;
        let Some(mut current) = self.get_failed_code_review_recovery_journal(
            &expected.project_id,
            &expected.issue_id,
            &expected.attempt_id,
        )?
        else {
            return Err(recovery_state_changed());
        };
        ensure_same_journal(&current, expected)?;
        if let Some(retry_role_run_id) = retry_role_run_id {
            validate_relative_id(retry_role_run_id)?;
            if current
                .retry_role_run_id
                .as_deref()
                .is_some_and(|existing| existing != retry_role_run_id)
            {
                return Err(recovery_state_changed());
            }
            current.retry_role_run_id = Some(retry_role_run_id.to_string());
        }
        if phase > current.phase {
            current.phase = phase;
        }
        current.updated_at = Utc::now().to_rfc3339();
        self.save_failed_code_review_recovery_journal(&current)?;
        Ok(current)
    }

    pub fn ensure_failed_code_review_retry_role_run(
        &self,
        attempt: &CodingExecutionAttempt,
        journal: &FailedCodeReviewRecoveryJournal,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        let _arbitration = self.acquire_failed_code_review_recovery_arbitration(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let attempt = self.validate_attempt_lineage(attempt)?;
        self.ensure_failed_review_recovery_write_allowed(&attempt)?;
        ensure_journal_attempt(journal, &attempt)?;
        let existing = self.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let mut matching = existing
            .iter()
            .filter(|run| run.reason_code.as_deref() == Some(journal.recovery_key.as_str()));
        let found = matching.next().cloned();
        if matching.next().is_some() {
            return Err(recovery_state_changed());
        }

        let retry = if let Some(run) = found {
            validate_retry_role_run(&run, journal)?;
            run
        } else {
            let id = next_sequential_id("coding_role_run", existing.len());
            let run_no = existing
                .iter()
                .filter(|run| {
                    run.stage == CodingExecutionStage::CodeReview
                        && run.role == CodingProviderRole::CodeReviewer
                })
                .map(|run| run.run_no)
                .max()
                .unwrap_or(0)
                + 1;
            let run = CodingRoleRun {
                id: id.clone(),
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                role: CodingProviderRole::CodeReviewer,
                run_no,
                status: CodingRoleRunStatus::Running,
                trigger: CodingRoleRunTrigger::ManualRetry,
                retry_metadata: Some(CodingRoleRunRetryMetadata {
                    cycle_id: id,
                    attempt_no: 1,
                    prior_run_id: Some(journal.expected_stale_role_run_id.clone()),
                }),
                node_id: None,
                started_at: Utc::now().to_rfc3339(),
                completed_at: None,
                supersedes_run_id: Some(journal.expected_stale_role_run_id.clone()),
                superseded_by_run_id: None,
                reason_code: Some(journal.recovery_key.clone()),
                raw_provider_output_refs: Vec::new(),
                artifact_refs: Vec::new(),
            };
            self.save_role_run(&attempt.project_id, &attempt.issue_id, &run)?;
            run
        };

        let mut stale = self.get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &journal.expected_stale_role_run_id,
        )?;
        validate_stale_role_run(&stale, journal, &retry.id)?;
        if matches!(
            stale.status,
            CodingRoleRunStatus::Running | CodingRoleRunStatus::Failed
        ) {
            if stale.status == CodingRoleRunStatus::Running {
                stale.status = CodingRoleRunStatus::Superseded;
                stale.completed_at = Some(Utc::now().to_rfc3339());
            }
            stale.superseded_by_run_id = Some(retry.id.clone());
            self.save_role_run(&attempt.project_id, &attempt.issue_id, &stale)?;
        }
        Ok(retry)
    }

    pub fn failed_code_review_recovery_gate_exists(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
    ) -> Result<bool, ProductStoreError> {
        validate_relative_id(gate_id)?;
        let root = self.blocked_gates_root(project_id, issue_id, attempt_id);
        Ok(
            super::path_is_regular_file(&root.join(format!("{gate_id}.json")))?
                || super::path_is_regular_file(
                    &root.join("resolved").join(format!("{gate_id}.json")),
                )?,
        )
    }

    pub fn resolve_failed_code_review_gate_idempotent(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(gate_id)?;
        let _arbitration =
            self.acquire_failed_code_review_recovery_arbitration(project_id, issue_id, attempt_id)?;
        let attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        self.ensure_failed_review_recovery_write_allowed(&attempt)?;
        let root = self.blocked_gates_root(project_id, issue_id, attempt_id);
        if super::path_is_regular_file(&root.join(format!("{gate_id}.json")))? {
            self.resolve_blocked_gate(project_id, issue_id, attempt_id, gate_id)?;
            return Ok(());
        }
        if super::path_is_regular_file(&root.join("resolved").join(format!("{gate_id}.json")))? {
            return Ok(());
        }
        Err(ProductStoreError::NotFound {
            kind: "coding_blocked_gate",
            id: gate_id.to_string(),
        })
    }

    pub fn complete_failed_code_review_recovery_journal(
        &self,
        attempt: &CodingExecutionAttempt,
        gate_id: &str,
    ) -> Result<FailedCodeReviewRecoveryJournal, ProductStoreError> {
        validate_relative_id(gate_id)?;
        let _arbitration = self.acquire_failed_code_review_recovery_arbitration(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let attempt = self.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        self.ensure_failed_review_recovery_write_allowed(&attempt)?;
        let Some(mut journal) = self.get_failed_code_review_recovery_journal(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?
        else {
            return Err(recovery_state_changed());
        };
        if journal.expected_gate_id != gate_id {
            return Err(recovery_state_changed());
        }
        if journal.is_completed() {
            return Ok(journal);
        }
        let Some(retry_role_run_id) = journal.retry_role_run_id.as_deref() else {
            return Err(recovery_state_changed());
        };
        if journal.phase < FailedCodeReviewRecoveryPhase::GateResolved
            || attempt.status != crate::product::coding_models::CodingAttemptStatus::Running
            || attempt.stage != CodingExecutionStage::CodeReview
            || !super::path_is_regular_file(
                &self
                    .blocked_gates_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
                    .join("resolved")
                    .join(format!("{gate_id}.json")),
            )?
        {
            return Err(recovery_state_changed());
        }
        let retry = self.get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            retry_role_run_id,
        )?;
        validate_retry_role_run(&retry, &journal)?;
        let now = Utc::now().to_rfc3339();
        journal.phase = FailedCodeReviewRecoveryPhase::Completed;
        journal.runner_started_at = Some(now.clone());
        journal.completed_at = Some(now.clone());
        journal.updated_at = now;
        self.save_failed_code_review_recovery_journal(&journal)?;
        Ok(journal)
    }

    pub(crate) fn reopen_failed_review_attempt_running(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let _arbitration = self.acquire_failed_code_review_recovery_arbitration(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let current = self.validate_attempt_lineage(attempt)?;
        self.ensure_failed_review_recovery_write_allowed(&current)?;
        match current.status {
            CodingAttemptStatus::Blocked => self.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Running,
            ),
            CodingAttemptStatus::Running => Ok(current),
            _ => Err(recovery_state_changed()),
        }
    }

    fn ensure_failed_review_recovery_write_allowed(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        if self.plan_amendment_blocks_failed_review_recovery(attempt)? {
            return Err(ProductStoreError::Io(
                "plan_amendment_blocks_provider_run".to_string(),
            ));
        }
        Ok(())
    }

    fn plan_amendment_blocks_failed_review_recovery(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<bool, ProductStoreError> {
        if matches!(
            attempt.status,
            CodingAttemptStatus::AwaitingPlanAmendment
                | CodingAttemptStatus::ApplyingPlanAmendment
                | CodingAttemptStatus::AmendmentApplyFailed
        ) {
            return Ok(true);
        }
        let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
            return Ok(false);
        };
        let plan = WorkItemRevisionStore::new(self.paths()).get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            plan_id,
        )?;
        Ok(plan.active_amendment_id.is_some())
    }

    fn save_failed_code_review_recovery_journal(
        &self,
        journal: &FailedCodeReviewRecoveryJournal,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&journal.attempt_id)?;
        validate_relative_id(&journal.project_id)?;
        validate_relative_id(&journal.issue_id)?;
        let path = self.failed_code_review_recovery_journal_path(
            &journal.project_id,
            &journal.issue_id,
            &journal.attempt_id,
        )?;
        write_json(&path, journal)
    }
}

fn ensure_same_journal(
    current: &FailedCodeReviewRecoveryJournal,
    expected: &FailedCodeReviewRecoveryJournal,
) -> Result<(), ProductStoreError> {
    if current.attempt_id == expected.attempt_id
        && current.project_id == expected.project_id
        && current.issue_id == expected.issue_id
        && current.expected_gate_id == expected.expected_gate_id
        && current.expected_failed_node_id == expected.expected_failed_node_id
        && current.expected_stale_role_run_id == expected.expected_stale_role_run_id
        && current.recovery_key == expected.recovery_key
    {
        return Ok(());
    }
    Err(recovery_state_changed())
}

fn ensure_journal_attempt(
    journal: &FailedCodeReviewRecoveryJournal,
    attempt: &CodingExecutionAttempt,
) -> Result<(), ProductStoreError> {
    if journal.attempt_id == attempt.id
        && journal.project_id == attempt.project_id
        && journal.issue_id == attempt.issue_id
    {
        return Ok(());
    }
    Err(recovery_state_changed())
}

fn validate_retry_role_run(
    run: &CodingRoleRun,
    journal: &FailedCodeReviewRecoveryJournal,
) -> Result<(), ProductStoreError> {
    if run.attempt_id == journal.attempt_id
        && run.stage == CodingExecutionStage::CodeReview
        && run.role == CodingProviderRole::CodeReviewer
        && run.status == CodingRoleRunStatus::Running
        && is_failed_review_manual_retry(run, journal)
        && run.supersedes_run_id.as_deref() == Some(journal.expected_stale_role_run_id.as_str())
    {
        return Ok(());
    }
    Err(recovery_state_changed())
}

fn validate_stale_role_run(
    stale: &CodingRoleRun,
    journal: &FailedCodeReviewRecoveryJournal,
    retry_role_run_id: &str,
) -> Result<(), ProductStoreError> {
    if stale.attempt_id != journal.attempt_id
        || stale.stage != CodingExecutionStage::CodeReview
        || stale.role != CodingProviderRole::CodeReviewer
        || stale.node_id.as_deref() != Some(journal.expected_failed_node_id.as_str())
        || !matches!(
            stale.status,
            CodingRoleRunStatus::Running
                | CodingRoleRunStatus::Failed
                | CodingRoleRunStatus::Superseded
        )
        || (stale.status == CodingRoleRunStatus::Superseded
            && stale.superseded_by_run_id.as_deref() != Some(retry_role_run_id))
        || (stale.status == CodingRoleRunStatus::Failed
            && stale
                .superseded_by_run_id
                .as_deref()
                .is_some_and(|run_id| run_id != retry_role_run_id))
    {
        return Err(recovery_state_changed());
    }
    Ok(())
}

fn is_failed_review_manual_retry(
    run: &CodingRoleRun,
    journal: &FailedCodeReviewRecoveryJournal,
) -> bool {
    (run.trigger == CodingRoleRunTrigger::ManualRetry
        && run.retry_metadata.as_ref().is_some_and(|retry| {
            retry.cycle_id == run.id
                && retry.attempt_no == 1
                && retry.prior_run_id.as_deref()
                    == Some(journal.expected_stale_role_run_id.as_str())
        }))
        || (run.trigger == CodingRoleRunTrigger::RetryReview && run.retry_metadata.is_none())
}

fn recovery_state_changed() -> ProductStoreError {
    ProductStoreError::Io("coding_failed_review_recovery_state_changed".to_string())
}
