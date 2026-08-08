use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};

const IDENTITY_MIGRATION_JOURNAL_FILE: &str = "identity-migration.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityMigrationPhase {
    Scanning,
    Mapping,
    WritingAuthority,
    BackfillingCompatibility,
    DualReadWrite,
    SwitchingReads,
    LegacyFallbackRemoved,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryIdentityMapping {
    pub legacy_repository_id: String,
    pub source_identity_digest: String,
    pub logical_repository_id: LogicalRepositoryId,
    pub primary_checkout_id: RepositoryCheckoutId,
    pub physical_repository_id: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub authority_written: bool,
    #[serde(default)]
    pub compatibility_backfilled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityMigrationJournal {
    pub journal_version: u16,
    pub migration_id: String,
    pub project_id: String,
    pub target_schema_version: u16,
    pub phase: IdentityMigrationPhase,
    pub source_repos_digest: String,
    pub mappings: Vec<RepositoryIdentityMapping>,
    #[serde(default)]
    pub completed_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

impl IdentityMigrationJournal {
    pub fn new(project_id: &str, source_repos_digest: &str) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            journal_version: 1,
            migration_id: format!("identity-migration:{project_id}:v1"),
            project_id: project_id.to_string(),
            target_schema_version: 1,
            phase: IdentityMigrationPhase::Scanning,
            source_repos_digest: source_repos_digest.to_string(),
            mappings: Vec::new(),
            completed_keys: Vec::new(),
            read_mode: None,
            last_error: None,
            created_at: now.clone(),
            updated_at: now,
            completed_at: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IdentityMigrationJournalStore {
    paths: ProductAppPaths,
}

impl IdentityMigrationJournalStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn load(
        &self,
        project_id: &str,
    ) -> Result<Option<IdentityMigrationJournal>, ProductStoreError> {
        let path = self.path(project_id)?;
        if !path.exists() {
            return Ok(None);
        }

        let journal: IdentityMigrationJournal = read_json(&path)?;
        self.validate_project(project_id, &journal)?;
        Ok(Some(journal))
    }

    pub fn save(
        &self,
        project_id: &str,
        journal: &IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        self.validate_project(project_id, journal)?;
        write_json(&self.path(project_id)?, journal)
    }

    fn path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        Ok(self
            .paths
            .logical_codebase_root(project_id)
            .join(IDENTITY_MIGRATION_JOURNAL_FILE))
    }

    fn validate_project(
        &self,
        project_id: &str,
        journal: &IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(&journal.project_id)?;
        if journal.project_id != project_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "identity_migration_journal",
                id: project_id.to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;

    #[test]
    fn journal_preserves_uuid_mapping_and_phase_for_crash_replay() {
        let journal = IdentityMigrationJournal::new("project_0001", "sha256:legacy-repos");
        assert_eq!(journal.phase, IdentityMigrationPhase::Scanning);
        assert_eq!(journal.migration_id, "identity-migration:project_0001:v1");
        assert_eq!(journal.target_schema_version, 1);
        assert_eq!(journal.read_mode, None);
        assert_eq!(
            serde_json::to_value(&journal).unwrap()["journal_version"],
            1
        );
    }

    #[test]
    fn journal_store_round_trips_the_project_scoped_journal() {
        let directory = tempfile::tempdir().expect("temporary product root");
        let store = IdentityMigrationJournalStore::new(ProductAppPaths::new(directory.path()));
        let journal = IdentityMigrationJournal::new("project_0001", "sha256:legacy-repos");

        assert_eq!(store.load("project_0001").expect("load missing"), None);
        store
            .save("project_0001", &journal)
            .expect("save project journal");

        assert_eq!(
            store.load("project_0001").expect("load journal"),
            Some(journal)
        );
    }
}
