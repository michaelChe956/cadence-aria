use chrono::Utc;

use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionUnitStatus,
};
use crate::product::json_store::{ProductStoreError, write_json};

use super::group_validation::incomplete_group_attempt;

impl super::CodingAttemptStore {
    pub(super) fn update_group_terminal_status_locked(
        &self,
        mut attempt: CodingExecutionAttempt,
        status: CodingAttemptStatus,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let (stored, _authoritative, units) = self.validate_group_attempt_structure(&attempt)?;
        attempt = stored;
        if status == CodingAttemptStatus::Completed
            && units
                .iter()
                .any(|unit| unit.status != CodingExecutionUnitStatus::Completed)
        {
            return Err(incomplete_group_attempt(
                &attempt.id,
                "completed group attempt requires every coding unit to be completed",
            ));
        }

        let now = Utc::now().to_rfc3339();
        for mut unit in units {
            let normalized = normalized_terminal_unit_status(&unit.status, &status);
            if normalized == unit.status {
                continue;
            }
            unit.status = normalized;
            unit.completed_at = Some(now.clone());
            unit.updated_at = now.clone();
            write_json(
                &self.coding_unit_path(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &unit.id,
                ),
                &unit,
            )?;
        }

        attempt.status = status;
        attempt.active_unit_id = None;
        attempt.current_work_item_id = None;
        attempt.completed_at = Some(now.clone());
        attempt.updated_at = now;
        write_json(
            &self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            &attempt,
        )?;
        Ok(attempt)
    }
}

fn normalized_terminal_unit_status(
    current: &CodingExecutionUnitStatus,
    attempt_status: &CodingAttemptStatus,
) -> CodingExecutionUnitStatus {
    match attempt_status {
        CodingAttemptStatus::Aborted if !is_terminal_unit_status(current) => {
            CodingExecutionUnitStatus::Skipped
        }
        CodingAttemptStatus::Failed if current == &CodingExecutionUnitStatus::Pending => {
            CodingExecutionUnitStatus::Skipped
        }
        CodingAttemptStatus::Failed if !is_terminal_unit_status(current) => {
            CodingExecutionUnitStatus::Failed
        }
        _ => current.clone(),
    }
}

fn is_terminal_unit_status(status: &CodingExecutionUnitStatus) -> bool {
    matches!(
        status,
        CodingExecutionUnitStatus::Completed
            | CodingExecutionUnitStatus::Failed
            | CodingExecutionUnitStatus::Superseded
            | CodingExecutionUnitStatus::Skipped
    )
}
