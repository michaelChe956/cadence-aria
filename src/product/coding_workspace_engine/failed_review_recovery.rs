use std::fs;
use std::path::{Path, PathBuf};

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingExecutionUnitStatus, CodingProviderRole, CodingRoleRunStatus, CodingRoleRunTrigger,
    CodingTimelineNodeStatus,
};
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
    if attempt.status != CodingAttemptStatus::Failed
        || attempt.stage != CodingExecutionStage::CodeReview
        || attempt.completed_at.is_none()
    {
        return Ok(None);
    }

    match attempt.scope {
        CodingAttemptScope::WorkItem => {
            if attempt.active_unit_id.is_some() {
                return Ok(None);
            }
        }
        CodingAttemptScope::WorkItemGroup => {
            let Some(active_unit_id) = attempt.active_unit_id.as_deref() else {
                return Ok(None);
            };
            let Some(active_unit) = coding_store.get_active_coding_unit(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )?
            else {
                return Ok(None);
            };
            if active_unit.id != active_unit_id
                || active_unit.status != CodingExecutionUnitStatus::Running
            {
                return Ok(None);
            }
        }
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

    let Some(worktree_path) = attempt.worktree_path.as_deref() else {
        return Ok(None);
    };
    if !worktree_path.is_dir() {
        return Ok(None);
    }

    Ok(Some(FailedCodeReviewRecovery {
        gate_id: dirty_gate.gate_id,
        failed_node_id: failed_node.id,
        stale_role_run_id: stale_role_run.id,
    }))
}

impl CodingWorkspaceEngine {
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

        let reopened = self.store.reopen_failed_code_review_attempt(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        self.store.supersede_latest_role_run_and_create(
            &reopened,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::RetryReview,
            None,
            Some("failed_code_review_recoverable".to_string()),
        )?;
        self.store.update_attempt_status(
            &reopened.project_id,
            &reopened.issue_id,
            &reopened.id,
            CodingAttemptStatus::Running,
        )?;
        self.store.resolve_blocked_gate(
            &reopened.project_id,
            &reopened.issue_id,
            &reopened.id,
            &recovery.gate_id,
        )?;
        Ok(self
            .store
            .get_attempt(&reopened.project_id, &reopened.issue_id, &reopened.id)?)
    }
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
                if attempt_path
                    .join("blocked-gates")
                    .join(format!("{gate_id}.json"))
                    .is_file()
                {
                    attempts.push(coding_store.get_attempt(project_id, issue_id, attempt_id)?);
                }
            }
        }
    }
    Ok(attempts)
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
