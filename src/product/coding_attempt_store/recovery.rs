use std::fs;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage, CodingProviderRole,
    CodingRoleRun, CodingRoleRunStatus, CodingRoleRunTrigger,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::locking::with_exclusive_lock;

pub(crate) const FAILED_CODE_REVIEW_RECOVERY_JOURNAL_FILE: &str =
    "failed-code-review-recovery.json";
const FAILED_CODE_REVIEW_RECOVERIES_DIR: &str = "failed-code-review-recoveries";
const COMPLETED_FAILED_CODE_REVIEW_RECOVERIES_DIR: &str = "completed";

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
        let attempt_path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let journal_path = self.failed_code_review_recovery_journal_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        with_exclusive_lock(&attempt_path, || {
            let current = self.validate_attempt_lineage(attempt)?;
            if matches!(
                current.status,
                CodingAttemptStatus::AwaitingPlanAmendment
                    | CodingAttemptStatus::ApplyingPlanAmendment
                    | CodingAttemptStatus::AmendmentApplyFailed
            ) {
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

    pub(crate) fn discard_prepared_failed_code_review_recovery_for_plan_amendment(
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
            return Ok(());
        }
        if journal.phase != FailedCodeReviewRecoveryPhase::Prepared
            || journal.retry_role_run_id.is_some()
            || journal.runner_started_at.is_some()
            || journal.completed_at.is_some()
        {
            return Err(recovery_state_changed());
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
        ensure_journal_attempt(journal, attempt)?;
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
                id,
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                role: CodingProviderRole::CodeReviewer,
                run_no,
                status: CodingRoleRunStatus::Running,
                trigger: CodingRoleRunTrigger::RetryReview,
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
        let attempt = self.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
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
        && run.trigger == CodingRoleRunTrigger::RetryReview
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

fn recovery_state_changed() -> ProductStoreError {
    ProductStoreError::Io("coding_failed_review_recovery_state_changed".to_string())
}
