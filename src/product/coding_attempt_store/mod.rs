use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::json_store::{ProductStoreError, validate_relative_id};

mod amendment_arbitration;
mod amendment_delivery;
mod amendment_recovery;
mod attempt;
mod attempt_creation;
mod context;
mod gate;
mod git_operation;
mod group;
mod group_initialization;
mod group_review_store;
mod group_terminal;
mod group_validation;
mod inputs;
pub(crate) mod locking;
mod paths;
mod plan_binding;
pub(crate) mod plan_repair_reconcile;
mod recovery;
mod report;
mod role_run;
mod role_run_event;
mod timeline;
mod unit_run;
mod unit_run_amendment;
mod unit_run_handoff;
mod utils;

#[cfg(test)]
pub(crate) use amendment_delivery::register_plan_amendment_delivery_mark_failpoint;
pub use attempt_creation::WorkItemAttemptCreationGuard;
pub use git_operation::*;
pub use group_initialization::*;
pub use group_validation::*;
pub use inputs::*;
pub(crate) use recovery::FAILED_CODE_REVIEW_RECOVERY_JOURNAL_FILE;
pub use recovery::{FailedCodeReviewRecoveryJournal, FailedCodeReviewRecoveryPhase};
pub(crate) use utils::*;

#[derive(Debug, Clone)]
pub struct CodingAttemptStore {
    paths: ProductAppPaths,
}

impl CodingAttemptStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> ProductAppPaths {
        self.paths.clone()
    }

    fn validate_scoped_attempt_record(
        &self,
        attempt: &CodingExecutionAttempt,
        record_attempt_id: &str,
        kind: &'static str,
        record_id: &str,
    ) -> Result<(), ProductStoreError> {
        if record_attempt_id != attempt.id {
            return Err(ProductStoreError::IdentityMismatch {
                kind,
                id: record_id.to_string(),
            });
        }
        self.validate_attempt_lineage(attempt)?;
        Ok(())
    }

    pub(crate) fn validate_attempt_lineage(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(&attempt.project_id)?;
        validate_relative_id(&attempt.issue_id)?;
        validate_relative_id(&attempt.id)?;
        let stored = self.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        if stored.id != attempt.id
            || stored.project_id != attempt.project_id
            || stored.issue_id != attempt.issue_id
            || stored.work_item_id != attempt.work_item_id
            || stored.attempt_no != attempt.attempt_no
            || stored.scope != attempt.scope
            || stored.work_item_group_id != attempt.work_item_group_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_attempt",
                id: attempt.id.clone(),
            });
        }
        Ok(stored)
    }

    pub(crate) fn find_attempt_by_id(
        &self,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(attempt_id)?;
        let mut found = None;
        for project_path in child_directories(&self.paths.projects_root())? {
            let Some(project_id) = project_path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let issues_root = project_path.join("issues");
            for issue_path in child_directories(&issues_root)? {
                let Some(issue_id) = issue_path.file_name().and_then(|value| value.to_str()) else {
                    continue;
                };
                let path = self.attempt_path(project_id, issue_id, attempt_id);
                if !path_is_regular_file(&path)? {
                    continue;
                }
                if found.is_some() {
                    return Err(ProductStoreError::Ambiguous {
                        kind: "coding_attempt",
                        id: attempt_id.to_string(),
                    });
                }
                found = Some((project_id.to_string(), issue_id.to_string()));
            }
        }
        let (project_id, issue_id) = found.ok_or_else(|| ProductStoreError::NotFound {
            kind: "coding_attempt",
            id: attempt_id.to_string(),
        })?;
        self.get_attempt(&project_id, &issue_id, attempt_id)
    }
}

#[cfg(test)]
mod tests;
