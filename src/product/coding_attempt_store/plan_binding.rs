use std::collections::HashSet;

use chrono::Utc;

use crate::product::coding_models::{
    CodingAmendmentApplicationJournal, CodingAmendmentApplicationPhase, CodingAttemptPlanBinding,
    CodingAttemptScope, CodingExecutionAttempt,
};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::PlanAmendmentManifest;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

use super::locking::with_exclusive_lock;

impl super::CodingAttemptStore {
    pub fn update_plan_binding_from_manifest(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<CodingAttemptPlanBinding, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        let binding = self.get_plan_binding(&current)?;
        let revision_store = WorkItemRevisionStore::new(self.paths());
        let plan = revision_store.get_plan_lineage(
            &current.project_id,
            &current.issue_id,
            &binding.plan_id,
        )?;
        revision_store.with_active_amendment_identity(
            &plan,
            &manifest.id,
            &manifest.new_plan_revision_id,
            || {
                let path =
                    self.plan_binding_path(&current.project_id, &current.issue_id, &current.id);
                with_exclusive_lock(&path, || {
                    let mut stored: CodingAttemptPlanBinding = read_json(&path)?;
                    validate_plan_binding(&current, &stored)?;
                    match stored
                        .applied_amendment_ids
                        .iter()
                        .position(|id| id == &manifest.id)
                    {
                        Some(index)
                            if index + 1 == stored.applied_amendment_ids.len()
                                && stored.bound_plan_revision_id
                                    == manifest.new_plan_revision_id =>
                        {
                            return Ok(stored);
                        }
                        Some(_) => {
                            return Err(identity_mismatch(
                                "coding_attempt_plan_binding",
                                &current.id,
                            ));
                        }
                        None if stored.bound_plan_revision_id
                            == manifest.previous_plan_revision_id => {}
                        None => {
                            return Err(identity_mismatch(
                                "coding_attempt_plan_binding",
                                &current.id,
                            ));
                        }
                    }
                    stored.bound_plan_revision_id = manifest.new_plan_revision_id.clone();
                    stored.applied_amendment_ids.push(manifest.id.clone());
                    stored.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &stored)?;
                    Ok(stored)
                })
            },
        )
    }

    pub fn load_or_prepare_amendment_application(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<CodingAmendmentApplicationJournal, ProductStoreError> {
        match self.get_amendment_application_journal(attempt, &manifest.id) {
            Ok(journal) => return Ok(journal),
            Err(ProductStoreError::NotFound { .. }) => {}
            Err(error) => return Err(error),
        }
        let current = self.validate_attempt_lineage(attempt)?;
        let now = Utc::now().to_rfc3339();
        let journal = CodingAmendmentApplicationJournal {
            id: format!("coding_amendment_application_{}", manifest.id),
            attempt_id: current.id.clone(),
            amendment_id: manifest.id.clone(),
            materialization_head_commit: current.head_commit.clone(),
            phase: CodingAmendmentApplicationPhase::Started,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.create_amendment_application_journal(&current, &journal)?;
        self.get_amendment_application_journal(&current, &manifest.id)
    }

    pub fn list_amendment_application_journals(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Vec<CodingAmendmentApplicationJournal>, ProductStoreError> {
        self.validate_attempt_lineage(attempt)?;
        let mut journals: Vec<CodingAmendmentApplicationJournal> = super::list_json_records(
            &self.amendment_applications_root(&attempt.project_id, &attempt.issue_id, &attempt.id),
        )?;
        for journal in &journals {
            validate_journal(attempt, journal)?;
        }
        journals.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(journals)
    }

    pub fn mark_amendment_application_failed(
        &self,
        attempt: &CodingExecutionAttempt,
        amendment_id: &str,
        error: String,
    ) -> Result<CodingAmendmentApplicationJournal, ProductStoreError> {
        let journal = self.get_amendment_application_journal(attempt, amendment_id)?;
        self.advance_amendment_application_journal(
            attempt,
            amendment_id,
            journal.phase,
            Some(error),
            Utc::now().to_rfc3339(),
        )
    }

    pub fn clear_amendment_application_error(
        &self,
        attempt: &CodingExecutionAttempt,
        amendment_id: &str,
    ) -> Result<CodingAmendmentApplicationJournal, ProductStoreError> {
        let journal = self.get_amendment_application_journal(attempt, amendment_id)?;
        self.advance_amendment_application_journal(
            attempt,
            amendment_id,
            journal.phase,
            None,
            Utc::now().to_rfc3339(),
        )
    }

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
            let same_phase = journal.phase == phase;
            let adjacent_phase = phase.order() == journal.phase.order().saturating_add(1);
            if journal.amendment_id != amendment_id || (!same_phase && !adjacent_phase) {
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
    if journal.attempt_id != attempt.id
        || journal.id != format!("coding_amendment_application_{}", journal.amendment_id)
    {
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
