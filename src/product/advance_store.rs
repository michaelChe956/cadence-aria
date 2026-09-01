use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvanceStatus {
    Initializing,
    Ready,
    Running,
    AwaitingPlanAmendment,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvanceRecord {
    pub id: String,
    pub command_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub plan_revision_id: String,
    pub attempt_id: Option<String>,
    pub status: AdvanceStatus,
    pub workspace_entry: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvanceInput {
    pub command_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvanceInitializationPhase {
    RecordPersisted,
    JournalPrepared,
    AttemptPersisted,
    WorktreeBound,
    PlanBindingSaved,
    UnitsMaterialized,
    Ready,
}

impl AdvanceInitializationPhase {
    fn order(self) -> u8 {
        match self {
            Self::RecordPersisted => 0,
            Self::JournalPrepared => 1,
            Self::AttemptPersisted => 2,
            Self::WorktreeBound => 3,
            Self::PlanBindingSaved => 4,
            Self::UnitsMaterialized => 5,
            Self::Ready => 6,
        }
    }

    pub(crate) fn order_for_engine(self) -> u8 {
        self.order()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvanceInitializationJournal {
    pub advance_id: String,
    pub plan_id: String,
    pub attempt_id: String,
    pub phase: AdvanceInitializationPhase,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvanceOutcome {
    Completed {
        record: AdvanceRecord,
        attempt_id: String,
        workspace_entry: String,
    },
    Replayed {
        record: AdvanceRecord,
    },
    Rejected {
        record: Option<AdvanceRecord>,
        code: String,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct AdvanceStore {
    app_paths: ProductAppPaths,
}

impl AdvanceStore {
    pub fn new(app_paths: ProductAppPaths) -> Self {
        Self { app_paths }
    }

    pub fn app_paths(&self) -> ProductAppPaths {
        self.app_paths.clone()
    }

    fn root(&self, project_id: &str, issue_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(self
            .app_paths
            .issue_root(project_id, issue_id)
            .join("advance-records"))
    }

    fn path_for(
        &self,
        project_id: &str,
        issue_id: &str,
        id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(id)?;
        Ok(self.root(project_id, issue_id)?.join(format!("{id}.json")))
    }

    fn with_existing_root<T>(
        &self,
        project_id: &str,
        issue_id: &str,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let root = self.root(project_id, issue_id)?;
        if !root.is_dir() {
            return operation();
        }
        with_exclusive_lock(&root, operation)
    }

    fn records(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<AdvanceRecord>, ProductStoreError> {
        let root = self.root(project_id, issue_id)?;
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };
        let mut records = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|error| {
                    ProductStoreError::Io(format!("read advance record entry: {error}"))
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let record: AdvanceRecord = read_json(&path)?;
            if record.project_id != project_id || record.issue_id != issue_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "advance_record",
                    id: record.id,
                });
            }
            records.push(record);
        }
        records.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(records)
    }

    pub fn get_advance_by_command_id(
        &self,
        project_id: &str,
        issue_id: &str,
        command_id: &str,
    ) -> Result<Option<AdvanceRecord>, ProductStoreError> {
        validate_relative_id(command_id)?;
        self.with_existing_root(project_id, issue_id, || {
            Ok(self
                .records(project_id, issue_id)?
                .into_iter()
                .find(|record| record.command_id == command_id))
        })
    }

    pub fn get_advance_for_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Option<AdvanceRecord>, ProductStoreError> {
        validate_relative_id(plan_id)?;
        self.with_existing_root(project_id, issue_id, || {
            Ok(self
                .records(project_id, issue_id)?
                .into_iter()
                .find(|record| record.plan_id == plan_id))
        })
    }

    /// Test/next-stage orchestration hook. Task 5.1 deliberately never calls this
    /// on a first request; Task 5.2 owns the first durable record write.
    pub fn put_record(&self, record: &AdvanceRecord) -> Result<AdvanceRecord, ProductStoreError> {
        validate_advance_record_identity(record)?;
        validate_relative_id(&record.id)?;
        validate_relative_id(&record.command_id)?;
        validate_relative_id(&record.project_id)?;
        validate_relative_id(&record.issue_id)?;
        validate_relative_id(&record.plan_id)?;
        validate_relative_id(&record.plan_revision_id)?;
        let path = self.path_for(&record.project_id, &record.issue_id, &record.id)?;
        let root = self.root(&record.project_id, &record.issue_id)?;
        with_exclusive_lock(&root, || {
            if path.exists() {
                let existing: AdvanceRecord = read_json(&path)?;
                validate_advance_record_identity(&existing)?;
                if existing == *record {
                    return Ok(existing);
                }
                return Err(ProductStoreError::Conflict {
                    kind: "advance_record",
                    id: record.id.clone(),
                });
            }
            if self
                .records(&record.project_id, &record.issue_id)?
                .into_iter()
                .any(|existing| {
                    existing.command_id == record.command_id || existing.plan_id == record.plan_id
                })
            {
                return Err(ProductStoreError::Conflict {
                    kind: "advance_record_identity",
                    id: record.id.clone(),
                });
            }
            write_json(&path, record)?;
            Ok(record.clone())
        })
    }

    /// Persist the first durable anchor for a valid advance. Existing command/plan
    /// records are returned unchanged so callers can safely resume without allocating
    /// another timestamp or identifier.
    pub fn persist_advance_record_if_absent(
        &self,
        input: &AdvanceInput,
        plan_revision_id: &str,
    ) -> Result<AdvanceRecord, ProductStoreError> {
        for id in [
            input.command_id.as_str(),
            input.project_id.as_str(),
            input.issue_id.as_str(),
            input.plan_id.as_str(),
            plan_revision_id,
        ] {
            validate_relative_id(id)?;
        }
        if let Some(record) =
            self.get_advance_by_command_id(&input.project_id, &input.issue_id, &input.command_id)?
        {
            return Ok(record);
        }
        if let Some(record) =
            self.get_advance_for_plan(&input.project_id, &input.issue_id, &input.plan_id)?
        {
            return Ok(record);
        }
        let record = self.now_record(input, plan_revision_id.to_string());
        match self.put_record(&record) {
            Ok(record) => Ok(record),
            Err(ProductStoreError::Conflict { .. }) => self
                .get_advance_by_command_id(&input.project_id, &input.issue_id, &input.command_id)?
                .or_else(|| {
                    self.get_advance_for_plan(&input.project_id, &input.issue_id, &input.plan_id)
                        .ok()
                        .flatten()
                })
                .ok_or(ProductStoreError::Conflict {
                    kind: "advance_record_identity",
                    id: record.id,
                }),
            Err(error) => Err(error),
        }
    }

    /// Update only mutable state while enforcing the immutable advance identity.
    pub fn update_record(
        &self,
        expected: &AdvanceRecord,
    ) -> Result<AdvanceRecord, ProductStoreError> {
        let path = self.path_for(&expected.project_id, &expected.issue_id, &expected.id)?;
        let root = self.root(&expected.project_id, &expected.issue_id)?;
        with_exclusive_lock(&root, || {
            let current: AdvanceRecord = read_json(&path)?;
            if current.project_id != expected.project_id
                || current.issue_id != expected.issue_id
                || current.plan_id != expected.plan_id
                || current.command_id != expected.command_id
                || current.plan_revision_id != expected.plan_revision_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "advance_record",
                    id: expected.id.clone(),
                });
            }
            write_json(&path, expected)?;
            Ok(expected.clone())
        })
    }

    pub fn get_advance_initialization(
        &self,
        record: &AdvanceRecord,
    ) -> Result<Option<AdvanceInitializationJournal>, ProductStoreError> {
        validate_advance_record_identity(record)?;
        let path = self.initialization_path(record)?;
        if !path.is_file() {
            return Ok(None);
        }
        let journal: AdvanceInitializationJournal = read_json(&path)?;
        validate_advance_initialization_journal(&journal, record)?;
        Ok(Some(journal))
    }
    pub fn put_advance_initialization_if_absent(
        &self,
        record: &AdvanceRecord,
        attempt_id: &str,
    ) -> Result<AdvanceInitializationJournal, ProductStoreError> {
        validate_advance_record_identity(record)?;
        let path = self.initialization_path(record)?;
        if path.is_file() {
            let journal: AdvanceInitializationJournal = read_json(&path)?;
            validate_advance_initialization_journal(&journal, record)?;
            if journal.attempt_id != attempt_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "advance_initialization_journal_attempt",
                    id: record.id.clone(),
                });
            }
            return Ok(journal);
        }
        let now = Utc::now().to_rfc3339();
        let journal = AdvanceInitializationJournal {
            advance_id: record.id.clone(),
            plan_id: record.plan_id.clone(),
            attempt_id: attempt_id.to_string(),
            phase: AdvanceInitializationPhase::JournalPrepared,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        };
        write_json(&path, &journal)?;
        Ok(journal)
    }

    pub fn mark_advance_initialization_error(
        &self,
        record: &AdvanceRecord,
        journal: &AdvanceInitializationJournal,
        error: &str,
    ) -> Result<(AdvanceRecord, AdvanceInitializationJournal), ProductStoreError> {
        validate_advance_record_identity(record)?;
        validate_advance_initialization_journal(journal, record)?;
        let mut failed_record = record.clone();
        failed_record.status = AdvanceStatus::Failed;
        failed_record.error = Some(error.to_string());
        failed_record.updated_at = Utc::now().to_rfc3339();
        let mut failed_journal = journal.clone();
        failed_journal.error = Some(error.to_string());
        failed_journal.updated_at = Utc::now().to_rfc3339();
        self.update_record(&failed_record)?;
        self.save_advance_initialization(&failed_record, &failed_journal)?;
        Ok((failed_record, failed_journal))
    }

    pub fn save_advance_initialization(
        &self,
        record: &AdvanceRecord,
        journal: &AdvanceInitializationJournal,
    ) -> Result<AdvanceInitializationJournal, ProductStoreError> {
        validate_advance_record_identity(record)?;
        validate_advance_initialization_journal(journal, record)?;
        let path = self.initialization_path(record)?;
        write_json(&path, journal)?;
        Ok(journal.clone())
    }

    pub fn load_or_prepare_advance_initialization(
        &self,
        record: &AdvanceRecord,
        group: &crate::product::coding_attempt_store::CodingGroupInitializationJournal,
    ) -> Result<AdvanceInitializationJournal, ProductStoreError> {
        validate_advance_record_identity(record)?;
        let path = self.initialization_path(record)?;
        if path.is_file() {
            let journal: AdvanceInitializationJournal = read_json(&path)?;
            validate_advance_initialization_journal(&journal, record)?;
            if journal.attempt_id != group.attempt.id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "advance_initialization_journal_attempt",
                    id: record.id.clone(),
                });
            }
            return Ok(journal);
        }
        let now = Utc::now().to_rfc3339();
        let journal = AdvanceInitializationJournal {
            advance_id: record.id.clone(),
            plan_id: record.plan_id.clone(),
            attempt_id: group.attempt.id.clone(),
            phase: AdvanceInitializationPhase::JournalPrepared,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        };
        write_json(&path, &journal)?;
        Ok(journal)
    }

    pub fn advance_initialization_phase(
        &self,
        record: &AdvanceRecord,
        expected: &AdvanceInitializationJournal,
        next: AdvanceInitializationPhase,
    ) -> Result<AdvanceInitializationJournal, ProductStoreError> {
        validate_advance_record_identity(record)?;
        validate_advance_initialization_journal(expected, record)?;
        let path = self.initialization_path(record)?;
        let mut current: AdvanceInitializationJournal = read_json(&path)?;
        validate_advance_initialization_journal(&current, record)?;
        if current != *expected {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "advance_initialization_journal",
                id: record.id.clone(),
            });
        }
        if current.phase == next {
            return Ok(current);
        }
        if next.order() != current.phase.order() + 1 {
            return Err(ProductStoreError::Conflict {
                kind: "advance_initialization_phase",
                id: record.id.clone(),
            });
        }
        current.phase = next;
        current.error = None;
        current.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &current)?;
        Ok(current)
    }

    fn initialization_path(&self, record: &AdvanceRecord) -> Result<PathBuf, ProductStoreError> {
        Ok(self
            .root(&record.project_id, &record.issue_id)?
            .join(format!("{}.initialization.json", record.plan_id)))
    }

    pub fn now_record(&self, input: &AdvanceInput, plan_revision_id: String) -> AdvanceRecord {
        let now = Utc::now().to_rfc3339();
        AdvanceRecord {
            id: format!("advance_{}", input.command_id),
            command_id: input.command_id.clone(),
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            plan_id: input.plan_id.clone(),
            plan_revision_id,
            attempt_id: None,
            status: AdvanceStatus::Initializing,
            workspace_entry: None,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

pub use AdvanceRecord as Record;

fn validate_advance_record_identity(record: &AdvanceRecord) -> Result<(), ProductStoreError> {
    for id in [
        record.id.as_str(),
        record.command_id.as_str(),
        record.project_id.as_str(),
        record.issue_id.as_str(),
        record.plan_id.as_str(),
        record.plan_revision_id.as_str(),
    ] {
        validate_relative_id(id)?;
    }
    Ok(())
}

#[allow(dead_code)]
fn validate_advance_initialization_journal(
    journal: &AdvanceInitializationJournal,
    record: &AdvanceRecord,
) -> Result<(), ProductStoreError> {
    for id in [
        journal.advance_id.as_str(),
        journal.plan_id.as_str(),
        journal.attempt_id.as_str(),
    ] {
        validate_relative_id(id)?;
    }
    if journal.advance_id != record.id || journal.plan_id != record.plan_id {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "advance_initialization_journal",
            id: record.id.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn record() -> AdvanceRecord {
        AdvanceRecord {
            id: "advance_0001".into(),
            command_id: "command_0001".into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: "plan_0001".into(),
            plan_revision_id: "revision_0001".into(),
            attempt_id: Some("attempt_0001".into()),
            status: AdvanceStatus::Ready,
            workspace_entry: Some("/workspaces/attempt_0001".into()),
            error: None,
            created_at: "2026-08-31T00:00:00Z".into(),
            updated_at: "2026-08-31T00:00:01Z".into(),
        }
    }

    #[test]
    fn advance_record_store_supports_both_idempotency_indexes() {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let value = record();
        assert_eq!(store.put_record(&value).unwrap(), value);
        assert_eq!(
            store
                .get_advance_by_command_id("project_0001", "issue_0001", "command_0001")
                .unwrap(),
            Some(value.clone())
        );
        assert_eq!(
            store
                .get_advance_for_plan("project_0001", "issue_0001", "plan_0001")
                .unwrap(),
            Some(value)
        );
    }

    #[test]
    fn persist_advance_record_if_absent_is_idempotent_and_binds_revision() {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let input = AdvanceInput {
            command_id: "command_0001".into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: "plan_0001".into(),
        };
        let first = store
            .persist_advance_record_if_absent(&input, "revision_0001")
            .unwrap();
        let replay = store
            .persist_advance_record_if_absent(&input, "revision_changed")
            .unwrap();
        assert_eq!(first, replay);
        assert_eq!(replay.plan_revision_id, "revision_0001");
        assert_eq!(replay.status, AdvanceStatus::Initializing);
    }

    #[test]
    fn persist_advance_record_if_absent_replays_by_plan_before_new_command() {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let first_input = AdvanceInput {
            command_id: "command_0001".into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: "plan_0001".into(),
        };
        let second_input = AdvanceInput {
            command_id: "command_0002".into(),
            ..first_input.clone()
        };
        let first = store
            .persist_advance_record_if_absent(&first_input, "revision_0001")
            .unwrap();
        assert_eq!(
            store
                .persist_advance_record_if_absent(&second_input, "revision_0002")
                .unwrap(),
            first
        );
    }

    #[test]
    fn advance_initialization_journal_rejects_identity_drift() {
        let record = record();
        let journal = AdvanceInitializationJournal {
            advance_id: "advance_other".into(),
            plan_id: record.plan_id.clone(),
            attempt_id: "attempt_0001".into(),
            phase: AdvanceInitializationPhase::JournalPrepared,
            error: None,
            created_at: record.created_at.clone(),
            updated_at: record.updated_at.clone(),
        };
        assert!(matches!(
            validate_advance_initialization_journal(&journal, &record),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }
}
