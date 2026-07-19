use crate::product::json_store::{ProductStoreError, validate_relative_id};

use super::locking::ExclusiveFileLock;

const AMENDMENT_APPLICATION_ARBITRATION_TARGET: &str = "amendment-application-arbitration";

pub(crate) struct AmendmentApplicationArbitrationGuard {
    _lock: ExclusiveFileLock,
}

impl super::CodingAttemptStore {
    pub(crate) async fn acquire_amendment_application_arbitration(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<AmendmentApplicationArbitrationGuard, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let target = self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join(AMENDMENT_APPLICATION_ARBITRATION_TARGET);
        let lock = tokio::task::spawn_blocking(move || ExclusiveFileLock::acquire(&target))
            .await
            .map_err(|error| {
                ProductStoreError::Io(format!(
                    "amendment application arbitration task failed: {error}"
                ))
            })??;
        Ok(AmendmentApplicationArbitrationGuard { _lock: lock })
    }
}
