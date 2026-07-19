use chrono::Utc;

use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
};
use crate::product::json_store::{ProductStoreError, write_json};
use crate::product::models::{AmendmentResumeMode, PlanAmendmentManifest};

use super::locking::with_exclusive_lock;

impl super::CodingAttemptStore {
    pub fn resume_attempt_after_amendment(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        let await_handoff = manifest.resume_target.mode == AmendmentResumeMode::AwaitHandoff;
        if !(matches!(
            current.status,
            CodingAttemptStatus::ApplyingPlanAmendment
                | CodingAttemptStatus::AmendmentApplyFailed
                | CodingAttemptStatus::Running
        ) || await_handoff && current.status == CodingAttemptStatus::AwaitingPlanAmendment)
        {
            return Err(identity_mismatch(
                "coding_amendment_resume_attempt",
                &current.id,
            ));
        }
        let active_unit_id =
            current
                .active_unit_id
                .as_deref()
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "coding_amendment_resume_target",
                    id: manifest.resume_target.logical_work_item_id.clone(),
                })?;
        let target = self
            .list_coding_units(&current.project_id, &current.issue_id, &current.id)?
            .into_iter()
            .find(|unit| unit.id == active_unit_id)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "coding_amendment_resume_target",
                id: active_unit_id.to_string(),
            })?;
        if target.logical_work_item_id != manifest.resume_target.logical_work_item_id {
            return Err(identity_mismatch(
                "coding_amendment_resume_target",
                &target.id,
            ));
        }
        let path = self.attempt_path(&current.project_id, &current.issue_id, &current.id);
        with_exclusive_lock(&path, || {
            let mut latest =
                self.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if latest != current {
                return Err(identity_mismatch(
                    "coding_amendment_resume_attempt",
                    &current.id,
                ));
            }
            latest.status = if await_handoff {
                CodingAttemptStatus::AwaitingPlanAmendment
            } else {
                CodingAttemptStatus::Running
            };
            latest.stage = match manifest.resume_target.mode {
                AmendmentResumeMode::Reexecute | AmendmentResumeMode::AwaitHandoff => {
                    CodingExecutionStage::Coding
                }
                AmendmentResumeMode::Revalidate => CodingExecutionStage::Testing,
            };
            latest.current_work_item_id = Some(target.logical_work_item_id.clone());
            latest.active_unit_id = Some(target.id.clone());
            latest.completed_at = None;
            latest.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &latest)?;
            Ok(latest)
        })
    }
}

fn identity_mismatch(kind: &'static str, id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind,
        id: id.to_string(),
    }
}
