use crate::product::json_store::{ProductStoreError, validate_relative_id};

use std::path::PathBuf;

use super::locking::{ExclusiveFileLock, canonical_lock_path_identity, canonical_path_identity};

pub struct WorkItemAttemptCreationGuard {
    project_id: String,
    issue_id: String,
    work_item_id: String,
    canonical_store_root: PathBuf,
    canonical_lock_path: PathBuf,
    _lock: ExclusiveFileLock,
}

impl WorkItemAttemptCreationGuard {
    pub(crate) fn validate_identity(
        &self,
        store: &super::CodingAttemptStore,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<(), ProductStoreError> {
        let expected_store_root = canonical_path_identity(store.paths.root())?;
        let expected_lock_path = canonical_lock_path_identity(
            &store.work_item_attempt_creation_path(project_id, issue_id, work_item_id),
        )?;
        if self.project_id == project_id
            && self.issue_id == issue_id
            && self.work_item_id == work_item_id
            && self.canonical_store_root == expected_store_root
            && self.canonical_lock_path == expected_lock_path
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
        let lock_target = self.work_item_attempt_creation_path(project_id, issue_id, work_item_id);
        let lock = ExclusiveFileLock::acquire(&lock_target)?;
        self.work_item_attempt_creation_guard(project_id, issue_id, work_item_id, lock)
    }

    pub async fn acquire_work_item_attempt_creation_async(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<WorkItemAttemptCreationGuard, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        let lock_target = self.work_item_attempt_creation_path(project_id, issue_id, work_item_id);
        let lock = ExclusiveFileLock::acquire_async(&lock_target).await?;
        self.work_item_attempt_creation_guard(project_id, issue_id, work_item_id, lock)
    }

    fn work_item_attempt_creation_guard(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        lock: ExclusiveFileLock,
    ) -> Result<WorkItemAttemptCreationGuard, ProductStoreError> {
        Ok(WorkItemAttemptCreationGuard {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            work_item_id: work_item_id.to_string(),
            canonical_store_root: canonical_path_identity(self.paths.root())?,
            canonical_lock_path: lock.canonical_lock_path().to_path_buf(),
            _lock: lock,
        })
    }
}
