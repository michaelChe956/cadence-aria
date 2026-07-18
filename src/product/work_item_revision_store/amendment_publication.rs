use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    PlanAmendmentPublicationJournal, PlanAmendmentPublicationPhase, WorkItemPlanLineage,
};

use super::{
    InitialWorkItemPublicationIds, WorkItemRevisionStore, identity_mismatch, json_file_paths,
    path_exists, read_required_json, with_exclusive_lock, write_immutable,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmendmentPublicationIds {
    pub journal_id: String,
    pub plan_revision_id: String,
    pub dependency_graph_revision_id: String,
    pub validation_report_id: String,
    pub plan_projection_bundle_id: String,
    pub work_items: BTreeMap<String, InitialWorkItemPublicationIds>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PlanAmendmentPublicationCheckpoint {
    JournalPreparing,
    FirstArtifactsWritten,
    JournalPrepared,
    ActivePlanRevisionPublished,
    JournalPlanPublished,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PlanAmendmentPublicationFailpointKey {
    journal_path: PathBuf,
    checkpoint: PlanAmendmentPublicationCheckpoint,
}

#[cfg(test)]
pub(crate) struct PlanAmendmentPublicationFailpointGuard {
    key: PlanAmendmentPublicationFailpointKey,
    registration_id: u64,
}

#[cfg(test)]
static PLAN_AMENDMENT_PUBLICATION_FAILPOINTS: OnceLock<
    Mutex<HashMap<PlanAmendmentPublicationFailpointKey, u64>>,
> = OnceLock::new();
#[cfg(test)]
static NEXT_PLAN_AMENDMENT_PUBLICATION_FAILPOINT_ID: AtomicU64 = AtomicU64::new(1);

impl WorkItemRevisionStore {
    pub fn allocate_plan_amendment_publication_ids(
        &self,
        plan: &WorkItemPlanLineage,
        amendment_id: &str,
        next_revision_no: u32,
        revised_logical_ids: &[String],
    ) -> Result<PlanAmendmentPublicationIds, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(amendment_id)?;
        if next_revision_no == 0 || revised_logical_ids.is_empty() {
            return Err(identity_mismatch(
                "plan_amendment_publication",
                amendment_id,
            ));
        }
        let unique = revised_logical_ids.iter().collect::<BTreeSet<_>>();
        if unique.len() != revised_logical_ids.len() {
            return Err(identity_mismatch(
                "plan_amendment_publication",
                amendment_id,
            ));
        }
        for logical_id in revised_logical_ids {
            validate_relative_id(logical_id)?;
        }
        let scoped = |prefix: &str, logical_id: Option<&str>| match logical_id {
            Some(logical_id) => format!("{prefix}_{logical_id}_{next_revision_no:04}"),
            None => format!("{prefix}_{next_revision_no:04}"),
        };
        Ok(PlanAmendmentPublicationIds {
            journal_id: format!("{amendment_id}_publication_journal"),
            plan_revision_id: scoped("plan_revision", None),
            dependency_graph_revision_id: scoped("dependency_graph_revision", None),
            validation_report_id: scoped("plan_validation_report", None),
            plan_projection_bundle_id: scoped("plan_projection_bundle", None),
            work_items: revised_logical_ids
                .iter()
                .map(|logical_id| {
                    (
                        logical_id.clone(),
                        InitialWorkItemPublicationIds {
                            work_item_revision_id: scoped("work_item_revision", Some(logical_id)),
                            verification_plan_revision_id: scoped(
                                "verification_plan_revision",
                                Some(logical_id),
                            ),
                            work_item_projection_bundle_id: scoped(
                                "work_item_projection_bundle",
                                Some(logical_id),
                            ),
                        },
                    )
                })
                .collect(),
        })
    }

    pub fn put_plan_amendment_publication_journal(
        &self,
        plan: &WorkItemPlanLineage,
        value: &PlanAmendmentPublicationJournal,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_journal(plan, value)?;
        write_immutable(
            &self.amendment_publication_journal_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &value.id,
            ),
            "plan_amendment_publication_journal",
            &value.id,
            value,
        )
    }

    pub fn advance_plan_amendment_publication(
        &self,
        plan: &WorkItemPlanLineage,
        journal_id: &str,
        next: PlanAmendmentPublicationPhase,
    ) -> Result<PlanAmendmentPublicationJournal, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(journal_id)?;
        let path = self.amendment_publication_journal_path(
            &plan.project_id,
            &plan.issue_id,
            &plan.id,
            journal_id,
        );
        with_exclusive_lock(&path, || {
            let mut journal = self.get_plan_amendment_publication_journal(plan, journal_id)?;
            if journal.phase == next {
                if journal.error.is_some() || journal.recovery.is_some() {
                    journal.error = None;
                    journal.recovery = None;
                    journal.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &journal)?;
                }
                return Ok(journal);
            }
            if journal.phase == PlanAmendmentPublicationPhase::PlanPublished
                || next.order() <= journal.phase.order()
            {
                return Err(ProductStoreError::Io(format!(
                    "amendment_phase_regression: {:?} -> {:?}",
                    journal.phase, next
                )));
            }
            journal.phase = next;
            journal.error = None;
            journal.recovery = None;
            journal.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &journal)?;
            Ok(journal)
        })
    }

    pub fn mark_plan_amendment_publication_failed(
        &self,
        plan: &WorkItemPlanLineage,
        journal_id: &str,
        error: String,
    ) -> Result<PlanAmendmentPublicationJournal, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(journal_id)?;
        let path = self.amendment_publication_journal_path(
            &plan.project_id,
            &plan.issue_id,
            &plan.id,
            journal_id,
        );
        with_exclusive_lock(&path, || {
            let mut journal = self.get_plan_amendment_publication_journal(plan, journal_id)?;
            journal.recovery = Some(format!("resume_from_{:?}", journal.phase));
            journal.error = Some(error);
            journal.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &journal)?;
            Ok(journal)
        })
    }

    pub fn get_plan_amendment_publication_journal(
        &self,
        plan: &WorkItemPlanLineage,
        journal_id: &str,
    ) -> Result<PlanAmendmentPublicationJournal, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(journal_id)?;
        let value: PlanAmendmentPublicationJournal = read_required_json(
            &self.amendment_publication_journal_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                journal_id,
            ),
            "plan_amendment_publication_journal",
            journal_id,
        )?;
        validate_journal(plan, &value)?;
        if value.id != journal_id {
            return Err(identity_mismatch(
                "plan_amendment_publication_journal",
                journal_id,
            ));
        }
        Ok(value)
    }

    pub fn find_plan_amendment_publication_journal(
        &self,
        plan: &WorkItemPlanLineage,
        amendment_id: &str,
    ) -> Result<Option<PlanAmendmentPublicationJournal>, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(amendment_id)?;
        let mut matched = None;
        for path in json_file_paths(&self.amendment_publication_journals_root(
            &plan.project_id,
            &plan.issue_id,
            &plan.id,
        ))? {
            let value: PlanAmendmentPublicationJournal = read_json(&path)?;
            let file_id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| ProductStoreError::Io("invalid journal path".to_string()))?;
            validate_journal(plan, &value)?;
            if value.id != file_id {
                return Err(identity_mismatch(
                    "plan_amendment_publication_journal",
                    file_id,
                ));
            }
            if value.amendment_id == amendment_id {
                if matched.is_some() {
                    return Err(ProductStoreError::Ambiguous {
                        kind: "plan_amendment_publication_journal",
                        id: amendment_id.to_string(),
                    });
                }
                matched = Some(value);
            }
        }
        Ok(matched)
    }

    pub fn publish_or_resume_plan_amendment(
        &self,
        expected: &PlanAmendmentPublicationJournal,
    ) -> Result<PlanAmendmentPublicationJournal, ProductStoreError> {
        let plan = &expected
            .snapshot
            .as_ref()
            .ok_or_else(|| identity_mismatch("plan_amendment_publication_journal", &expected.id))?
            .lineage;
        validate_journal(plan, expected)?;
        let journal = self.prepare_plan_amendment_publication_journal(plan, expected)?;
        match self.replay_plan_amendment_publication(plan, &journal) {
            Ok(journal) => Ok(journal),
            Err(error) => {
                let _ = self.mark_plan_amendment_publication_failed(
                    plan,
                    &journal.id,
                    error.to_string(),
                );
                Err(error)
            }
        }
    }

    fn prepare_plan_amendment_publication_journal(
        &self,
        plan: &WorkItemPlanLineage,
        expected: &PlanAmendmentPublicationJournal,
    ) -> Result<PlanAmendmentPublicationJournal, ProductStoreError> {
        let path = self.amendment_publication_journal_path(
            &plan.project_id,
            &plan.issue_id,
            &plan.id,
            &expected.id,
        );
        with_exclusive_lock(&path, || {
            if path_exists(&path)? {
                let existing: PlanAmendmentPublicationJournal = read_json(&path)?;
                validate_journal(plan, &existing)?;
                if same_publication_identity(&existing, expected) {
                    return Ok(existing);
                }
                return Err(identity_mismatch(
                    "plan_amendment_publication_journal",
                    &expected.id,
                ));
            }
            write_json(&path, expected)?;
            Ok(expected.clone())
        })
    }

    fn replay_plan_amendment_publication(
        &self,
        plan: &WorkItemPlanLineage,
        journal: &PlanAmendmentPublicationJournal,
    ) -> Result<PlanAmendmentPublicationJournal, ProductStoreError> {
        #[cfg(test)]
        maybe_fail_plan_amendment_publication(
            self,
            journal,
            PlanAmendmentPublicationCheckpoint::JournalPreparing,
        )?;
        let mut current = self.get_plan_lineage(&plan.project_id, &plan.issue_id, &plan.id)?;
        if current.active_amendment_id.as_deref() != Some(journal.amendment_id.as_str())
            || !matches!(
                current.active_revision_id.as_deref(),
                Some(active)
                    if active == journal.base_plan_revision_id
                        || active == journal.new_plan_revision_id
            )
        {
            return Err(identity_mismatch("active_plan_amendment", &plan.id));
        }
        let snapshot = journal
            .snapshot
            .as_ref()
            .ok_or_else(|| identity_mismatch("plan_amendment_publication_journal", &journal.id))?;
        let mut phase = journal.phase.clone();
        if phase == PlanAmendmentPublicationPhase::Preparing {
            for (index, item) in snapshot.work_items.iter().enumerate() {
                self.put_logical_work_item(&current, &item.logical_work_item)?;
                self.put_draft_revision(&current, &item.draft_revision)?;
                self.put_verification_plan_revision(&current, &item.verification_plan_revision)?;
                self.put_work_item_projection_bundle(&current, &item.projection_bundle)?;
                self.put_work_item_revision(&current, &item.work_item_revision)?;
                #[cfg(test)]
                if index == 0 {
                    maybe_fail_plan_amendment_publication(
                        self,
                        journal,
                        PlanAmendmentPublicationCheckpoint::FirstArtifactsWritten,
                    )?;
                }
                #[cfg(not(test))]
                let _ = index;
            }
            self.put_dependency_graph_revision(&current, &snapshot.dependency_graph_revision)?;
            self.put_plan_projection_bundle(&current, &snapshot.plan_projection_bundle)?;
            self.put_plan_validation_report(&current, &snapshot.validation_report)?;
            self.put_plan_revision(&current, &snapshot.plan_revision)?;
            self.put_amendment_manifest(&current, &snapshot.manifest)?;
            let prepared = self.advance_plan_amendment_publication(
                &current,
                &journal.id,
                PlanAmendmentPublicationPhase::Prepared,
            )?;
            phase = prepared.phase;
            #[cfg(test)]
            maybe_fail_plan_amendment_publication(
                self,
                journal,
                PlanAmendmentPublicationCheckpoint::JournalPrepared,
            )?;
        }
        if phase == PlanAmendmentPublicationPhase::Prepared {
            current = self.publish_active_plan_amendment_revision(
                &current,
                &journal.amendment_id,
                &journal.base_plan_revision_id,
                &journal.new_plan_revision_id,
                &journal.updated_at,
            )?;
            #[cfg(test)]
            maybe_fail_plan_amendment_publication(
                self,
                journal,
                PlanAmendmentPublicationCheckpoint::ActivePlanRevisionPublished,
            )?;
            let published = self.advance_plan_amendment_publication(
                &current,
                &journal.id,
                PlanAmendmentPublicationPhase::PlanPublished,
            )?;
            #[cfg(test)]
            maybe_fail_plan_amendment_publication(
                self,
                journal,
                PlanAmendmentPublicationCheckpoint::JournalPlanPublished,
            )?;
            return Ok(published);
        }
        if current.active_revision_id.as_deref() != Some(journal.new_plan_revision_id.as_str()) {
            return Err(identity_mismatch(
                "active_work_item_plan_revision",
                &plan.id,
            ));
        }
        self.advance_plan_amendment_publication(
            &current,
            &journal.id,
            PlanAmendmentPublicationPhase::PlanPublished,
        )
    }
}

#[cfg(test)]
pub(crate) fn register_plan_amendment_publication_failpoint(
    store: &WorkItemRevisionStore,
    plan: &WorkItemPlanLineage,
    journal_id: &str,
    checkpoint: PlanAmendmentPublicationCheckpoint,
) -> PlanAmendmentPublicationFailpointGuard {
    let key = PlanAmendmentPublicationFailpointKey {
        journal_path: store.amendment_publication_journal_path(
            &plan.project_id,
            &plan.issue_id,
            &plan.id,
            journal_id,
        ),
        checkpoint,
    };
    let registration_id =
        NEXT_PLAN_AMENDMENT_PUBLICATION_FAILPOINT_ID.fetch_add(1, Ordering::Relaxed);
    let previous = plan_amendment_publication_failpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key.clone(), registration_id);
    assert!(
        previous.is_none(),
        "plan amendment publication failpoint already registered"
    );
    PlanAmendmentPublicationFailpointGuard {
        key,
        registration_id,
    }
}

#[cfg(test)]
fn maybe_fail_plan_amendment_publication(
    store: &WorkItemRevisionStore,
    journal: &PlanAmendmentPublicationJournal,
    checkpoint: PlanAmendmentPublicationCheckpoint,
) -> Result<(), ProductStoreError> {
    let key = PlanAmendmentPublicationFailpointKey {
        journal_path: store.amendment_publication_journal_path(
            &journal.project_id,
            &journal.issue_id,
            &journal.plan_id,
            &journal.id,
        ),
        checkpoint,
    };
    if plan_amendment_publication_failpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&key)
        .is_some()
    {
        return Err(ProductStoreError::Io(format!(
            "amendment_publication_failpoint:{checkpoint:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
fn plan_amendment_publication_failpoints()
-> &'static Mutex<HashMap<PlanAmendmentPublicationFailpointKey, u64>> {
    PLAN_AMENDMENT_PUBLICATION_FAILPOINTS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
impl Drop for PlanAmendmentPublicationFailpointGuard {
    fn drop(&mut self) {
        let mut failpoints = plan_amendment_publication_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failpoints.get(&self.key) == Some(&self.registration_id) {
            failpoints.remove(&self.key);
        }
    }
}

impl PlanAmendmentPublicationPhase {
    fn order(&self) -> u8 {
        match self {
            Self::Preparing => 0,
            Self::Prepared => 1,
            Self::PlanPublished => 2,
        }
    }
}

fn validate_journal(
    plan: &WorkItemPlanLineage,
    value: &PlanAmendmentPublicationJournal,
) -> Result<(), ProductStoreError> {
    for id in [
        &value.id,
        &value.project_id,
        &value.issue_id,
        &value.plan_id,
        &value.amendment_id,
        &value.request_id,
        &value.base_plan_revision_id,
        &value.new_plan_revision_id,
        &value.artifact_fingerprint,
    ] {
        validate_relative_id(id)?;
    }
    if value.project_id != plan.project_id
        || value.issue_id != plan.issue_id
        || value.plan_id != plan.id
        || value.confirmation.as_ref().is_some_and(|confirmation| {
            confirmation.amendment_id != value.amendment_id
                || confirmation.base_plan_revision_id != value.base_plan_revision_id
        })
        || value.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.lineage.id != plan.id
                || snapshot.plan_revision.id != value.new_plan_revision_id
                || snapshot.manifest.id != value.amendment_id
                || snapshot.manifest.repair_request_id != value.request_id
        })
    {
        return Err(identity_mismatch(
            "plan_amendment_publication_journal",
            &value.id,
        ));
    }
    Ok(())
}

fn same_publication_identity(
    left: &PlanAmendmentPublicationJournal,
    right: &PlanAmendmentPublicationJournal,
) -> bool {
    left.id == right.id
        && left.project_id == right.project_id
        && left.issue_id == right.issue_id
        && left.plan_id == right.plan_id
        && left.amendment_id == right.amendment_id
        && left.request_id == right.request_id
        && left.base_plan_revision_id == right.base_plan_revision_id
        && left.new_plan_revision_id == right.new_plan_revision_id
        && left.confirmation == right.confirmation
        && left.artifact_fingerprint == right.artifact_fingerprint
        && left.snapshot == right.snapshot
        && left.created_at == right.created_at
}
