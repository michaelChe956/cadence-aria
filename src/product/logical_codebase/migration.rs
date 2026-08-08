use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::coding_models::{
    AttemptTargetSnapshot, CodingAttemptPlanBinding, CodingAttemptScope, CodingExecutionAttempt,
    CodingExecutionUnit,
};
use crate::product::id::repo_hash_for_path;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IdentityRegistryEntry,
    IdentityRegistryState, IdentityRegistryStore, LogicalCodebaseLayout, LogicalCodebaseManifest,
    LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
    RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
use crate::product::models::{
    IssueRecord, IssueRuntimeBindingRecord, IssueSharedWorktree, LifecycleWorkItemRecord,
    RepositoryProfile, RepositoryRecord, StorySpecRecord,
};

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
    pub fn permits_legacy_projection(&self) -> bool {
        self.read_mode.as_deref() == Some("dual")
    }

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

    /// Runs the complete identity schema migration through the logical-authoritative
    /// read switch. The switch marker is persisted only after verification succeeds.
    pub fn ensure_identity_schema(&self, project_id: &str) -> Result<(), ProductStoreError> {
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
            if journal.phase == IdentityMigrationPhase::BackfillingCompatibility {
                self.backfill_compatibility(&mut journal)?;
            }
            match journal.phase {
                IdentityMigrationPhase::DualReadWrite => self.switch_reads(&mut journal)?,
                IdentityMigrationPhase::SwitchingReads
                | IdentityMigrationPhase::LegacyFallbackRemoved
                | IdentityMigrationPhase::Completed => {}
                IdentityMigrationPhase::Failed => return self.failed_migration_error(&journal),
                phase => {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "identity_migration_phase",
                        reason: format!(
                            "unsupported migration phase after authority migration: {phase:?}"
                        ),
                    });
                }
            }
            Ok(())
        })
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
            match journal.phase {
                IdentityMigrationPhase::WritingAuthority => {
                    self.write_authority_records(&mut journal)?
                }
                IdentityMigrationPhase::BackfillingCompatibility
                | IdentityMigrationPhase::DualReadWrite
                | IdentityMigrationPhase::SwitchingReads
                | IdentityMigrationPhase::LegacyFallbackRemoved
                | IdentityMigrationPhase::Completed => {}
                IdentityMigrationPhase::Failed => return self.failed_migration_error(&journal),
                phase => {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "identity_migration_phase",
                        reason: format!("unsupported migration phase through authority: {phase:?}"),
                    });
                }
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

    fn backfill_compatibility(
        &self,
        journal: &mut IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        let mappings = mappings_by_physical_id(journal)?;
        self.backfill_repository_projections(journal, &mappings)?;
        for issue in self.legacy_issues(&journal.project_id)? {
            self.backfill_issue_records(journal, &mappings, &issue)?;
        }
        self.backfill_attempt_snapshots(journal, &mappings)?;

        for index in 0..journal.mappings.len() {
            if journal.mappings[index].compatibility_backfilled {
                continue;
            }
            let physical_repository_id = journal.mappings[index].physical_repository_id.clone();
            journal.mappings[index].compatibility_backfilled = true;
            journal.completed_keys.push(format!(
                "backfill:{}:repository:{physical_repository_id}",
                journal.migration_id
            ));
            touch(journal);
            self.journals.save(&journal.project_id, journal)?;
        }
        journal.phase = IdentityMigrationPhase::DualReadWrite;
        journal.read_mode = Some("dual".to_string());
        journal.last_error = None;
        touch(journal);
        self.journals.save(&journal.project_id, journal)
    }

    fn backfill_repository_projections(
        &self,
        journal: &IdentityMigrationJournal,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let path = self
            .paths
            .project_root(&journal.project_id)
            .join("repos.json");
        if !path.exists() {
            return Ok(());
        }
        let mut repositories: Vec<RepositoryRecord> = read_json(&path)?;
        let mut changed = false;
        for repository in &mut repositories {
            validate_relative_id(&repository.id)?;
            let mapping = mapping_for_physical(mappings, &repository.id)?;
            let expected = (
                Some(mapping.logical_repository_id),
                Some(mapping.primary_checkout_id),
                1,
            );
            if repository.logical_repository_id.is_some()
                || repository.primary_checkout_id.is_some()
                || repository.identity_schema_version != 0
            {
                if (
                    repository.logical_repository_id,
                    repository.primary_checkout_id,
                    repository.identity_schema_version,
                ) != expected
                {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "repository_projection",
                        id: repository.id.clone(),
                    });
                }
            } else {
                repository.logical_repository_id = expected.0;
                repository.primary_checkout_id = expected.1;
                repository.identity_schema_version = expected.2;
                changed = true;
            }
        }
        if changed {
            write_json(&path, &repositories)?;
        }
        Ok(())
    }

    fn backfill_issue_records(
        &self,
        journal: &IdentityMigrationJournal,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
        issue: &IssueRecord,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&issue.id)?;
        if issue.project_id != journal.project_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "issue",
                id: issue.id.clone(),
            });
        }
        let issue_root = self.paths.issue_root(&journal.project_id, &issue.id);
        if let Some(physical_id) = issue.repo_id.as_deref() {
            validate_relative_id(physical_id)?;
            self.write_issue_selection(&issue_root, mapping_for_physical(mappings, physical_id)?)?;
        }
        self.backfill_bindings(&journal.project_id, &issue.id, mappings)?;
        self.backfill_stories(&journal.project_id, &issue.id, mappings)?;
        self.backfill_work_items(&journal.project_id, &issue.id, mappings)?;
        self.backfill_shared_worktree(&journal.project_id, &issue.id, mappings)?;
        self.backfill_repository_profiles(&journal.project_id, &issue.id, mappings)
    }

    fn write_issue_selection(
        &self,
        issue_root: &Path,
        mapping: &RepositoryIdentityMapping,
    ) -> Result<(), ProductStoreError> {
        let path = issue_root.join("codebase-selection.json");
        let expected = IssueCodebaseSelection {
            included: vec![mapping.logical_repository_id],
            focus: vec![mapping.logical_repository_id],
            selection_policy: "explicit".to_string(),
        };
        if path.exists() {
            let existing: IssueCodebaseSelection = read_json(&path)?;
            if existing != expected {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "issue_codebase_selection",
                    id: mapping.physical_repository_id.clone(),
                });
            }
            return Ok(());
        }
        write_json(&path, &expected)
    }

    fn backfill_bindings(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let root = self.paths.issue_root(project_id, issue_id).join("bindings");
        rewrite_json_records::<IssueRuntimeBindingRecord, _>(&root, |binding| {
            validate_relative_id(&binding.id)?;
            let mapping = mapping_for_physical(mappings, &binding.repo_id)?;
            assign_optional_identity(
                &mut binding.logical_repository_id,
                mapping.logical_repository_id,
                "runtime_binding",
                &binding.id,
            )?;
            assign_optional_identity(
                &mut binding.checkout_id,
                mapping.primary_checkout_id,
                "runtime_binding",
                &binding.id,
            )
        })
    }

    fn backfill_stories(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("story-specs");
        let manifest = self.required_manifest(project_id)?;
        rewrite_json_records::<StorySpecRecord, _>(&root, |story| {
            validate_relative_id(&story.id)?;
            let mapping = mapping_for_physical(mappings, &story.repository_id)?;
            assign_optional_identity(
                &mut story.logical_codebase_ref,
                manifest.logical_codebase_id,
                "story_spec",
                &story.id,
            )?;
            assign_vec_identity(
                &mut story.involved_repository_ids,
                vec![mapping.logical_repository_id],
                "story_spec",
                &story.id,
            )?;
            assign_optional_identity(
                &mut story.focus_repository_id,
                mapping.logical_repository_id,
                "story_spec",
                &story.id,
            )
        })
    }

    fn backfill_work_items(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("work-items");
        rewrite_json_records::<LifecycleWorkItemRecord, _>(&root, |work_item| {
            validate_relative_id(&work_item.id)?;
            let mapping = mapping_for_physical(mappings, &work_item.repository_id)?;
            assign_optional_identity(
                &mut work_item.target_repository_id,
                mapping.logical_repository_id,
                "work_item",
                &work_item.id,
            )
        })
    }

    fn backfill_shared_worktree(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let path = self
            .paths
            .issue_root(project_id, issue_id)
            .join("issue-shared-worktree.json");
        if !path.exists() {
            return Ok(());
        }
        let mut worktree: IssueSharedWorktree = read_json(&path)?;
        validate_relative_id(&worktree.id)?;
        let mapping = mapping_for_physical(mappings, &worktree.repository_id)?;
        assign_optional_identity(
            &mut worktree.target_repository_id,
            mapping.logical_repository_id,
            "issue_shared_worktree",
            &worktree.id,
        )?;
        assign_optional_identity(
            &mut worktree.checkout_id,
            mapping.primary_checkout_id,
            "issue_shared_worktree",
            &worktree.id,
        )?;
        if worktree.path_schema_version == 0 {
            worktree.path_schema_version = 1;
        } else if worktree.path_schema_version != 1 {
            return Err(ProductStoreError::InvalidRecord {
                kind: "issue_shared_worktree",
                reason: format!("unsupported path_schema_version for {}", worktree.id),
            });
        }
        write_json(&path, &worktree)
    }

    fn backfill_repository_profiles(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("repository-profiles");
        let manifest = self.required_manifest(project_id)?;
        rewrite_json_records::<RepositoryProfile, _>(&root, |profile| {
            validate_relative_id(&profile.id)?;
            let mapping = mapping_for_physical(mappings, &profile.repository_id)?;
            assign_optional_identity(
                &mut profile.logical_repository_id,
                mapping.logical_repository_id,
                "repository_profile",
                &profile.id,
            )?;
            if profile.membership_revision == 0 {
                profile.membership_revision = manifest.membership_revision;
                Ok(())
            } else if profile.membership_revision == manifest.membership_revision {
                Ok(())
            } else {
                Err(ProductStoreError::IdentityMismatch {
                    kind: "repository_profile",
                    id: profile.id.clone(),
                })
            }
        })
    }

    fn backfill_attempt_snapshots(
        &self,
        journal: &IdentityMigrationJournal,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let manifest = self.required_manifest(&journal.project_id)?;
        for issue in self.legacy_issues(&journal.project_id)? {
            let root = self
                .paths
                .issue_root(&journal.project_id, &issue.id)
                .join("coding-attempts");
            if !root.exists() {
                continue;
            }
            for entry in std::fs::read_dir(&root).map_err(|error| {
                ProductStoreError::Io(format!("read {}: {error}", root.display()))
            })? {
                let path = entry
                    .map_err(|error| {
                        ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
                    })?
                    .path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                    continue;
                }
                let mut attempt: CodingExecutionAttempt = read_json(&path)?;
                self.backfill_attempt_snapshot(journal, mappings, &manifest, &mut attempt)?;
                write_json(&path, &attempt)?;
            }
        }
        Ok(())
    }

    fn backfill_attempt_snapshot(
        &self,
        journal: &IdentityMigrationJournal,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
        manifest: &LogicalCodebaseManifest,
        attempt: &mut CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&attempt.id)?;
        validate_relative_id(&attempt.issue_id)?;
        if attempt.project_id != journal.project_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_attempt",
                id: attempt.id.clone(),
            });
        }
        if attempt.target_snapshot.is_some() {
            return Ok(());
        }
        if attempt.status.is_active() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "target_snapshot_missing",
                reason: format!("active legacy attempt {} cannot resume", attempt.id),
            });
        }
        let work_item = self.resolve_attempt_work_item(attempt)?;
        let mapping = mapping_for_physical(mappings, &work_item.repository_id)?;
        let checkout = self
            .authority
            .load_checkout(&journal.project_id, mapping.primary_checkout_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "repository_checkout",
                id: mapping.primary_checkout_id.0.to_string(),
            })?;
        if checkout.logical_repository_id != mapping.logical_repository_id
            || checkout.physical_repository_id != mapping.physical_repository_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "repository_checkout",
                id: checkout.checkout_id.0.to_string(),
            });
        }
        attempt.target_snapshot = Some(AttemptTargetSnapshot {
            logical_repository_id: mapping.logical_repository_id,
            checkout_id: mapping.primary_checkout_id,
            physical_repository_id: mapping.physical_repository_id.clone(),
            canonical_path: checkout.canonical_path,
            git_dir_identity: checkout.git_dir_identity,
            revision: None,
            policy_digest: manifest.context_policy_digest.clone(),
            membership_revision: manifest.membership_revision,
            captured_at: Utc::now().to_rfc3339(),
            capture_source: "migration_observed".to_string(),
        });
        Ok(())
    }

    fn resolve_attempt_work_item(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        let current_work_item_id = match attempt.scope {
            CodingAttemptScope::WorkItem => attempt
                .current_work_item_id
                .as_deref()
                .unwrap_or(&attempt.work_item_id),
            CodingAttemptScope::WorkItemGroup => attempt
                .current_work_item_id
                .as_deref()
                .ok_or_else(|| ProductStoreError::InvalidRecord {
                    kind: "target_snapshot_missing",
                    reason: format!("group attempt {} has no current_work_item_id", attempt.id),
                })?,
        };
        validate_relative_id(current_work_item_id)?;
        let current =
            self.load_work_item(&attempt.project_id, &attempt.issue_id, current_work_item_id)?;
        if attempt.scope == CodingAttemptScope::WorkItemGroup {
            self.validate_group_attempt_target(attempt, &current)?;
        }
        Ok(current)
    }

    fn validate_group_attempt_target(
        &self,
        attempt: &CodingExecutionAttempt,
        current: &LifecycleWorkItemRecord,
    ) -> Result<(), ProductStoreError> {
        let group_id = attempt.work_item_group_id.as_deref().ok_or_else(|| {
            ProductStoreError::InvalidRecord {
                kind: "target_snapshot_missing",
                reason: format!("group attempt {} has no group id", attempt.id),
            }
        })?;
        validate_relative_id(group_id)?;
        let root = self
            .paths
            .issue_root(&attempt.project_id, &attempt.issue_id)
            .join("coding-attempts")
            .join(&attempt.id);
        let mut work_item_ids = BTreeSet::from([current.id.clone()]);
        for unit in read_json_records::<CodingExecutionUnit>(&root.join("units"))? {
            validate_relative_id(&unit.id)?;
            validate_relative_id(&unit.logical_work_item_id)?;
            if unit.attempt_id != attempt.id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_execution_unit",
                    id: unit.id,
                });
            }
            work_item_ids.insert(unit.logical_work_item_id);
        }
        let binding_path = root.join("plan-binding.json");
        if binding_path.exists() {
            let binding: CodingAttemptPlanBinding = read_json(&binding_path)?;
            if binding.attempt_id != attempt.id || binding.plan_id != group_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_attempt_plan_binding",
                    id: attempt.id.clone(),
                });
            }
        }
        let initialization_path = self
            .paths
            .issue_root(&attempt.project_id, &attempt.issue_id)
            .join("coding-attempts")
            .join("group-initializations")
            .join(format!("{group_id}.json"));
        if initialization_path.exists() {
            let initialization: serde_json::Value = read_json(&initialization_path)?;
            let initialized_attempt_id = initialization
                .get("attempt")
                .and_then(|value| value.get("id"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| ProductStoreError::InvalidRecord {
                    kind: "target_snapshot_missing",
                    reason: format!("group initialization for {} is unresolved", attempt.id),
                })?;
            let initialized_current = initialization
                .get("attempt")
                .and_then(|value| value.get("current_work_item_id"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| ProductStoreError::InvalidRecord {
                    kind: "target_snapshot_missing",
                    reason: format!("group initialization for {} is unresolved", attempt.id),
                })?;
            if initialized_attempt_id != attempt.id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_group_initialization",
                    id: attempt.id.clone(),
                });
            }
            validate_relative_id(initialized_current)?;
            work_item_ids.insert(initialized_current.to_string());
        }
        for work_item_id in work_item_ids {
            let candidate =
                self.load_work_item(&attempt.project_id, &attempt.issue_id, &work_item_id)?;
            if candidate.repository_id != current.repository_id {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "target_snapshot_missing",
                    reason: format!("group attempt {} has mixed work item targets", attempt.id),
                });
            }
        }
        Ok(())
    }

    fn switch_reads(
        &self,
        journal: &mut IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        IdentityMigrationVerifier::new(self.paths.clone()).verify(&journal.project_id)?;
        let manifest = self.required_manifest(&journal.project_id)?;
        journal.phase = IdentityMigrationPhase::SwitchingReads;
        journal.read_mode = Some("logical_authoritative".to_string());
        journal.completed_keys.push(format!(
            "switch:{}:{}:{}",
            journal.migration_id, journal.source_repos_digest, manifest.membership_revision
        ));
        journal.last_error = None;
        touch(journal);
        // The marker is the last migration write: a pre-marker crash remains dual.
        self.journals.save(&journal.project_id, journal)
    }

    fn required_manifest(
        &self,
        project_id: &str,
    ) -> Result<LogicalCodebaseManifest, ProductStoreError> {
        self.authority
            .load_manifest(project_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "logical_codebase_manifest",
                id: project_id.to_string(),
            })
    }

    fn legacy_issues(&self, project_id: &str) -> Result<Vec<IssueRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        let root = self.paths.project_root(project_id).join("issues");
        let mut issues = Vec::new();
        for path in child_json_paths(&root, Some("issue.json"))? {
            let issue: IssueRecord = read_json(&path)?;
            validate_relative_id(&issue.id)?;
            issues.push(issue);
        }
        issues.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(issues)
    }

    fn load_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        let path = self
            .paths
            .issue_root(project_id, issue_id)
            .join("work-items")
            .join(format!("{work_item_id}.json"));
        if !path.exists() {
            return Err(ProductStoreError::NotFound {
                kind: "work_item",
                id: work_item_id.to_string(),
            });
        }
        let work_item: LifecycleWorkItemRecord = read_json(&path)?;
        if work_item.project_id != project_id
            || work_item.issue_id != issue_id
            || work_item.id != work_item_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "work_item",
                id: work_item_id.to_string(),
            });
        }
        Ok(work_item)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct IssueCodebaseSelection {
    included: Vec<LogicalRepositoryId>,
    focus: Vec<LogicalRepositoryId>,
    selection_policy: String,
}

pub struct IdentityMigrationVerifier {
    paths: ProductAppPaths,
    authority: LogicalCodebaseStore,
}

impl IdentityMigrationVerifier {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self {
            authority: LogicalCodebaseStore::new(paths.clone()),
            paths,
        }
    }

    pub fn verify(&self, project_id: &str) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        let journal = IdentityMigrationJournalStore::new(self.paths.clone())
            .load(project_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "identity_migration_journal",
                id: project_id.to_string(),
            })?;
        if journal.read_mode.as_deref() != Some("dual") {
            return Err(ProductStoreError::InvalidRecord {
                kind: "identity_migration_verifier",
                reason: "read_mode must be dual before switching".to_string(),
            });
        }
        if !journal
            .mappings
            .iter()
            .all(|mapping| mapping.authority_written && mapping.compatibility_backfilled)
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "identity_migration_verifier",
                reason: "migration compatibility backfill is incomplete".to_string(),
            });
        }
        let manifest = self.authority.load_manifest(project_id)?.ok_or_else(|| {
            ProductStoreError::NotFound {
                kind: "logical_codebase_manifest",
                id: project_id.to_string(),
            }
        })?;
        let mappings = mappings_by_physical_id(&journal)?;
        let mut expected_members = BTreeSet::new();
        for mapping in mappings.values() {
            expected_members.insert(mapping.logical_repository_id);
            let member = self
                .authority
                .load_member(project_id, mapping.logical_repository_id)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "logical_codebase_member",
                    id: mapping.logical_repository_id.0.to_string(),
                })?;
            let checkout = self
                .authority
                .load_checkout(project_id, mapping.primary_checkout_id)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "repository_checkout",
                    id: mapping.primary_checkout_id.0.to_string(),
                })?;
            if member.physical_repository_id != mapping.physical_repository_id
                || !member.checkout_ids.contains(&mapping.primary_checkout_id)
                || checkout.logical_repository_id != mapping.logical_repository_id
                || checkout.physical_repository_id != mapping.physical_repository_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "logical_authority",
                    id: mapping.physical_repository_id.clone(),
                });
            }
        }
        if manifest.member_ids.iter().copied().collect::<BTreeSet<_>>() != expected_members {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_manifest",
                id: project_id.to_string(),
            });
        }
        self.verify_repository_projections(project_id, &mappings)?;
        self.verify_issue_projections(project_id, &manifest, &mappings)?;
        self.verify_attempts(project_id, &manifest, &mappings)
    }

    fn verify_repository_projections(
        &self,
        project_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let path = self.paths.project_root(project_id).join("repos.json");
        if !path.exists() {
            return Ok(());
        }
        let records: Vec<RepositoryRecord> = read_json(&path)?;
        for record in records {
            validate_relative_id(&record.id)?;
            let mapping = mapping_for_physical(mappings, &record.id)?;
            if record.logical_repository_id != Some(mapping.logical_repository_id)
                || record.primary_checkout_id != Some(mapping.primary_checkout_id)
                || record.identity_schema_version != 1
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "repository_projection",
                    id: record.id,
                });
            }
        }
        Ok(())
    }

    fn verify_issue_projections(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        for issue_path in child_json_paths(
            &self.paths.project_root(project_id).join("issues"),
            Some("issue.json"),
        )? {
            let issue: IssueRecord = read_json(&issue_path)?;
            validate_relative_id(&issue.id)?;
            if issue.project_id != project_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "issue",
                    id: issue.id,
                });
            }
            let issue_root = self.paths.issue_root(project_id, &issue.id);
            if let Some(physical_id) = issue.repo_id.as_deref() {
                let mapping = mapping_for_physical(mappings, physical_id)?;
                let selection_path = issue_root.join("codebase-selection.json");
                let selection: IssueCodebaseSelection = read_json(&selection_path)?;
                let expected = IssueCodebaseSelection {
                    included: vec![mapping.logical_repository_id],
                    focus: vec![mapping.logical_repository_id],
                    selection_policy: "explicit".to_string(),
                };
                if selection != expected {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "issue_codebase_selection",
                        id: issue.id,
                    });
                }
            }
            self.verify_bindings(project_id, &issue.id, mappings)?;
            self.verify_stories(project_id, &issue.id, manifest, mappings)?;
            self.verify_work_items(project_id, &issue.id, mappings)?;
            self.verify_shared_worktree(project_id, &issue.id, mappings)?;
            self.verify_repository_profiles(project_id, &issue.id, manifest, mappings)?;
        }
        Ok(())
    }

    fn verify_bindings(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let root = self.paths.issue_root(project_id, issue_id).join("bindings");
        for binding in read_json_records::<IssueRuntimeBindingRecord>(&root)? {
            validate_relative_id(&binding.id)?;
            let mapping = mapping_for_physical(mappings, &binding.repo_id)?;
            if binding.logical_repository_id != Some(mapping.logical_repository_id)
                || binding.checkout_id != Some(mapping.primary_checkout_id)
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "runtime_binding",
                    id: binding.id,
                });
            }
        }
        Ok(())
    }

    fn verify_stories(
        &self,
        project_id: &str,
        issue_id: &str,
        manifest: &LogicalCodebaseManifest,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("story-specs");
        for story in read_json_records::<StorySpecRecord>(&root)? {
            validate_relative_id(&story.id)?;
            let mapping = mapping_for_physical(mappings, &story.repository_id)?;
            if story.logical_codebase_ref != Some(manifest.logical_codebase_id)
                || story.involved_repository_ids != vec![mapping.logical_repository_id]
                || story.focus_repository_id != Some(mapping.logical_repository_id)
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "story_spec",
                    id: story.id,
                });
            }
        }
        Ok(())
    }

    fn verify_work_items(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("work-items");
        for work_item in read_json_records::<LifecycleWorkItemRecord>(&root)? {
            validate_relative_id(&work_item.id)?;
            let mapping = mapping_for_physical(mappings, &work_item.repository_id)?;
            if work_item.target_repository_id != Some(mapping.logical_repository_id) {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "work_item",
                    id: work_item.id,
                });
            }
        }
        Ok(())
    }

    fn verify_shared_worktree(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let path = self
            .paths
            .issue_root(project_id, issue_id)
            .join("issue-shared-worktree.json");
        if !path.exists() {
            return Ok(());
        }
        let worktree: IssueSharedWorktree = read_json(&path)?;
        validate_relative_id(&worktree.id)?;
        let mapping = mapping_for_physical(mappings, &worktree.repository_id)?;
        if worktree.target_repository_id != Some(mapping.logical_repository_id)
            || worktree.checkout_id != Some(mapping.primary_checkout_id)
            || worktree.path_schema_version != 1
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "issue_shared_worktree",
                id: worktree.id,
            });
        }
        Ok(())
    }

    fn verify_repository_profiles(
        &self,
        project_id: &str,
        issue_id: &str,
        manifest: &LogicalCodebaseManifest,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("repository-profiles");
        for profile in read_json_records::<RepositoryProfile>(&root)? {
            validate_relative_id(&profile.id)?;
            let mapping = mapping_for_physical(mappings, &profile.repository_id)?;
            if profile.logical_repository_id != Some(mapping.logical_repository_id)
                || profile.membership_revision != manifest.membership_revision
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "repository_profile",
                    id: profile.id,
                });
            }
        }
        Ok(())
    }

    fn verify_attempts(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        for issue_path in child_json_paths(
            &self.paths.project_root(project_id).join("issues"),
            Some("issue.json"),
        )? {
            let issue: IssueRecord = read_json(&issue_path)?;
            let root = self
                .paths
                .issue_root(project_id, &issue.id)
                .join("coding-attempts");
            for attempt in read_json_records_shallow::<CodingExecutionAttempt>(&root)? {
                validate_relative_id(&attempt.id)?;
                if attempt.project_id != project_id || attempt.issue_id != issue.id {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "coding_attempt",
                        id: attempt.id,
                    });
                }
                let Some(snapshot) = attempt.target_snapshot.as_ref() else {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "target_snapshot_missing",
                        reason: format!(
                            "legacy attempt {} blocks logical-authoritative reads",
                            attempt.id
                        ),
                    });
                };
                let mapping = mapping_for_physical(mappings, &snapshot.physical_repository_id)?;
                let checkout = self
                    .authority
                    .load_checkout(project_id, snapshot.checkout_id)?
                    .ok_or_else(|| ProductStoreError::NotFound {
                        kind: "repository_checkout",
                        id: snapshot.checkout_id.0.to_string(),
                    })?;
                if snapshot.logical_repository_id != mapping.logical_repository_id
                    || snapshot.checkout_id != mapping.primary_checkout_id
                    || checkout.logical_repository_id != snapshot.logical_repository_id
                    || checkout.physical_repository_id != snapshot.physical_repository_id
                    || snapshot.membership_revision != manifest.membership_revision
                {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "attempt_target_snapshot",
                        id: attempt.id,
                    });
                }
                if attempt.status.is_active() && snapshot.capture_source == "migration_observed" {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "target_snapshot_missing",
                        reason: format!(
                            "active migration-observed attempt {} blocks switch",
                            attempt.id
                        ),
                    });
                }
            }
        }
        Ok(())
    }
}

fn mappings_by_physical_id(
    journal: &IdentityMigrationJournal,
) -> Result<BTreeMap<String, RepositoryIdentityMapping>, ProductStoreError> {
    let mut mappings = BTreeMap::new();
    for mapping in &journal.mappings {
        validate_relative_id(&mapping.physical_repository_id)?;
        if mappings
            .insert(mapping.physical_repository_id.clone(), mapping.clone())
            .is_some()
        {
            return Err(ProductStoreError::Ambiguous {
                kind: "identity_migration_mapping",
                id: mapping.physical_repository_id.clone(),
            });
        }
    }
    Ok(mappings)
}

fn mapping_for_physical<'a>(
    mappings: &'a BTreeMap<String, RepositoryIdentityMapping>,
    physical_repository_id: &str,
) -> Result<&'a RepositoryIdentityMapping, ProductStoreError> {
    validate_relative_id(physical_repository_id)?;
    mappings
        .get(physical_repository_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "identity_migration_mapping",
            id: physical_repository_id.to_string(),
        })
}

fn assign_optional_identity<T: Copy + Eq>(
    slot: &mut Option<T>,
    expected: T,
    kind: &'static str,
    id: &str,
) -> Result<(), ProductStoreError> {
    match slot {
        Some(actual) if *actual != expected => Err(ProductStoreError::IdentityMismatch {
            kind,
            id: id.to_string(),
        }),
        Some(_) => Ok(()),
        None => {
            *slot = Some(expected);
            Ok(())
        }
    }
}

fn assign_vec_identity<T: Eq>(
    slot: &mut Vec<T>,
    expected: Vec<T>,
    kind: &'static str,
    id: &str,
) -> Result<(), ProductStoreError> {
    if slot.is_empty() {
        *slot = expected;
        return Ok(());
    }
    if *slot == expected {
        return Ok(());
    }
    Err(ProductStoreError::IdentityMismatch {
        kind,
        id: id.to_string(),
    })
}

fn child_json_paths(
    root: &Path,
    exact_file_name: Option<&str>,
) -> Result<Vec<PathBuf>, ProductStoreError> {
    fn collect(
        root: &Path,
        exact_file_name: Option<&str>,
        paths: &mut Vec<PathBuf>,
    ) -> Result<(), ProductStoreError> {
        if !root.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(root)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?
        {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            let path = entry.path();
            if let Some(file_name) = exact_file_name {
                if path.is_dir() {
                    collect(&path, Some(file_name), paths)?;
                } else if path.file_name().and_then(|name| name.to_str()) == Some(file_name) {
                    paths.push(path);
                }
            } else if path.is_dir() {
                collect(&path, None, paths)?;
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
                paths.push(path);
            }
        }
        Ok(())
    }

    let mut paths = Vec::new();
    collect(root, exact_file_name, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn read_json_records<T: serde::de::DeserializeOwned>(
    root: &Path,
) -> Result<Vec<T>, ProductStoreError> {
    child_json_paths(root, None)?
        .into_iter()
        .map(|path| read_json(&path))
        .collect()
}

fn read_json_records_shallow<T: serde::de::DeserializeOwned>(
    root: &Path,
) -> Result<Vec<T>, ProductStoreError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(root)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?
    {
        let path = entry
            .map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?
            .path();
        if path.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("json")
        {
            paths.push(path);
        }
    }
    paths.sort();
    paths.into_iter().map(|path| read_json(&path)).collect()
}

fn rewrite_json_records<T, F>(root: &Path, mut mutate: F) -> Result<(), ProductStoreError>
where
    T: serde::de::DeserializeOwned + Serialize,
    F: FnMut(&mut T) -> Result<(), ProductStoreError>,
{
    for path in child_json_paths(root, None)? {
        let mut record: T = read_json(&path)?;
        mutate(&mut record)?;
        write_json(&path, &record)?;
    }
    Ok(())
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
    fn active_legacy_attempt_blocks_switch_but_terminal_attempt_gets_observed_snapshot() {
        let fixture = migration_fixture_with_one_git_repository();
        fixture.write_active_legacy_attempt_without_target_snapshot();
        let executor = IdentityMigrationExecutor::new(fixture.paths.clone());

        let error = executor.ensure_identity_schema("project_0001").unwrap_err();
        assert!(
            error.to_string().contains("target_snapshot_missing"),
            "unexpected migration error: {error}"
        );

        fixture.mark_attempt_completed();
        executor.ensure_identity_schema("project_0001").unwrap();
        let attempt = fixture.read_attempt();
        assert_eq!(
            attempt.target_snapshot.unwrap().capture_source,
            "migration_observed"
        );
        assert_eq!(
            fixture.journal().read_mode.as_deref(),
            Some("logical_authoritative")
        );
    }

    #[test]
    fn verifier_ignores_non_attempt_json_beneath_coding_attempt_directory() {
        let fixture = migration_fixture_with_one_git_repository();
        fixture.write_active_legacy_attempt_without_target_snapshot();
        fixture.mark_attempt_completed();
        IdentityMigrationExecutor::new(fixture.paths.clone())
            .ensure_identity_schema("project_0001")
            .expect("migrate terminal legacy attempt");
        fixture.write_non_attempt_json_beneath_coding_attempt_directory();
        fixture.set_journal_read_mode("dual");

        IdentityMigrationVerifier::new(fixture.paths.clone())
            .verify("project_0001")
            .expect("attempt verifier must ignore child records");
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
        write_json(
            &paths
                .issue_root("project_0001", "issue_0001")
                .join("issue.json"),
            &serde_json::json!({
                "id": "issue_0001",
                "project_id": "project_0001",
                "repo_id": "repository_0001",
                "author": null,
                "title": "legacy issue",
                "description": null,
                "change_id": "legacy",
                "phase": "clarification",
                "status": "draft",
                "active_binding_id": null,
                "created_at": "2026-08-08T00:00:00Z",
                "updated_at": "2026-08-08T00:00:00Z"
            }),
        )
        .expect("write legacy issue");
        MigrationFixture { _root: root, paths }
    }

    impl MigrationFixture {
        fn attempt_path(&self) -> PathBuf {
            self.paths
                .issue_root("project_0001", "issue_0001")
                .join("coding-attempts")
                .join("coding_attempt_0001.json")
        }

        fn write_active_legacy_attempt_without_target_snapshot(&self) {
            write_json(
                &self
                    .paths
                    .issue_root("project_0001", "issue_0001")
                    .join("work-items")
                    .join("work_item_0001.json"),
                &serde_json::json!({
                    "id": "work_item_0001",
                    "project_id": "project_0001",
                    "issue_id": "issue_0001",
                    "repository_id": "repository_0001",
                    "story_spec_ids": [],
                    "design_spec_ids": [],
                    "title": "legacy work item",
                    "plan_status": "not_started",
                    "execution_status": "pending",
                    "worktree_path": null,
                    "created_at": "2026-08-08T00:00:00Z",
                    "updated_at": "2026-08-08T00:00:00Z"
                }),
            )
            .expect("write legacy work item");
            write_json(
                &self.attempt_path(),
                &serde_json::json!({
                    "id": "coding_attempt_0001",
                    "project_id": "project_0001",
                    "issue_id": "issue_0001",
                    "work_item_id": "work_item_0001",
                    "attempt_no": 1,
                    "status": "running",
                    "stage": "coding",
                    "base_branch": "main",
                    "branch_name": "aria/legacy",
                    "worktree_path": null,
                    "provider_config_snapshot": {
                        "author": "fake",
                        "reviewer": null,
                        "review_rounds": 0,
                        "permission_modes": {"author": "auto", "reviewer": "auto"}
                    },
                    "rework_count": 0,
                    "max_auto_rework": 0,
                    "head_commit": null,
                    "pushed_remote": null,
                    "review_request_id": null,
                    "created_at": "2026-08-08T00:00:00Z",
                    "updated_at": "2026-08-08T00:00:00Z",
                    "completed_at": null
                }),
            )
            .expect("write legacy attempt");
        }

        fn mark_attempt_completed(&self) {
            let mut value: serde_json::Value =
                read_json(&self.attempt_path()).expect("read attempt JSON");
            value["status"] = serde_json::Value::String("completed".to_string());
            value["completed_at"] = serde_json::Value::String("2026-08-08T00:01:00Z".to_string());
            write_json(&self.attempt_path(), &value).expect("write completed attempt");
        }

        fn read_attempt(&self) -> crate::product::coding_models::CodingExecutionAttempt {
            read_json(&self.attempt_path()).expect("read migrated attempt")
        }

        fn write_non_attempt_json_beneath_coding_attempt_directory(&self) {
            write_json(
                &self
                    .paths
                    .issue_root("project_0001", "issue_0001")
                    .join("coding-attempts")
                    .join("coding_attempt_0001")
                    .join("units")
                    .join("coding_unit_0001.json"),
                &serde_json::json!({"kind": "coding_execution_unit"}),
            )
            .expect("write non-attempt child record");
        }

        fn set_journal_read_mode(&self, read_mode: &str) {
            let mut journal = self.journal();
            journal.read_mode = Some(read_mode.to_string());
            IdentityMigrationJournalStore::new(self.paths.clone())
                .save("project_0001", &journal)
                .expect("save journal read mode");
        }

        fn journal(&self) -> IdentityMigrationJournal {
            IdentityMigrationJournalStore::new(self.paths.clone())
                .load("project_0001")
                .expect("load journal")
                .expect("journal")
        }
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
