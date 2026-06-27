use super::*;
use crate::product::coding_models::{CodingAttemptScope, CodingExecutionUnitStatus};

impl CodingWorkspaceEngine {
    pub async fn complete_current_group_unit(
        &self,
        attempt: &CodingExecutionAttempt,
        summary: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Ok(attempt.clone());
        }

        let active = self
            .store
            .get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .ok_or_else(|| {
                CodingWorkspaceEngineError::WorkItemHandoffMissing(attempt.id.clone())
            })?;
        self.store.update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &active.id,
            CodingExecutionUnitStatus::Completed,
            summary,
        )?;

        self.advance_to_next_group_unit(attempt).await
    }

    pub async fn advance_to_next_group_unit(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;

        if let Some(next) = units
            .iter()
            .filter(|unit| unit.status == CodingExecutionUnitStatus::Pending)
            .min_by_key(|unit| unit.order_index)
        {
            self.store.update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &next.id,
                CodingExecutionUnitStatus::Running,
                Some("进入下一个 Work Item".to_string()),
            )?;

            let mut updated =
                self.store
                    .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
            updated.current_work_item_id = Some(next.work_item_id.clone());
            updated.active_unit_id = Some(next.id.clone());
            updated.stage = CodingExecutionStage::PrepareContext;
            updated.status = CodingAttemptStatus::Running;
            updated.updated_at = Utc::now().to_rfc3339();
            self.store.save_coding_attempt(&updated)?;
            return Ok(updated);
        }

        self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )?;
        self.store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .map_err(CodingWorkspaceEngineError::from)
    }

    pub fn group_attempt_ready_for_final_review(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<bool, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Ok(false);
        }

        Ok(self
            .store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .iter()
            .all(|unit| unit.status == CodingExecutionUnitStatus::Completed))
    }
}
