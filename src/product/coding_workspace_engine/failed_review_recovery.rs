use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::product::coding_attempt_store::{
    CodingAttemptStore, FAILED_CODE_REVIEW_RECOVERY_JOURNAL_FILE, FailedCodeReviewRecoveryJournal,
    FailedCodeReviewRecoveryPhase,
};
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingExecutionUnitStatus, CodingGateRequired, CodingProviderRole, CodingReworkInstruction,
    CodingRoleRunStatus, CodingRoleRunTrigger, CodingTimelineNodeStatus,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, validate_relative_id};

use super::{CodingWorkspaceEngine, CodingWorkspaceEngineError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FailedCodeReviewRecovery {
    pub(crate) gate_id: String,
    pub(crate) failed_node_id: String,
    pub(crate) stale_role_run_id: String,
}

pub(crate) fn recoverable_failed_code_review(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<Option<FailedCodeReviewRecovery>, CodingWorkspaceEngineError> {
    if let Some(journal) = coding_store.get_failed_code_review_recovery_journal(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )? {
        if !journal.is_completed() {
            if !journal_recovery_prefix_is_valid(coding_store, attempt, &journal)? {
                return Ok(None);
            }
            return Ok(Some(FailedCodeReviewRecovery {
                gate_id: journal.expected_gate_id,
                failed_node_id: journal.expected_failed_node_id,
                stale_role_run_id: journal.expected_stale_role_run_id,
            }));
        }
        if completed_journal_waits_for_retry_node(coding_store, attempt, &journal)? {
            return Ok(Some(FailedCodeReviewRecovery {
                gate_id: journal.expected_gate_id,
                failed_node_id: journal.expected_failed_node_id,
                stale_role_run_id: journal.expected_stale_role_run_id,
            }));
        }
    }

    if attempt.status == CodingAttemptStatus::Blocked
        && attempt.stage == CodingExecutionStage::CodeReview
        && attempt.completed_at.is_none()
        && attempt_execution_fingerprint_is_valid(coding_store, attempt)?
    {
        let mut gates = coding_store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .filter(is_code_review_provider_interrupted_gate);
        let Some(gate) = gates.next() else {
            return Ok(None);
        };
        if gates.next().is_some() {
            return Ok(None);
        }
        let Some(failed_node_id) = coding_store.open_blocked_gate_node_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
        )?
        else {
            return Ok(None);
        };
        let failed_node_matches = coding_store
            .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .any(|node| {
                node.id == failed_node_id
                    && node.stage == CodingExecutionStage::CodeReview
                    && node.status == CodingTimelineNodeStatus::Failed
            });
        if !failed_node_matches {
            return Ok(None);
        }
        let mut failed_runs = coding_store
            .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .filter(|run| {
                run.stage == CodingExecutionStage::CodeReview
                    && run.role == CodingProviderRole::CodeReviewer
                    && run.node_id.as_deref() == Some(failed_node_id.as_str())
                    && run.status == CodingRoleRunStatus::Failed
                    && run.reason_code.as_deref() == Some("code_review_provider_interrupted")
            });
        let Some(failed_run) = failed_runs.next() else {
            return Ok(None);
        };
        if failed_runs.next().is_some() {
            return Ok(None);
        }
        return Ok(Some(FailedCodeReviewRecovery {
            gate_id: gate.gate_id,
            failed_node_id,
            stale_role_run_id: failed_run.id,
        }));
    }

    if attempt.status != CodingAttemptStatus::Failed
        || attempt.stage != CodingExecutionStage::CodeReview
        || attempt.completed_at.is_none()
    {
        return Ok(None);
    }
    if !attempt_execution_fingerprint_is_valid(coding_store, attempt)? {
        return Ok(None);
    }

    let Some(failed_node) = coding_store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .rev()
        .find(|node| node.stage == CodingExecutionStage::CodeReview)
    else {
        return Ok(None);
    };
    if failed_node.status != CodingTimelineNodeStatus::Failed {
        return Ok(None);
    }

    let Some(stale_role_run) = coding_store.latest_role_run(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        CodingExecutionStage::CodeReview,
        CodingProviderRole::CodeReviewer,
    )?
    else {
        return Ok(None);
    };
    if stale_role_run.status != CodingRoleRunStatus::Running
        || stale_role_run.node_id.as_deref() != Some(failed_node.id.as_str())
    {
        return Ok(None);
    }

    let Some(dirty_gate) = coding_store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .find(|gate| gate.reason_code.as_deref() == Some("shared_worktree_dirty_manual_gate"))
    else {
        return Ok(None);
    };

    Ok(Some(FailedCodeReviewRecovery {
        gate_id: dirty_gate.gate_id,
        failed_node_id: failed_node.id,
        stale_role_run_id: stale_role_run.id,
    }))
}

impl CodingWorkspaceEngine {
    pub(crate) fn send_interrupted_code_review_to_coder(
        &self,
        current: &CodingExecutionAttempt,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if current.stage != CodingExecutionStage::CodeReview
            || current.status != CodingAttemptStatus::Blocked
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "send_to_coder_not_available".to_string(),
            ));
        }

        let operator_context = extra_context
            .map(|content| content.trim().to_string())
            .filter(|content| !content.is_empty())
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "coding_gate_extra_context_required".to_string(),
                )
            })?;
        self.store
            .create_context_note(&current.id, operator_context.clone())?;

        let existing = self.store.list_rework_instructions(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        self.store
            .save_rework_instruction(&CodingReworkInstruction {
                id: next_sequential_id("coding_rework_instruction", existing.len()),
                attempt_id: current.id.clone(),
                source_stage: CodingExecutionStage::CodeReview,
                rework_round: current.rework_count + 1,
                summary: operator_context,
                fix_hints: Vec::new(),
                questions: Vec::new(),
                created_at: Utc::now().to_rfc3339(),
                consumed_by_node_id: None,
                consumed_at: None,
            })?;

        let running = self.store.update_attempt_status(
            &current.project_id,
            &current.issue_id,
            &current.id,
            CodingAttemptStatus::Running,
        )?;
        let coding_attempt = self.store.update_attempt_stage(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::Coding,
        )?;
        self.store
            .increment_attempt_rework_count(
                &coding_attempt.project_id,
                &coding_attempt.issue_id,
                &coding_attempt.id,
            )
            .map_err(CodingWorkspaceEngineError::from)
    }

    pub async fn recover_failed_code_review(
        &self,
        gate_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let candidates = failed_review_gate_attempts(&self.store, gate_id)?;
        let mut recoverable = Vec::new();
        for candidate in candidates {
            let current = self.store.get_attempt(
                &candidate.project_id,
                &candidate.issue_id,
                &candidate.id,
            )?;
            if matches!(
                recoverable_failed_code_review(&self.store, &current)?,
                Some(recovery) if recovery.gate_id == gate_id
            ) {
                recoverable.push(current);
            }
        }
        if recoverable.len() != 1 {
            return Err(recovery_state_changed());
        }
        self.recover_failed_code_review_for_attempt(&recoverable[0].id, gate_id)
            .await
    }

    pub(crate) async fn recover_failed_code_review_for_attempt(
        &self,
        attempt_id: &str,
        gate_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        validate_relative_id(gate_id)?;
        let located = self.store.get_attempt_by_id(attempt_id)?;
        let current =
            self.store
                .get_attempt(&located.project_id, &located.issue_id, &located.id)?;
        let Some(recovery) = recoverable_failed_code_review(&self.store, &current)? else {
            return Err(recovery_state_changed());
        };
        if recovery.gate_id != gate_id {
            return Err(recovery_state_changed());
        }
        let mut journal = self.store.prepare_failed_code_review_recovery_journal(
            &current,
            &recovery.gate_id,
            &recovery.failed_node_id,
            &recovery.stale_role_run_id,
        )?;

        let current =
            self.store
                .get_attempt(&journal.project_id, &journal.issue_id, &journal.attempt_id)?;
        if !matches!(
            recoverable_failed_code_review(&self.store, &current)?,
            Some(recovery)
                if recovery.gate_id == journal.expected_gate_id
                    && recovery.failed_node_id == journal.expected_failed_node_id
                    && recovery.stale_role_run_id == journal.expected_stale_role_run_id
        ) {
            return Err(recovery_state_changed());
        }

        let reopened = match current.status {
            CodingAttemptStatus::Failed => self.store.reopen_failed_code_review_attempt(
                &current.project_id,
                &current.issue_id,
                &current.id,
            )?,
            CodingAttemptStatus::Blocked | CodingAttemptStatus::Running => current,
            _ => return Err(recovery_state_changed()),
        };
        journal = self.store.advance_failed_code_review_recovery_journal(
            &journal,
            FailedCodeReviewRecoveryPhase::AttemptReopened,
            None,
        )?;

        let retry = self
            .store
            .ensure_failed_code_review_retry_role_run(&reopened, &journal)?;
        journal = self.store.advance_failed_code_review_recovery_journal(
            &journal,
            FailedCodeReviewRecoveryPhase::RetryRunCreated,
            Some(&retry.id),
        )?;

        let current =
            self.store
                .get_attempt(&journal.project_id, &journal.issue_id, &journal.attempt_id)?;
        let running = match current.status {
            CodingAttemptStatus::Blocked => self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Running,
            )?,
            CodingAttemptStatus::Running => current,
            _ => return Err(recovery_state_changed()),
        };
        journal = self.store.advance_failed_code_review_recovery_journal(
            &journal,
            FailedCodeReviewRecoveryPhase::AttemptRunning,
            Some(&retry.id),
        )?;

        self.store.resolve_failed_code_review_gate_idempotent(
            &journal.project_id,
            &journal.issue_id,
            &journal.attempt_id,
            &journal.expected_gate_id,
        )?;
        self.store.advance_failed_code_review_recovery_journal(
            &journal,
            FailedCodeReviewRecoveryPhase::GateResolved,
            Some(&retry.id),
        )?;
        Ok(running)
    }
}

pub(super) fn is_code_review_provider_interrupted_gate(gate: &CodingGateRequired) -> bool {
    gate.reason_code.as_deref() == Some("code_review_provider_interrupted")
        && gate.stage == Some(CodingExecutionStage::CodeReview)
        && gate.role == Some(CodingProviderRole::CodeReviewer)
}

fn failed_review_gate_attempts(
    coding_store: &CodingAttemptStore,
    gate_id: &str,
) -> Result<Vec<CodingExecutionAttempt>, ProductStoreError> {
    validate_relative_id(gate_id)?;
    let mut attempts = Vec::new();
    for project_path in child_directories(&coding_store.paths().projects_root())? {
        let Some(project_id) = project_path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        for issue_path in child_directories(&project_path.join("issues"))? {
            let Some(issue_id) = issue_path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let attempts_root = issue_path.join("coding-attempts");
            for attempt_path in child_directories(&attempts_root)? {
                let Some(attempt_id) = attempt_path.file_name().and_then(|value| value.to_str())
                else {
                    continue;
                };
                let open_gate_exists = attempt_path
                    .join("blocked-gates")
                    .join(format!("{gate_id}.json"))
                    .is_file();
                let journal_exists = attempt_path
                    .join(FAILED_CODE_REVIEW_RECOVERY_JOURNAL_FILE)
                    .is_file();
                if !open_gate_exists && !journal_exists {
                    continue;
                }
                let attempt = coding_store.get_attempt(project_id, issue_id, attempt_id)?;
                let matching_journal = coding_store
                    .get_failed_code_review_recovery_journal(project_id, issue_id, attempt_id)?
                    .is_some_and(|journal| journal.expected_gate_id == gate_id);
                if open_gate_exists || matching_journal {
                    attempts.push(attempt);
                }
            }
        }
    }
    Ok(attempts)
}

fn completed_journal_waits_for_retry_node(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    journal: &FailedCodeReviewRecoveryJournal,
) -> Result<bool, CodingWorkspaceEngineError> {
    if journal.attempt_id != attempt.id
        || journal.project_id != attempt.project_id
        || journal.issue_id != attempt.issue_id
        || attempt.status != CodingAttemptStatus::Running
        || attempt.stage != CodingExecutionStage::CodeReview
        || attempt.completed_at.is_some()
        || !attempt_execution_fingerprint_is_valid(coding_store, attempt)?
    {
        return Ok(false);
    }
    let Some(retry_role_run_id) = journal.retry_role_run_id.as_deref() else {
        return Ok(false);
    };
    let retry = coding_store.get_role_run(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        retry_role_run_id,
    )?;
    if retry.stage != CodingExecutionStage::CodeReview
        || retry.role != CodingProviderRole::CodeReviewer
        || retry.status != CodingRoleRunStatus::Running
        || retry.trigger != CodingRoleRunTrigger::RetryReview
        || retry.node_id.is_some()
        || retry.supersedes_run_id.as_deref() != Some(journal.expected_stale_role_run_id.as_str())
        || retry.reason_code.as_deref() != Some(journal.recovery_key.as_str())
    {
        return Ok(false);
    }
    let stale = coding_store.get_role_run(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        &journal.expected_stale_role_run_id,
    )?;
    Ok(stale.stage == CodingExecutionStage::CodeReview
        && stale.role == CodingProviderRole::CodeReviewer
        && matches!(
            stale.status,
            CodingRoleRunStatus::Failed | CodingRoleRunStatus::Superseded
        )
        && stale.node_id.as_deref() == Some(journal.expected_failed_node_id.as_str())
        && stale.superseded_by_run_id.as_deref() == Some(retry.id.as_str()))
}

fn journal_recovery_prefix_is_valid(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    journal: &FailedCodeReviewRecoveryJournal,
) -> Result<bool, CodingWorkspaceEngineError> {
    if journal.attempt_id != attempt.id
        || journal.project_id != attempt.project_id
        || journal.issue_id != attempt.issue_id
        || attempt.stage != CodingExecutionStage::CodeReview
        || !matches!(
            attempt.status,
            CodingAttemptStatus::Failed
                | CodingAttemptStatus::Blocked
                | CodingAttemptStatus::Running
        )
        || (attempt.status == CodingAttemptStatus::Failed && attempt.completed_at.is_none())
        || (attempt.status != CodingAttemptStatus::Failed && attempt.completed_at.is_some())
        || !attempt_execution_fingerprint_is_valid(coding_store, attempt)?
    {
        return Ok(false);
    }

    let failed_node_matches = coding_store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .any(|node| {
            node.id == journal.expected_failed_node_id
                && node.stage == CodingExecutionStage::CodeReview
                && node.status == CodingTimelineNodeStatus::Failed
        });
    if !failed_node_matches {
        return Ok(false);
    }

    let stale = coding_store.get_role_run(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        &journal.expected_stale_role_run_id,
    )?;
    if stale.stage != CodingExecutionStage::CodeReview
        || stale.role != CodingProviderRole::CodeReviewer
        || stale.node_id.as_deref() != Some(journal.expected_failed_node_id.as_str())
        || !matches!(
            stale.status,
            CodingRoleRunStatus::Running
                | CodingRoleRunStatus::Failed
                | CodingRoleRunStatus::Superseded
        )
    {
        return Ok(false);
    }

    let retry_runs = coding_store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .filter(|run| run.reason_code.as_deref() == Some(journal.recovery_key.as_str()))
        .collect::<Vec<_>>();
    if retry_runs.len() > 1 {
        return Ok(false);
    }
    if let Some(retry) = retry_runs.first() {
        if retry.trigger != CodingRoleRunTrigger::RetryReview
            || retry.stage != CodingExecutionStage::CodeReview
            || retry.role != CodingProviderRole::CodeReviewer
            || retry.supersedes_run_id.as_deref()
                != Some(journal.expected_stale_role_run_id.as_str())
            || journal
                .retry_role_run_id
                .as_deref()
                .is_some_and(|expected| expected != retry.id)
            || (stale.status == CodingRoleRunStatus::Superseded
                && stale.superseded_by_run_id.as_deref() != Some(retry.id.as_str()))
            || (stale.status == CodingRoleRunStatus::Failed
                && stale
                    .superseded_by_run_id
                    .as_deref()
                    .is_some_and(|run_id| run_id != retry.id))
        {
            return Ok(false);
        }
    } else if stale.status == CodingRoleRunStatus::Superseded
        || stale.superseded_by_run_id.is_some()
        || journal.retry_role_run_id.is_some()
    {
        return Ok(false);
    }

    coding_store
        .failed_code_review_recovery_gate_exists(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &journal.expected_gate_id,
        )
        .map_err(Into::into)
}

fn attempt_execution_fingerprint_is_valid(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<bool, CodingWorkspaceEngineError> {
    match attempt.scope {
        CodingAttemptScope::WorkItem => {
            if attempt.active_unit_id.is_some() {
                return Ok(false);
            }
        }
        CodingAttemptScope::WorkItemGroup => {
            let Some(active_unit_id) = attempt.active_unit_id.as_deref() else {
                return Ok(false);
            };
            let Some(active_unit) = coding_store.get_active_coding_unit(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )?
            else {
                return Ok(false);
            };
            if active_unit.id != active_unit_id
                || active_unit.status != CodingExecutionUnitStatus::Running
            {
                return Ok(false);
            }
        }
    }
    Ok(attempt
        .worktree_path
        .as_deref()
        .is_some_and(|path| path.is_dir()))
}

fn child_directories(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut directories = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        if entry
            .file_type()
            .map_err(|error| {
                ProductStoreError::Io(format!("read {} type: {error}", entry.path().display()))
            })?
            .is_dir()
        {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

fn recovery_state_changed() -> CodingWorkspaceEngineError {
    CodingWorkspaceEngineError::ProviderStream(
        "coding_failed_review_recovery_state_changed".to_string(),
    )
}
