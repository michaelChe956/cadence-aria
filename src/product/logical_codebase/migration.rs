use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::id::repo_hash_for_path;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IdentityRegistryEntry,
    IdentityRegistryState, IdentityRegistryStore, LogicalCodebaseLayout, LogicalCodebaseManifest,
    LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
    RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
use crate::product::models::RepositoryRecord;

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

/// Test-only and embedding hook for simulating a process interruption after an
/// authority write. Production executors use its no-op implementation.
pub trait MigrationFaultInjector: Send + Sync {
    fn after_authority_write(
        &self,
        _project_id: &str,
        _mapping: &RepositoryIdentityMapping,
    ) -> Result<(), ProductStoreError> {
        Ok(())
    }
}

#[derive(Debug)]
struct NoopMigrationFaultInjector;

impl MigrationFaultInjector for NoopMigrationFaultInjector {}

pub struct IdentityMigrationExecutor {
    paths: ProductAppPaths,
    journals: IdentityMigrationJournalStore,
    authority: LogicalCodebaseStore,
    registry: IdentityRegistryStore,
    fault_injector: Arc<dyn MigrationFaultInjector>,
}

impl IdentityMigrationExecutor {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self::with_fault_injector(paths, Arc::new(NoopMigrationFaultInjector))
    }

    pub fn with_fault_injector(
        paths: ProductAppPaths,
        fault_injector: Arc<dyn MigrationFaultInjector>,
    ) -> Self {
        Self {
            journals: IdentityMigrationJournalStore::new(paths.clone()),
            authority: LogicalCodebaseStore::new(paths.clone()),
            registry: IdentityRegistryStore::new(paths.clone()),
            paths,
            fault_injector,
        }
    }

    /// Runs discovery, source mapping, and authority writes. Later migration
    /// stages intentionally start from `BackfillingCompatibility`.
    pub fn ensure_through_authority(&self, project_id: &str) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        let lock_path = self.paths.identity_migration_lock_path(project_id);
        with_exact_exclusive_lock(&lock_path, || {
            let mut journal = self.load_or_begin_scanning(project_id)?;
            match journal.phase {
                IdentityMigrationPhase::Scanning => self.scan_legacy_repositories(&mut journal)?,
                IdentityMigrationPhase::Failed => return self.failed_migration_error(&journal),
                _ => {}
            }
            if journal.phase == IdentityMigrationPhase::Mapping {
                self.persist_mappings_from_source_identity(&mut journal)?;
            }
            if journal.phase == IdentityMigrationPhase::WritingAuthority {
                self.write_authority_records(&mut journal)?;
            }
            Ok(())
        })
    }

    fn load_or_begin_scanning(
        &self,
        project_id: &str,
    ) -> Result<IdentityMigrationJournal, ProductStoreError> {
        if let Some(journal) = self.journals.load(project_id)? {
            return Ok(journal);
        }

        let journal = IdentityMigrationJournal::new(project_id, "");
        self.journals.save(project_id, &journal)?;
        Ok(journal)
    }

    fn scan_legacy_repositories(
        &self,
        journal: &mut IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        let repositories = self.legacy_repositories(&journal.project_id)?;
        if let Some(duplicate_id) = duplicate_repository_id(&repositories) {
            return self.fail_identity_mismatch(journal, "legacy_repository", duplicate_id);
        }

        journal.source_repos_digest = source_repositories_digest(&repositories)?;
        journal.phase = IdentityMigrationPhase::Mapping;
        journal.last_error = None;
        touch(journal);
        self.journals.save(&journal.project_id, journal)
    }

    fn persist_mappings_from_source_identity(
        &self,
        journal: &mut IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        let repositories = self.load_scanned_repositories(journal)?;
        if let Some(duplicate_id) = duplicate_mapping_legacy_id(&journal.mappings) {
            return self.fail_identity_mismatch(
                journal,
                "identity_migration_mapping",
                duplicate_id,
            );
        }

        for repository in &repositories {
            let source_identity = repository_source_identity(repository)?;
            if let Some(mapping) = journal
                .mappings
                .iter()
                .find(|mapping| mapping.legacy_repository_id == repository.id)
            {
                if mapping.source_identity_digest != source_identity.key_digest
                    || mapping.physical_repository_id != repository.id
                    || mapping.idempotency_key
                        != mapping_idempotency_key(
                            &journal.project_id,
                            &repository.id,
                            &source_identity.key_digest,
                        )
                {
                    return self.fail_identity_mismatch(
                        journal,
                        "identity_migration_mapping",
                        repository.id.clone(),
                    );
                }
                continue;
            }

            let idempotency_key = mapping_idempotency_key(
                &journal.project_id,
                &repository.id,
                &source_identity.key_digest,
            );
            let (logical_repository_id, primary_checkout_id) = match self
                .registry
                .find_by_source(&journal.project_id, &source_identity)?
            {
                Some(entry) if entry.state == IdentityRegistryState::Active => {
                    if entry.physical_repository_id != repository.id {
                        return self.fail_identity_mismatch(
                            journal,
                            "identity_registry",
                            repository.id.clone(),
                        );
                    }
                    (entry.logical_repository_id, entry.primary_checkout_id)
                }
                Some(_) => {
                    return self.fail_identity_mismatch(
                        journal,
                        "identity_registry",
                        repository.id.clone(),
                    );
                }
                None => (
                    LogicalRepositoryId(Uuid::new_v4()),
                    RepositoryCheckoutId(Uuid::new_v4()),
                ),
            };

            journal.mappings.push(RepositoryIdentityMapping {
                legacy_repository_id: repository.id.clone(),
                source_identity_digest: source_identity.key_digest,
                logical_repository_id,
                primary_checkout_id,
                physical_repository_id: repository.id.clone(),
                idempotency_key,
                authority_written: false,
                compatibility_backfilled: false,
            });
            touch(journal);
            // The generated UUIDs become durable before the next allocation.
            self.journals.save(&journal.project_id, journal)?;
        }

        if journal.mappings.len() != repositories.len() {
            return self.fail_identity_mismatch(
                journal,
                "identity_migration_mapping",
                journal.project_id.clone(),
            );
        }
        journal.phase = IdentityMigrationPhase::WritingAuthority;
        journal.last_error = None;
        touch(journal);
        self.journals.save(&journal.project_id, journal)
    }

    fn write_authority_records(
        &self,
        journal: &mut IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        let repositories = self.load_scanned_repositories(journal)?;
        let inputs = self.authority_inputs(journal, &repositories)?;
        let member_ids = inputs
            .iter()
            .map(|input| input.mapping.logical_repository_id)
            .collect::<Vec<_>>();
        if duplicate_logical_repository_id(&member_ids).is_some() {
            return self.fail_identity_mismatch(
                journal,
                "identity_migration_mapping",
                journal.project_id.clone(),
            );
        }

        self.ensure_manifest(journal, &inputs, member_ids)?;
        for input in inputs {
            self.ensure_authority_for_mapping(journal, input)?;
        }

        journal.phase = IdentityMigrationPhase::BackfillingCompatibility;
        journal.last_error = None;
        touch(journal);
        self.journals.save(&journal.project_id, journal)
    }

    fn authority_inputs(
        &self,
        journal: &mut IdentityMigrationJournal,
        repositories: &[RepositoryRecord],
    ) -> Result<Vec<AuthorityInput>, ProductStoreError> {
        let mut inputs = Vec::with_capacity(repositories.len());
        for (index, repository) in repositories.iter().enumerate() {
            let mapping = match journal
                .mappings
                .iter()
                .find(|mapping| mapping.legacy_repository_id == repository.id)
            {
                Some(mapping) => mapping.clone(),
                None => {
                    return self.fail_identity_mismatch(
                        journal,
                        "identity_migration_mapping",
                        repository.id.clone(),
                    );
                }
            };
            let source_identity = repository_source_identity(repository)?;
            if mapping.source_identity_digest != source_identity.key_digest
                || mapping.physical_repository_id != repository.id
                || mapping.idempotency_key
                    != mapping_idempotency_key(
                        &journal.project_id,
                        &repository.id,
                        &source_identity.key_digest,
                    )
            {
                return self.fail_identity_mismatch(
                    journal,
                    "identity_migration_mapping",
                    repository.id.clone(),
                );
            }
            let ordinal =
                u32::try_from(index + 1).map_err(|_| ProductStoreError::InvalidRecord {
                    kind: "identity_migration_mapping",
                    reason: "repository ordinal exceeds u32".to_string(),
                })?;
            let canonical_path = canonicalize_repository_path(&repository.path)?;
            inputs.push(AuthorityInput {
                repository: repository.clone(),
                mapping,
                source_identity,
                canonical_path,
                ordinal,
            });
        }
        Ok(inputs)
    }

    fn ensure_manifest(
        &self,
        journal: &mut IdentityMigrationJournal,
        inputs: &[AuthorityInput],
        member_ids: Vec<LogicalRepositoryId>,
    ) -> Result<(), ProductStoreError> {
        let provider_context_root = common_non_git_parent(inputs)
            .unwrap_or_else(|| self.paths.project_root(&journal.project_id));
        match self.authority.load_manifest(&journal.project_id)? {
            Some(manifest)
                if manifest.schema_version == 1
                    && manifest.project_id == journal.project_id
                    && manifest.layout == LogicalCodebaseLayout::CommonNonGitParent
                    && manifest.provider_context_root == provider_context_root
                    && manifest.member_ids == member_ids =>
            {
                Ok(())
            }
            Some(_) => self.fail_identity_mismatch(
                journal,
                "logical_codebase_manifest",
                journal.project_id.clone(),
            ),
            None => {
                let manifest = LogicalCodebaseManifest::new(
                    &journal.project_id,
                    provider_context_root,
                    member_ids,
                );
                self.authority.save_manifest(&journal.project_id, &manifest)
            }
        }
    }

    fn ensure_authority_for_mapping(
        &self,
        journal: &mut IdentityMigrationJournal,
        input: AuthorityInput,
    ) -> Result<(), ProductStoreError> {
        let member = expected_member(&input);
        match self
            .authority
            .load_member(&journal.project_id, input.mapping.logical_repository_id)?
        {
            Some(existing) if existing == member => {}
            Some(_) => {
                return self.fail_identity_mismatch(
                    journal,
                    "logical_codebase_member",
                    input.mapping.logical_repository_id.0.to_string(),
                );
            }
            None => self.authority.save_member(&journal.project_id, &member)?,
        }

        let checkout = expected_checkout(&input);
        match self
            .authority
            .load_checkout(&journal.project_id, input.mapping.primary_checkout_id)?
        {
            Some(existing) if existing == checkout => {}
            Some(_) => {
                return self.fail_identity_mismatch(
                    journal,
                    "repository_checkout",
                    input.mapping.primary_checkout_id.0.to_string(),
                );
            }
            None => self
                .authority
                .save_checkout(&journal.project_id, &checkout)?,
        }

        let expected_registry = IdentityRegistryEntry::active(
            input.source_identity.clone(),
            input.mapping.logical_repository_id,
            input.mapping.physical_repository_id.clone(),
            input.mapping.primary_checkout_id,
            input.mapping.idempotency_key.clone(),
        );
        match self
            .registry
            .find_by_source(&journal.project_id, &input.source_identity)?
        {
            Some(existing) if existing == expected_registry => {}
            Some(_) => {
                return self.fail_identity_mismatch(
                    journal,
                    "identity_registry",
                    input.mapping.physical_repository_id.clone(),
                );
            }
            None => self
                .registry
                .upsert_active(&journal.project_id, expected_registry)?,
        }

        let mapping_index = journal
            .mappings
            .iter()
            .position(|mapping| mapping.legacy_repository_id == input.mapping.legacy_repository_id)
            .expect("authority input must have a journal mapping");
        if !journal.mappings[mapping_index].authority_written {
            journal.mappings[mapping_index].authority_written = true;
            touch(journal);
            // The marker is persisted after all authority files and before a
            // failpoint can simulate an abrupt process exit.
            self.journals.save(&journal.project_id, journal)?;
        }
        let persisted_mapping = journal.mappings[mapping_index].clone();
        self.fault_injector
            .after_authority_write(&journal.project_id, &persisted_mapping)
    }

    fn load_scanned_repositories(
        &self,
        journal: &mut IdentityMigrationJournal,
    ) -> Result<Vec<RepositoryRecord>, ProductStoreError> {
        let repositories = self.legacy_repositories(&journal.project_id)?;
        let digest = source_repositories_digest(&repositories)?;
        if journal.source_repos_digest != digest {
            return self.fail_identity_mismatch(
                journal,
                "identity_migration_source_repositories",
                journal.project_id.clone(),
            );
        }
        Ok(repositories)
    }

    fn legacy_repositories(
        &self,
        project_id: &str,
    ) -> Result<Vec<RepositoryRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        let path = self.paths.project_root(project_id).join("repos.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let mut repositories: Vec<RepositoryRecord> = read_json(&path)?;
        repositories.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(repositories)
    }

    fn fail_identity_mismatch<T>(
        &self,
        journal: &mut IdentityMigrationJournal,
        kind: &'static str,
        id: String,
    ) -> Result<T, ProductStoreError> {
        journal.phase = IdentityMigrationPhase::Failed;
        journal.last_error = Some(format!("identity mismatch: {kind} {id}"));
        touch(journal);
        self.journals.save(&journal.project_id, journal)?;
        Err(ProductStoreError::IdentityMismatch { kind, id })
    }

    fn failed_migration_error<T>(
        &self,
        journal: &IdentityMigrationJournal,
    ) -> Result<T, ProductStoreError> {
        Err(ProductStoreError::Conflict {
            kind: "identity_migration_failed",
            id: journal.project_id.clone(),
        })
    }
}

#[derive(Debug, Clone)]
struct AuthorityInput {
    repository: RepositoryRecord,
    mapping: RepositoryIdentityMapping,
    source_identity: RepositorySourceIdentity,
    canonical_path: PathBuf,
    ordinal: u32,
}

fn source_repositories_digest(
    repositories: &[RepositoryRecord],
) -> Result<String, ProductStoreError> {
    let canonical_json = serde_json::to_vec(repositories).map_err(|error| {
        ProductStoreError::Json(format!("serialize legacy repositories: {error}"))
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(canonical_json)))
}

fn duplicate_repository_id(repositories: &[RepositoryRecord]) -> Option<String> {
    repositories
        .windows(2)
        .find(|pair| pair[0].id == pair[1].id)
        .map(|pair| pair[0].id.clone())
}

fn duplicate_mapping_legacy_id(mappings: &[RepositoryIdentityMapping]) -> Option<String> {
    let mut ids = HashSet::new();
    mappings.iter().find_map(|mapping| {
        (!ids.insert(&mapping.legacy_repository_id)).then(|| mapping.legacy_repository_id.clone())
    })
}

fn duplicate_logical_repository_id(ids: &[LogicalRepositoryId]) -> Option<LogicalRepositoryId> {
    let mut seen = HashSet::new();
    ids.iter().find_map(|id| (!seen.insert(*id)).then_some(*id))
}

fn mapping_idempotency_key(
    project_id: &str,
    legacy_repository_id: &str,
    source_digest: &str,
) -> String {
    format!("map:{project_id}:{legacy_repository_id}:{source_digest}")
}

fn repository_source_identity(
    repository: &RepositoryRecord,
) -> Result<RepositorySourceIdentity, ProductStoreError> {
    let canonical_path = canonicalize_repository_path(&repository.path)?;
    let git_dir_output = run_git(&canonical_path, &["rev-parse", "--git-dir"])?;
    let git_dir = PathBuf::from(git_dir_output.trim());
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        canonical_path.join(git_dir)
    };
    let canonical_git_dir = canonicalize_repository_path(&git_dir)?;
    let canonical_origin = match run_git(&canonical_path, &["remote", "get-url", "origin"]) {
        Ok(value) => {
            let origin = value.trim();
            (!origin.is_empty()).then(|| origin.to_string())
        }
        Err(ProductStoreError::Io(message)) if message.contains("git exited") => None,
        Err(error) => return Err(error),
    };
    Ok(RepositorySourceIdentity::from_git_parts(
        &canonical_path,
        canonical_git_dir,
        canonical_origin,
    ))
}

fn canonicalize_repository_path(path: &Path) -> Result<PathBuf, ProductStoreError> {
    std::fs::canonicalize(path)
        .map_err(|error| ProductStoreError::Io(format!("canonicalize {}: {error}", path.display())))
}

fn run_git(repository_path: &Path, arguments: &[&str]) -> Result<String, ProductStoreError> {
    let output = Command::new("git")
        .current_dir(repository_path)
        .args(arguments)
        .output()
        .map_err(|error| {
            ProductStoreError::Io(format!("run git in {}: {error}", repository_path.display()))
        })?;
    if !output.status.success() {
        return Err(ProductStoreError::Io(format!(
            "git exited {:?} in {}: {}",
            output.status.code(),
            repository_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| ProductStoreError::Io(format!("git output was not UTF-8: {error}")))
}

fn common_non_git_parent(inputs: &[AuthorityInput]) -> Option<PathBuf> {
    let first = inputs.first()?.canonical_path.parent()?.to_path_buf();
    let mut common = first;
    for input in &inputs[1..] {
        let parent = input.canonical_path.parent()?;
        while !parent.starts_with(&common) {
            common = common.parent()?.to_path_buf();
        }
    }
    Some(common)
}

fn expected_member(input: &AuthorityInput) -> CodebaseMemberRecord {
    CodebaseMemberRecord {
        logical_repository_id: input.mapping.logical_repository_id,
        physical_repository_id: input.mapping.physical_repository_id.clone(),
        alias: input.repository.name.clone(),
        role: "repository".to_string(),
        ordinal: input.ordinal,
        source_identity: input.source_identity.clone(),
        repo_type: RepositoryType::Unknown,
        tech_stack: Vec::new(),
        owner: None,
        tags: Vec::new(),
        default_ref: None,
        checkout_ids: vec![input.mapping.primary_checkout_id],
        status: MemberStatus::Active,
        created_at: input.repository.created_at.clone(),
        updated_at: input.repository.updated_at.clone(),
    }
}

fn expected_checkout(input: &AuthorityInput) -> RepositoryCheckoutRecord {
    RepositoryCheckoutRecord {
        checkout_id: input.mapping.primary_checkout_id,
        logical_repository_id: input.mapping.logical_repository_id,
        physical_repository_id: input.mapping.physical_repository_id.clone(),
        kind: CheckoutKind::Main,
        canonical_path: input.canonical_path.clone(),
        checkout_path_hash: repo_hash_for_path(input.canonical_path.to_string_lossy().as_ref()),
        git_dir_identity: input.source_identity.git_dir_identity(),
        revision: None,
        availability: CheckoutAvailability::Available,
        observed_at: input.repository.updated_at.clone(),
        created_at: input.repository.created_at.clone(),
        updated_at: input.repository.updated_at.clone(),
    }
}

fn touch(journal: &mut IdentityMigrationJournal) {
    journal.updated_at = Utc::now().to_rfc3339();
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::json_store::write_json;
    use crate::product::logical_codebase::LogicalCodebaseStore;
    use crate::product::models::RepositoryRecord;

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

    #[test]
    fn authority_write_crash_replays_the_same_mapping_without_duplicate_members() {
        let fixture = migration_fixture_with_one_git_repository();
        let failing = IdentityMigrationExecutor::with_fault_injector(
            fixture.paths.clone(),
            Arc::new(FailAfterAuthorityWrite::new()),
        );
        assert!(failing.ensure_through_authority("project_0001").is_err());

        let first = IdentityMigrationJournalStore::new(fixture.paths.clone())
            .load("project_0001")
            .unwrap()
            .unwrap();
        let first_mapping = first.mappings[0].clone();
        assert!(first_mapping.authority_written);

        IdentityMigrationExecutor::new(fixture.paths.clone())
            .ensure_through_authority("project_0001")
            .unwrap();
        let second = IdentityMigrationJournalStore::new(fixture.paths.clone())
            .load("project_0001")
            .unwrap()
            .unwrap();
        let members = LogicalCodebaseStore::new(fixture.paths.clone())
            .list_members("project_0001")
            .unwrap();

        assert_eq!(
            second.mappings[0].logical_repository_id,
            first_mapping.logical_repository_id
        );
        assert_eq!(
            second.mappings[0].primary_checkout_id,
            first_mapping.primary_checkout_id
        );
        assert_eq!(members.len(), 1);
    }

    struct MigrationFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
    }

    fn migration_fixture_with_one_git_repository() -> MigrationFixture {
        let root = tempfile::tempdir().expect("temporary product root");
        let repository_path = root.path().join("repository");
        run_git_command(&["init", "--quiet", repository_path.to_str().unwrap()]);
        run_git_command_in(
            &repository_path,
            &[
                "remote",
                "add",
                "origin",
                "ssh://git@example.test/acme/api.git",
            ],
        );

        let paths = ProductAppPaths::new(root.path());
        let record = RepositoryRecord {
            id: "repository_0001".to_string(),
            project_id: "project_0001".to_string(),
            name: "api".to_string(),
            path: repository_path,
            repo_hash: "legacy-hash".to_string(),
            runtime_root: PathBuf::from("/unused/.aria/runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: "2026-08-08T00:00:00Z".to_string(),
            updated_at: "2026-08-08T00:00:00Z".to_string(),
            logical_repository_id: None,
            primary_checkout_id: None,
            identity_schema_version: 0,
        };
        write_json(
            &paths.project_root("project_0001").join("repos.json"),
            &vec![record],
        )
        .expect("write legacy repositories");
        MigrationFixture { _root: root, paths }
    }

    fn run_git_command(arguments: &[&str]) {
        let status = Command::new("git")
            .args(arguments)
            .status()
            .expect("start git");
        assert!(status.success(), "git {arguments:?}");
    }

    fn run_git_command_in(repository: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .current_dir(repository)
            .args(arguments)
            .status()
            .expect("start git");
        assert!(
            status.success(),
            "git -C {} {arguments:?}",
            repository.display()
        );
    }

    struct FailAfterAuthorityWrite {
        has_failed: AtomicBool,
    }

    impl FailAfterAuthorityWrite {
        fn new() -> Self {
            Self {
                has_failed: AtomicBool::new(false),
            }
        }
    }

    impl MigrationFaultInjector for FailAfterAuthorityWrite {
        fn after_authority_write(
            &self,
            _project_id: &str,
            _mapping: &RepositoryIdentityMapping,
        ) -> Result<(), ProductStoreError> {
            if !self.has_failed.swap(true, Ordering::SeqCst) {
                return Err(ProductStoreError::Io(
                    "injected crash after authority write".to_string(),
                ));
            }
            Ok(())
        }
    }
}
