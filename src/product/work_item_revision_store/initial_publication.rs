use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    DependencyGraphRevision, LogicalWorkItem, PlanProjectionBundle, PlanValidationReportArtifact,
    VerificationPlanRevision, WorkItemDraftRevision, WorkItemDraftRevisionStatus,
    WorkItemPlanLineage, WorkItemPlanRevision, WorkItemProjectionBundle, WorkItemRevision,
};

use super::{
    WorkItemRevisionStore, identity_mismatch, path_exists, read_required_json, with_exclusive_lock,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialWorkItemPublicationIds {
    pub work_item_revision_id: String,
    pub verification_plan_revision_id: String,
    pub work_item_projection_bundle_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialPlanPublicationIds {
    pub journal_id: String,
    pub plan_revision_id: String,
    pub dependency_graph_revision_id: String,
    pub validation_report_id: String,
    pub plan_projection_bundle_id: String,
    pub work_items: BTreeMap<String, InitialWorkItemPublicationIds>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialWorkItemPublicationArtifacts {
    pub logical_work_item: LogicalWorkItem,
    pub draft_revision: WorkItemDraftRevision,
    pub work_item_revision: WorkItemRevision,
    pub verification_plan_revision: VerificationPlanRevision,
    pub projection_bundle: WorkItemProjectionBundle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialPlanPublicationArtifacts {
    pub lineage: WorkItemPlanLineage,
    pub plan_revision: WorkItemPlanRevision,
    pub dependency_graph_revision: DependencyGraphRevision,
    pub validation_report: PlanValidationReportArtifact,
    pub plan_projection_bundle: PlanProjectionBundle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_provenance_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_provenance_content_hash: Option<String>,
    pub work_items: Vec<InitialWorkItemPublicationArtifacts>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InitialPlanPublicationPhase {
    Prepared,
    PlanActivated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialPlanPublicationJournal {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub compile_id: String,
    pub outline_version_ref: String,
    pub active_draft_revision_ids: BTreeMap<String, String>,
    pub allocated_ids: InitialPlanPublicationIds,
    pub artifact_fingerprint: String,
    pub artifacts: InitialPlanPublicationArtifacts,
    pub phase: InitialPlanPublicationPhase,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum InitialPlanPublicationCheckpoint {
    LineageWritten,
    FirstWorkItemArtifactsWritten,
    PlanArtifactsWritten,
    FirstWorkItemActivated,
    PlanActivated,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InitialPlanPublicationFailpointKey {
    journal_path: PathBuf,
    checkpoint: InitialPlanPublicationCheckpoint,
}

#[cfg(test)]
pub(crate) struct InitialPlanPublicationFailpointGuard {
    key: InitialPlanPublicationFailpointKey,
    registration_id: u64,
}

#[cfg(test)]
static INITIAL_PUBLICATION_FAILPOINTS: OnceLock<
    Mutex<HashMap<InitialPlanPublicationFailpointKey, u64>>,
> = OnceLock::new();
#[cfg(test)]
static NEXT_INITIAL_PUBLICATION_FAILPOINT_ID: AtomicU64 = AtomicU64::new(1);

impl WorkItemRevisionStore {
    pub fn allocate_initial_plan_publication_ids(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        compile_id: &str,
        logical_work_item_ids: &[String],
    ) -> Result<InitialPlanPublicationIds, ProductStoreError> {
        allocate_initial_plan_publication_ids(
            project_id,
            issue_id,
            plan_id,
            compile_id,
            logical_work_item_ids,
        )
    }
}

pub fn allocate_initial_plan_publication_ids(
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    compile_id: &str,
    logical_work_item_ids: &[String],
) -> Result<InitialPlanPublicationIds, ProductStoreError> {
    validate_relative_id(project_id)?;
    validate_relative_id(issue_id)?;
    validate_relative_id(plan_id)?;
    validate_relative_id(compile_id)?;
    if logical_work_item_ids.is_empty() {
        return Err(identity_mismatch("initial_plan_publication", compile_id));
    }
    let mut unique = BTreeSet::new();
    for logical_id in logical_work_item_ids {
        validate_relative_id(logical_id)?;
        if !unique.insert(logical_id.as_str()) {
            return Err(identity_mismatch(
                "initial_plan_publication_logical_work_item",
                logical_id,
            ));
        }
    }

    let scoped_id = |prefix: &str, logical_id: Option<&str>| {
        initial_publication_id(
            prefix, project_id, issue_id, plan_id, compile_id, logical_id,
        )
    };
    let work_items = logical_work_item_ids
        .iter()
        .map(|logical_id| {
            (
                logical_id.clone(),
                InitialWorkItemPublicationIds {
                    work_item_revision_id: scoped_id("work_item_revision", Some(logical_id)),
                    verification_plan_revision_id: scoped_id(
                        "verification_plan_revision",
                        Some(logical_id),
                    ),
                    work_item_projection_bundle_id: scoped_id(
                        "work_item_projection_bundle",
                        Some(logical_id),
                    ),
                },
            )
        })
        .collect();

    Ok(InitialPlanPublicationIds {
        journal_id: scoped_id("initial_plan_publication", None),
        plan_revision_id: scoped_id("plan_revision", None),
        dependency_graph_revision_id: scoped_id("dependency_graph_revision", None),
        validation_report_id: scoped_id("plan_validation_report", None),
        plan_projection_bundle_id: scoped_id("plan_projection_bundle", None),
        work_items,
    })
}

impl WorkItemRevisionStore {
    pub fn build_initial_plan_publication_journal(
        &self,
        compile_id: &str,
        outline_version_ref: &str,
        active_draft_revision_ids: BTreeMap<String, String>,
        publication_created_at: &str,
        artifacts: InitialPlanPublicationArtifacts,
    ) -> Result<InitialPlanPublicationJournal, ProductStoreError> {
        let logical_ids = artifacts
            .work_items
            .iter()
            .map(|item| item.logical_work_item.id.clone())
            .collect::<Vec<_>>();
        let allocated_ids = self.allocate_initial_plan_publication_ids(
            &artifacts.lineage.project_id,
            &artifacts.lineage.issue_id,
            &artifacts.lineage.id,
            compile_id,
            &logical_ids,
        )?;
        prepare_initial_plan_publication_journal(
            compile_id,
            outline_version_ref,
            active_draft_revision_ids,
            allocated_ids,
            publication_created_at,
            artifacts,
        )
    }

    pub fn publish_or_resume_initial_plan_revision(
        &self,
        expected: &InitialPlanPublicationJournal,
    ) -> Result<InitialPlanPublicationJournal, ProductStoreError> {
        validate_initial_publication_journal(expected)?;
        let journal = self.prepare_initial_plan_publication_journal(expected)?;
        match self.replay_initial_plan_publication(&journal) {
            Ok(()) => self.mark_initial_plan_publication_activated(&journal),
            Err(error) => {
                let _ = self.mark_initial_plan_publication_failed(&journal, error.to_string());
                Err(error)
            }
        }
    }

    pub fn get_initial_plan_publication_journal(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        compile_id: &str,
    ) -> Result<InitialPlanPublicationJournal, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        validate_relative_id(compile_id)?;
        let journal: InitialPlanPublicationJournal = read_required_json(
            &self.initial_plan_publication_journal_path(project_id, issue_id, plan_id, compile_id),
            "initial_plan_publication_journal",
            compile_id,
        )?;
        validate_initial_publication_journal(&journal)?;
        if journal.project_id != project_id
            || journal.issue_id != issue_id
            || journal.plan_id != plan_id
            || journal.compile_id != compile_id
        {
            return Err(identity_mismatch(
                "initial_plan_publication_journal",
                compile_id,
            ));
        }
        Ok(journal)
    }

    fn prepare_initial_plan_publication_journal(
        &self,
        expected: &InitialPlanPublicationJournal,
    ) -> Result<InitialPlanPublicationJournal, ProductStoreError> {
        let path = self.initial_plan_publication_journal_path(
            &expected.project_id,
            &expected.issue_id,
            &expected.plan_id,
            &expected.compile_id,
        );
        with_exclusive_lock(&path, || {
            if path_exists(&path)? {
                let existing: InitialPlanPublicationJournal = read_json(&path)?;
                validate_initial_publication_journal(&existing)?;
                if same_initial_publication_identity(&existing, expected) {
                    return Ok(existing);
                }
                return Err(identity_mismatch(
                    "initial_plan_publication_journal",
                    &expected.compile_id,
                ));
            }
            write_json(&path, expected)?;
            Ok(expected.clone())
        })
    }

    fn replay_initial_plan_publication(
        &self,
        journal: &InitialPlanPublicationJournal,
    ) -> Result<(), ProductStoreError> {
        let artifacts = &journal.artifacts;
        let lineage = self.ensure_initial_lineage(artifacts)?;
        #[cfg(test)]
        self.maybe_fail_initial_plan_publication(
            journal,
            InitialPlanPublicationCheckpoint::LineageWritten,
        )?;

        for (index, item) in artifacts.work_items.iter().enumerate() {
            self.ensure_initial_logical_work_item(&lineage, item)?;
            self.put_draft_revision_at(&lineage, &item.draft_revision, &journal.created_at)?;
            self.put_verification_plan_revision(&lineage, &item.verification_plan_revision)?;
            self.put_work_item_projection_bundle(&lineage, &item.projection_bundle)?;
            self.put_work_item_revision(&lineage, &item.work_item_revision)?;
            #[cfg(test)]
            if index == 0 {
                self.maybe_fail_initial_plan_publication(
                    journal,
                    InitialPlanPublicationCheckpoint::FirstWorkItemArtifactsWritten,
                )?;
            }
            #[cfg(not(test))]
            let _ = index;
        }
        self.put_dependency_graph_revision(&lineage, &artifacts.dependency_graph_revision)?;
        self.put_plan_projection_bundle(&lineage, &artifacts.plan_projection_bundle)?;
        self.put_plan_validation_report(&lineage, &artifacts.validation_report)?;
        self.put_plan_revision(&lineage, &artifacts.plan_revision)?;
        #[cfg(test)]
        self.maybe_fail_initial_plan_publication(
            journal,
            InitialPlanPublicationCheckpoint::PlanArtifactsWritten,
        )?;

        for (index, item) in artifacts.work_items.iter().enumerate() {
            self.update_draft_revision_state_at(
                &lineage,
                &item.draft_revision.id,
                WorkItemDraftRevisionStatus::Compiled,
                &journal.created_at,
            )?;
            self.set_initial_active_work_item_revision(
                &lineage,
                &item.logical_work_item,
                &item.work_item_revision.id,
                &journal.created_at,
            )?;
            #[cfg(test)]
            if index == 0 {
                self.maybe_fail_initial_plan_publication(
                    journal,
                    InitialPlanPublicationCheckpoint::FirstWorkItemActivated,
                )?;
            }
            #[cfg(not(test))]
            let _ = index;
        }
        self.set_initial_active_plan_revision(
            &lineage,
            &artifacts.plan_revision.id,
            &journal.created_at,
        )?;
        #[cfg(test)]
        self.maybe_fail_initial_plan_publication(
            journal,
            InitialPlanPublicationCheckpoint::PlanActivated,
        )?;
        Ok(())
    }

    fn ensure_initial_lineage(
        &self,
        artifacts: &InitialPlanPublicationArtifacts,
    ) -> Result<WorkItemPlanLineage, ProductStoreError> {
        match self.get_plan_lineage(
            &artifacts.lineage.project_id,
            &artifacts.lineage.issue_id,
            &artifacts.lineage.id,
        ) {
            Err(ProductStoreError::NotFound { .. }) => {
                self.put_plan_lineage(&artifacts.lineage)?;
                Ok(artifacts.lineage.clone())
            }
            Ok(existing) => {
                let mut normalized = existing.clone();
                if normalized.active_revision_id.as_deref()
                    == Some(artifacts.plan_revision.id.as_str())
                {
                    normalized.active_revision_id = None;
                }
                if normalized != artifacts.lineage {
                    return Err(identity_mismatch(
                        "work_item_plan_lineage",
                        &artifacts.lineage.id,
                    ));
                }
                Ok(existing)
            }
            Err(error) => Err(error),
        }
    }

    fn ensure_initial_logical_work_item(
        &self,
        lineage: &WorkItemPlanLineage,
        artifacts: &InitialWorkItemPublicationArtifacts,
    ) -> Result<(), ProductStoreError> {
        match self.get_logical_work_item(lineage, &artifacts.logical_work_item.id) {
            Err(ProductStoreError::NotFound { .. }) => {
                self.put_logical_work_item(lineage, &artifacts.logical_work_item)
            }
            Ok(existing) => {
                let mut normalized = existing;
                if normalized.active_revision_id.as_deref()
                    == Some(artifacts.work_item_revision.id.as_str())
                {
                    normalized.active_revision_id = None;
                }
                if normalized != artifacts.logical_work_item {
                    return Err(identity_mismatch(
                        "logical_work_item",
                        &artifacts.logical_work_item.id,
                    ));
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn mark_initial_plan_publication_activated(
        &self,
        expected: &InitialPlanPublicationJournal,
    ) -> Result<InitialPlanPublicationJournal, ProductStoreError> {
        self.update_initial_plan_publication_journal(expected, |journal| {
            journal.phase = InitialPlanPublicationPhase::PlanActivated;
            journal.error = None;
        })
    }

    fn mark_initial_plan_publication_failed(
        &self,
        expected: &InitialPlanPublicationJournal,
        error: String,
    ) -> Result<InitialPlanPublicationJournal, ProductStoreError> {
        self.update_initial_plan_publication_journal(expected, |journal| {
            journal.error = Some(error);
        })
    }

    fn update_initial_plan_publication_journal(
        &self,
        expected: &InitialPlanPublicationJournal,
        update: impl FnOnce(&mut InitialPlanPublicationJournal),
    ) -> Result<InitialPlanPublicationJournal, ProductStoreError> {
        let path = self.initial_plan_publication_journal_path(
            &expected.project_id,
            &expected.issue_id,
            &expected.plan_id,
            &expected.compile_id,
        );
        with_exclusive_lock(&path, || {
            let mut journal: InitialPlanPublicationJournal = read_json(&path)?;
            if !same_initial_publication_identity(&journal, expected) {
                return Err(identity_mismatch(
                    "initial_plan_publication_journal",
                    &expected.compile_id,
                ));
            }
            update(&mut journal);
            journal.updated_at = expected.created_at.clone();
            write_json(&path, &journal)?;
            Ok(journal)
        })
    }

    #[cfg(test)]
    pub(crate) fn register_initial_plan_publication_failpoint(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        compile_id: &str,
        checkpoint: InitialPlanPublicationCheckpoint,
    ) -> InitialPlanPublicationFailpointGuard {
        let key = InitialPlanPublicationFailpointKey {
            journal_path: self
                .initial_plan_publication_journal_path(project_id, issue_id, plan_id, compile_id),
            checkpoint,
        };
        let registration_id = NEXT_INITIAL_PUBLICATION_FAILPOINT_ID.fetch_add(1, Ordering::Relaxed);
        let previous = initial_publication_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key.clone(), registration_id);
        assert!(
            previous.is_none(),
            "initial publication failpoint already registered"
        );
        InitialPlanPublicationFailpointGuard {
            key,
            registration_id,
        }
    }

    #[cfg(test)]
    fn maybe_fail_initial_plan_publication(
        &self,
        journal: &InitialPlanPublicationJournal,
        checkpoint: InitialPlanPublicationCheckpoint,
    ) -> Result<(), ProductStoreError> {
        let key = InitialPlanPublicationFailpointKey {
            journal_path: self.initial_plan_publication_journal_path(
                &journal.project_id,
                &journal.issue_id,
                &journal.plan_id,
                &journal.compile_id,
            ),
            checkpoint,
        };
        if initial_publication_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&key)
            .is_some()
        {
            return Err(ProductStoreError::Io(format!(
                "initial_publication_failpoint:{checkpoint:?}"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
fn initial_publication_failpoints()
-> &'static Mutex<HashMap<InitialPlanPublicationFailpointKey, u64>> {
    INITIAL_PUBLICATION_FAILPOINTS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
impl Drop for InitialPlanPublicationFailpointGuard {
    fn drop(&mut self) {
        let mut failpoints = initial_publication_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failpoints.get(&self.key) == Some(&self.registration_id) {
            failpoints.remove(&self.key);
        }
    }
}

fn initial_publication_id(
    prefix: &str,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    compile_id: &str,
    logical_work_item_id: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    for component in [
        project_id,
        issue_id,
        plan_id,
        compile_id,
        logical_work_item_id.unwrap_or("plan"),
    ] {
        hasher.update(component.len().to_be_bytes());
        hasher.update(component.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    format!("{prefix}_{}", &digest[..24])
}

pub fn prepare_initial_plan_publication_journal(
    compile_id: &str,
    outline_version_ref: &str,
    active_draft_revision_ids: BTreeMap<String, String>,
    allocated_ids: InitialPlanPublicationIds,
    publication_created_at: &str,
    artifacts: InitialPlanPublicationArtifacts,
) -> Result<InitialPlanPublicationJournal, ProductStoreError> {
    validate_relative_id(compile_id)?;
    validate_relative_id(outline_version_ref)?;
    validate_initial_validation_report(&artifacts.validation_report)?;
    let journal = InitialPlanPublicationJournal {
        id: allocated_ids.journal_id.clone(),
        project_id: artifacts.lineage.project_id.clone(),
        issue_id: artifacts.lineage.issue_id.clone(),
        plan_id: artifacts.lineage.id.clone(),
        compile_id: compile_id.to_string(),
        outline_version_ref: outline_version_ref.to_string(),
        active_draft_revision_ids,
        allocated_ids,
        artifact_fingerprint: publication_fingerprint(&artifacts)?,
        artifacts,
        phase: InitialPlanPublicationPhase::Prepared,
        error: None,
        created_at: publication_created_at.to_string(),
        updated_at: publication_created_at.to_string(),
    };
    validate_initial_publication_journal(&journal)?;
    Ok(journal)
}

fn publication_fingerprint(
    artifacts: &InitialPlanPublicationArtifacts,
) -> Result<String, ProductStoreError> {
    let bytes = serde_json::to_vec(artifacts).map_err(|error| {
        ProductStoreError::Io(format!("serialize initial publication artifacts: {error}"))
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn same_initial_publication_identity(
    left: &InitialPlanPublicationJournal,
    right: &InitialPlanPublicationJournal,
) -> bool {
    left.id == right.id
        && left.project_id == right.project_id
        && left.issue_id == right.issue_id
        && left.plan_id == right.plan_id
        && left.compile_id == right.compile_id
        && left.outline_version_ref == right.outline_version_ref
        && left.active_draft_revision_ids == right.active_draft_revision_ids
        && left.allocated_ids == right.allocated_ids
        && left.artifact_fingerprint == right.artifact_fingerprint
        && left.artifacts == right.artifacts
        && left.created_at == right.created_at
}

fn validate_initial_publication_journal(
    journal: &InitialPlanPublicationJournal,
) -> Result<(), ProductStoreError> {
    for value in [
        &journal.id,
        &journal.project_id,
        &journal.issue_id,
        &journal.plan_id,
        &journal.compile_id,
        &journal.outline_version_ref,
    ] {
        validate_relative_id(value)?;
    }
    validate_initial_validation_report(&journal.artifacts.validation_report)?;
    if journal.artifacts.lineage.project_id != journal.project_id
        || journal.artifacts.lineage.issue_id != journal.issue_id
        || journal.artifacts.lineage.id != journal.plan_id
        || journal.artifacts.lineage.active_revision_id.is_some()
        || journal.artifacts.plan_revision.id != journal.allocated_ids.plan_revision_id
        || journal.artifacts.plan_revision.plan_id != journal.plan_id
        || journal.artifacts.plan_revision.dependency_graph_revision_id
            != journal.allocated_ids.dependency_graph_revision_id
        || journal.artifacts.plan_revision.validation_report_ref
            != journal.allocated_ids.validation_report_id
        || journal.artifacts.plan_revision.plan_projection_bundle_id
            != journal.allocated_ids.plan_projection_bundle_id
        || journal.artifacts.plan_revision.publication_provenance_ref
            != journal.artifacts.publication_provenance_ref
        || journal.artifacts.publication_provenance_ref.is_some()
            != journal
                .artifacts
                .publication_provenance_content_hash
                .is_some()
        || journal.artifacts.dependency_graph_revision.id
            != journal.allocated_ids.dependency_graph_revision_id
        || journal.artifacts.validation_report.id != journal.allocated_ids.validation_report_id
        || journal.artifacts.validation_report.plan_revision_id
            != journal.allocated_ids.plan_revision_id
        || journal
            .artifacts
            .validation_report
            .plan_projection_bundle_id
            != journal.allocated_ids.plan_projection_bundle_id
        || journal.artifacts.plan_projection_bundle.id
            != journal.allocated_ids.plan_projection_bundle_id
        || journal.artifact_fingerprint != publication_fingerprint(&journal.artifacts)?
    {
        return Err(identity_mismatch(
            "initial_plan_publication_journal",
            &journal.compile_id,
        ));
    }
    let draft_bindings = journal
        .artifacts
        .work_items
        .iter()
        .map(|item| {
            (
                item.logical_work_item.id.clone(),
                item.draft_revision.id.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let revision_bindings = journal
        .artifacts
        .work_items
        .iter()
        .map(|item| {
            (
                item.logical_work_item.id.clone(),
                item.work_item_revision.id.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if draft_bindings != journal.active_draft_revision_ids
        || revision_bindings != journal.artifacts.plan_revision.work_item_bindings
        || journal.artifacts.work_items.len() != journal.allocated_ids.work_items.len()
    {
        return Err(identity_mismatch(
            "initial_plan_publication_journal",
            &journal.compile_id,
        ));
    }
    for item in &journal.artifacts.work_items {
        let Some(ids) = journal
            .allocated_ids
            .work_items
            .get(&item.logical_work_item.id)
        else {
            return Err(identity_mismatch(
                "initial_plan_publication_journal",
                &journal.compile_id,
            ));
        };
        if item.logical_work_item.plan_id != journal.plan_id
            || item.logical_work_item.active_revision_id.is_some()
            || item.work_item_revision.id != ids.work_item_revision_id
            || item.verification_plan_revision.id != ids.verification_plan_revision_id
            || item.projection_bundle.id != ids.work_item_projection_bundle_id
            || item.work_item_revision.logical_work_item_id != item.logical_work_item.id
            || item.draft_revision.logical_work_item_id != item.logical_work_item.id
            || item.verification_plan_revision.logical_work_item_id != item.logical_work_item.id
            || item.work_item_revision.source_draft_revision_id != item.draft_revision.id
            || item.verification_plan_revision.source_draft_revision_id != item.draft_revision.id
            || item.work_item_revision.work_item_projection_bundle_id != item.projection_bundle.id
            || item.work_item_revision.verification_plan_revision_id
                != item.verification_plan_revision.id
            || item.projection_bundle.work_item_revision_id != item.work_item_revision.id
        {
            return Err(identity_mismatch(
                "initial_plan_publication_journal",
                &journal.compile_id,
            ));
        }
    }
    Ok(())
}

fn validate_initial_validation_report(
    report: &PlanValidationReportArtifact,
) -> Result<(), ProductStoreError> {
    if !report.contract_validation.is_valid() {
        return Err(ProductStoreError::Io(
            "initial_contract_validation_failed".to_string(),
        ));
    }
    if !report.projection_validation.is_valid() {
        return Err(ProductStoreError::Io(
            "initial_projection_validation_failed".to_string(),
        ));
    }
    Ok(())
}
