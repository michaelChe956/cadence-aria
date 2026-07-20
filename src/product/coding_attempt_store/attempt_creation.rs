use crate::product::json_store::{ProductStoreError, validate_relative_id};

use super::locking::ExclusiveFileLock;

pub struct WorkItemAttemptCreationGuard {
    project_id: String,
    issue_id: String,
    work_item_id: String,
    _lock: ExclusiveFileLock,
}

impl WorkItemAttemptCreationGuard {
    pub(crate) fn validate_identity(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<(), ProductStoreError> {
        if self.project_id == project_id
            && self.issue_id == issue_id
            && self.work_item_id == work_item_id
        {
            return Ok(());
        }
        Err(ProductStoreError::IdentityMismatch {
            kind: "work_item_attempt_creation_guard",
            id: work_item_id.to_string(),
        })
    }
}

impl super::CodingAttemptStore {
    pub fn acquire_work_item_attempt_creation(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<WorkItemAttemptCreationGuard, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        Ok(WorkItemAttemptCreationGuard {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            work_item_id: work_item_id.to_string(),
            _lock: ExclusiveFileLock::acquire(&self.work_item_attempt_creation_path(
                project_id,
                issue_id,
                work_item_id,
            ))?,
        })
    }
}
