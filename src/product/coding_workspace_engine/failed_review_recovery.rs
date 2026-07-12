use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingExecutionUnitStatus, CodingProviderRole, CodingRoleRunStatus, CodingTimelineNodeStatus,
};

use super::CodingWorkspaceEngineError;

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
