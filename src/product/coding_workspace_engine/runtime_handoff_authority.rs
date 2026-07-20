use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionUnit, CodingUnitRun, CodingUnitRunStatus,
};
use crate::product::models::{HandoffRevision, WorkItemPlanLineage};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

use super::{CodingWorkspaceEngine, CodingWorkspaceEngineError};

#[derive(Debug, Clone)]
pub(super) struct AuthoritativeHandoffTransition {
    pub(super) previous: Option<HandoffRevision>,
    pub(super) next: HandoffRevision,
}

impl CodingWorkspaceEngine {
    pub(super) fn authoritative_handoff_run(
        &self,
        attempt: &CodingExecutionAttempt,
        handoff: &HandoffRevision,
    ) -> Result<CodingUnitRun, CodingWorkspaceEngineError> {
        let units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let unit = unique_authority_unit(&units, &handoff.logical_work_item_id)
            .map_err(|_| runtime_handoff_authority_conflict(&handoff.id))?;
        let mut runs = self
            .store
            .list_coding_unit_runs(attempt, &unit.id)?
            .into_iter()
            .filter(|run| run.id == handoff.coding_unit_run_id);
        let run = runs
            .next()
            .ok_or_else(|| runtime_handoff_authority_conflict(&handoff.id))?;
        if runs.next().is_some()
            || run.unit_id != unit.id
            || run.work_item_revision_id != handoff.work_item_revision_id
            || run.status != CodingUnitRunStatus::Completed
            || run.completion_commit.as_deref() != Some(handoff.commit_sha.as_str())
        {
            return Err(runtime_handoff_authority_conflict(&handoff.id));
        }
        Ok(run)
    }

    pub(super) fn authoritative_current_handoff_run(
        &self,
        attempt: &CodingExecutionAttempt,
        handoff: &HandoffRevision,
    ) -> Result<CodingUnitRun, CodingWorkspaceEngineError> {
        let units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let unit = unique_authority_unit(&units, &handoff.logical_work_item_id)
            .map_err(|_| runtime_handoff_authority_conflict(&handoff.id))?;
        if unit.latest_handoff_revision_id.as_deref() != Some(handoff.id.as_str()) {
            return Err(runtime_handoff_authority_conflict(&handoff.id));
        }
        let run = self.authoritative_handoff_run(attempt, handoff)?;
        let latest = self
            .store
            .list_coding_unit_runs(attempt, &unit.id)?
            .into_iter()
            .max_by_key(|candidate| candidate.execution_no)
            .ok_or_else(|| runtime_handoff_authority_conflict(&handoff.id))?;
        if latest != run {
            return Err(runtime_handoff_authority_conflict(&handoff.id));
        }
        Ok(run)
    }

    pub(super) fn authoritative_previous_handoff_for_run(
        &self,
        attempt: &CodingExecutionAttempt,
        lineage: &WorkItemPlanLineage,
        next_handoff: &HandoffRevision,
        next_run: &CodingUnitRun,
    ) -> Result<Option<HandoffRevision>, CodingWorkspaceEngineError> {
        let mut candidates = Vec::new();
        for handoff in WorkItemRevisionStore::new(self.store.paths())
            .list_handoff_revisions(lineage, &next_handoff.logical_work_item_id)?
        {
            if handoff.id == next_handoff.id {
                continue;
            }
            let Ok(run) = self.authoritative_handoff_run(attempt, &handoff) else {
                continue;
            };
            if run.execution_no < next_run.execution_no {
                candidates.push((run.execution_no, handoff));
            }
        }
        let Some(previous_execution_no) = candidates
            .iter()
            .map(|(execution_no, _)| *execution_no)
            .max()
        else {
            return Ok(None);
        };
        let mut previous = candidates
            .into_iter()
            .filter(|(execution_no, _)| *execution_no == previous_execution_no)
            .map(|(_, handoff)| handoff);
        let handoff = previous
            .next()
            .ok_or_else(|| runtime_handoff_authority_conflict(&next_handoff.id))?;
        if previous.next().is_some() {
            return Err(runtime_handoff_authority_conflict(&next_handoff.id));
        }
        Ok(Some(handoff))
    }

    pub(super) fn authoritative_handoff_transition(
        &self,
        attempt: &CodingExecutionAttempt,
        previous: Option<HandoffRevision>,
        next: HandoffRevision,
    ) -> Result<AuthoritativeHandoffTransition, CodingWorkspaceEngineError> {
        let next_run = self.authoritative_current_handoff_run(attempt, &next)?;
        if let Some(previous_handoff) = previous.as_ref() {
            if previous_handoff.logical_work_item_id != next.logical_work_item_id {
                return Err(runtime_handoff_authority_conflict(&next.id));
            }
            let previous_run = self.authoritative_handoff_run(attempt, previous_handoff)?;
            if previous_run.execution_no >= next_run.execution_no {
                return Err(runtime_handoff_authority_conflict(&next.id));
            }
        }
        Ok(AuthoritativeHandoffTransition { previous, next })
    }
}

fn unique_authority_unit<'a>(
    units: &'a [CodingExecutionUnit],
    logical_id: &str,
) -> Result<&'a CodingExecutionUnit, CodingWorkspaceEngineError> {
    let mut matches = units
        .iter()
        .filter(|unit| unit.logical_work_item_id == logical_id);
    let unit = matches.next().ok_or_else(|| {
        CodingWorkspaceEngineError::ProviderStream(format!(
            "runtime_handoff_unit_missing: {logical_id}"
        ))
    })?;
    if matches.next().is_some() {
        return Err(CodingWorkspaceEngineError::ProviderStream(format!(
            "runtime_handoff_unit_ambiguous: {logical_id}"
        )));
    }
    Ok(unit)
}

fn runtime_handoff_authority_conflict(handoff_id: &str) -> CodingWorkspaceEngineError {
    CodingWorkspaceEngineError::ProviderStream(format!(
        "runtime_handoff_authority_conflict: {handoff_id}"
    ))
}
