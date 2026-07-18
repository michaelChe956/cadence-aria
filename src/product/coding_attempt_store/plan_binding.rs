use std::collections::HashSet;

use crate::product::coding_models::{
    CodingAmendmentApplicationJournal, CodingAmendmentApplicationPhase, CodingAttemptPlanBinding,
    CodingAttemptScope, CodingExecutionAttempt,
};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::locking::with_exclusive_lock;

impl super::CodingAttemptStore {
    pub fn save_plan_binding(
        &self,
        attempt: &CodingExecutionAttempt,
        binding: &CodingAttemptPlanBinding,
    ) -> Result<(), ProductStoreError> {
        let stored_attempt = self.validate_attempt_lineage(attempt)?;
        validate_plan_binding(&stored_attempt, binding)?;
        let path = self.plan_binding_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        with_exclusive_lock(&path, || {
            if super::path_is_regular_file(&path)? {
                let existing: CodingAttemptPlanBinding = read_json(&path)?;
                validate_plan_binding(&stored_attempt, &existing)?;
                if same_binding_state(&existing, binding) {
                    return Ok(());
                }
                validate_binding_advance(&existing, binding)?;
            }
            write_json(&path, binding)
        })
    }

    pub fn get_plan_binding(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingAttemptPlanBinding, ProductStoreError> {
        let stored_attempt = self.validate_attempt_lineage(attempt)?;
        let path = self.plan_binding_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        if !super::path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_attempt_plan_binding",
                id: attempt.id.clone(),
            });
        }
        let binding = read_json(&path)?;
        validate_plan_binding(&stored_attempt, &binding)?;
        Ok(binding)
    }

    pub fn create_amendment_application_journal(
        &self,
        attempt: &CodingExecutionAttempt,
        journal: &CodingAmendmentApplicationJournal,
    ) -> Result<(), ProductStoreError> {
        self.validate_attempt_lineage(attempt)?;
        validate_journal(attempt, journal)?;
        let path = self.amendment_application_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &journal.amendment_id,
        );
        with_exclusive_lock(&path, || {
            if super::path_is_regular_file(&path)? {
                let existing: CodingAmendmentApplicationJournal = read_json(&path)?;
                if existing == *journal {
                    return Ok(());
                }
                return Err(identity_mismatch(
                    "coding_amendment_application_journal",
                    &journal.id,
                ));
            }
            write_json(&path, journal)
        })
    }

    pub fn get_amendment_application_journal(
        &self,
        attempt: &CodingExecutionAttempt,
        amendment_id: &str,
    ) -> Result<CodingAmendmentApplicationJournal, ProductStoreError> {
        self.validate_attempt_lineage(attempt)?;
        validate_relative_id(amendment_id)?;
        let path = self.amendment_application_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            amendment_id,
        );
        if !super::path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_amendment_application_journal",
                id: amendment_id.to_string(),
            });
        }
        let journal = read_json(&path)?;
        validate_journal(attempt, &journal)?;
        if journal.amendment_id != amendment_id {
            return Err(identity_mismatch(
                "coding_amendment_application_journal",
                amendment_id,
            ));
        }
        Ok(journal)
    }

    pub fn advance_amendment_application_journal(
        &self,
        attempt: &CodingExecutionAttempt,
        amendment_id: &str,
        phase: CodingAmendmentApplicationPhase,
        error: Option<String>,
        updated_at: String,
    ) -> Result<CodingAmendmentApplicationJournal, ProductStoreError> {
        self.validate_attempt_lineage(attempt)?;
        validate_relative_id(amendment_id)?;
        let path = self.amendment_application_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            amendment_id,
        );
        with_exclusive_lock(&path, || {
            if !super::path_is_regular_file(&path)? {
                return Err(ProductStoreError::NotFound {
                    kind: "coding_amendment_application_journal",
                    id: amendment_id.to_string(),
                });
            }
            let mut journal: CodingAmendmentApplicationJournal = read_json(&path)?;
            validate_journal(attempt, &journal)?;
            if journal.amendment_id != amendment_id || phase.order() < journal.phase.order() {
                return Err(identity_mismatch(
                    "coding_amendment_application_journal",
                    amendment_id,
                ));
            }
            if journal.phase == phase && journal.error == error {
                return Ok(journal);
            }
            journal.phase = phase;
            journal.error = error;
            journal.updated_at = updated_at;
            write_json(&path, &journal)?;
            Ok(journal)
        })
    }
}

fn validate_plan_binding(
    attempt: &CodingExecutionAttempt,
    binding: &CodingAttemptPlanBinding,
) -> Result<(), ProductStoreError> {
    for id in [
        binding.attempt_id.as_str(),
        binding.plan_id.as_str(),
        binding.bound_plan_revision_id.as_str(),
    ] {
        validate_relative_id(id)?;
    }
    let expected_plan_id = match (&attempt.scope, attempt.work_item_group_id.as_deref()) {
        (CodingAttemptScope::WorkItemGroup, Some(plan_id)) => plan_id,
        _ => {
            return Err(identity_mismatch(
                "coding_attempt_plan_binding",
                &binding.attempt_id,
            ));
        }
    };
    if binding.attempt_id != attempt.id || binding.plan_id != expected_plan_id {
        return Err(identity_mismatch(
            "coding_attempt_plan_binding",
            &binding.attempt_id,
        ));
    }
    let mut seen = HashSet::new();
    for amendment_id in &binding.applied_amendment_ids {
        validate_relative_id(amendment_id)?;
        if !seen.insert(amendment_id) {
            return Err(identity_mismatch(
                "coding_attempt_plan_binding",
                &binding.attempt_id,
            ));
        }
    }
    Ok(())
}

fn same_binding_state(left: &CodingAttemptPlanBinding, right: &CodingAttemptPlanBinding) -> bool {
    left.attempt_id == right.attempt_id
        && left.plan_id == right.plan_id
        && left.bound_plan_revision_id == right.bound_plan_revision_id
        && left.applied_amendment_ids == right.applied_amendment_ids
}

fn validate_binding_advance(
    existing: &CodingAttemptPlanBinding,
    next: &CodingAttemptPlanBinding,
) -> Result<(), ProductStoreError> {
    let extends_existing = next.applied_amendment_ids.len() > existing.applied_amendment_ids.len()
        && next.applied_amendment_ids[..existing.applied_amendment_ids.len()]
            == existing.applied_amendment_ids;
    if existing.attempt_id != next.attempt_id
        || existing.plan_id != next.plan_id
        || !extends_existing
        || existing.bound_plan_revision_id == next.bound_plan_revision_id
    {
        return Err(identity_mismatch(
            "coding_attempt_plan_binding",
            &next.attempt_id,
        ));
    }
    Ok(())
}

fn validate_journal(
    attempt: &CodingExecutionAttempt,
    journal: &CodingAmendmentApplicationJournal,
) -> Result<(), ProductStoreError> {
    for id in [
        journal.id.as_str(),
        journal.attempt_id.as_str(),
        journal.amendment_id.as_str(),
    ] {
        validate_relative_id(id)?;
    }
    if journal.attempt_id != attempt.id {
        return Err(identity_mismatch(
            "coding_amendment_application_journal",
            &journal.id,
        ));
    }
    Ok(())
}

fn identity_mismatch(kind: &'static str, id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind,
        id: id.to_string(),
    }
}
