use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::id::{next_sequential_id, repo_hash_for_path};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IdentityMigrationExecutor,
    IdentityRegistryEntry, IdentityRegistryState, IdentityRegistryStore, LogicalCodebaseFeature,
    LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
    RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
use crate::product::models::RepositoryRecord;

mod initializer;
mod operation;
mod registration;
mod types;

pub use initializer::ClaudeRepositoryInitializer;
pub use operation::RepositoryInitializationOperationStore;
#[allow(unused_imports)]
pub(crate) use registration::RepositoryInitializationLaunch;
pub use registration::{
    CadenceSkillsPreparation, ProjectLookup, RepositoryInitializer, RepositoryPersistence,
    RepositoryRegistrationCoordinator,
};
pub use types::{
    CadenceSkillsPreparationSummary, RepositoryInitializationCommandSummary,
    RepositoryInitializationOperation, RepositoryInitializationOperationInput,
    RepositoryInitializationOperationStatus, RepositoryInitializationProgress,
    RepositoryInitializationStepKind, RepositoryInitializationStepRecord,
    RepositoryInitializationStepStatus, RepositoryInitializationSummary,
    RepositoryRegistrationError, RepositoryRegistrationInput, RepositoryRegistrationSuccess,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRepositoryInput {
    pub project_id: String,
    pub name: String,
    pub path: PathBuf,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
    /// 调用方持久化的 command/operation ID；用于安全重放创建命令。
    pub idempotency_key: String,
}

#[derive(Debug, Clone)]
pub struct RepositoryStore {
    paths: ProductAppPaths,
    logical_codebase_feature: LogicalCodebaseFeature,
}

impl RepositoryStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self::with_logical_codebase_feature(paths, LogicalCodebaseFeature::disabled())
    }

    pub fn with_logical_codebase_feature(
        paths: ProductAppPaths,
        feature: LogicalCodebaseFeature,
    ) -> Self {
        Self {
            paths,
            logical_codebase_feature: feature,
        }
    }

    pub fn ensure_identity_schema(&self, project_id: &str) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        if self.logical_codebase_feature.is_enabled() {
            IdentityMigrationExecutor::new(self.paths.clone())
                .ensure_identity_schema(project_id)?;
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn initialization_operation_store(&self) -> RepositoryInitializationOperationStore {
        RepositoryInitializationOperationStore::new(self.paths.clone())
    }

    pub fn list(&self, project_id: &str) -> Result<Vec<RepositoryRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        self.ensure_identity_schema(project_id)?;
        let path = self.repos_path(project_id);
        if !path.exists() {
            return Ok(Vec::new());
        }

        read_json(&path)
    }

    pub fn create(
        &self,
        input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        if !self.logical_codebase_feature.is_enabled() {
            return self.create_legacy(input);
        }

        self.ensure_identity_schema(&input.project_id)?;
        let project_id = input.project_id.clone();
        let lock_path = self.paths.identity_migration_lock_path(&project_id);
        with_exact_exclusive_lock(&lock_path, || self.create_logical_repository(input))
    }

    fn create_legacy(
        &self,
        input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        let project_id = input.project_id;
        let mut repositories = self.list(&project_id)?;
        let existing_len = repositories.len();
        let id = next_sequential_id("repository", existing_len);
        let now = Utc::now().to_rfc3339();
        let canonical_path = canonicalize_repo_path(&input.path)?;
        let repo_path_text = canonical_path.to_string_lossy();
        let repository = RepositoryRecord {
            id: id.clone(),
            project_id: project_id.clone(),
            name: input.name,
            repo_hash: repo_hash_for_path(repo_path_text.as_ref()),
            runtime_root: canonical_path.join(".aria/runtime"),
            path: canonical_path,
            default_policy_preset: input
                .default_policy_preset
                .unwrap_or_else(|| "manual-write".to_string()),
            default_provider_mode: input
                .default_provider_mode
                .unwrap_or_else(|| "fake".to_string()),
            created_at: now.clone(),
            updated_at: now,
            logical_repository_id: None,
            primary_checkout_id: None,
            identity_schema_version: 0,
        };

        repositories.push(repository.clone());
        write_json(&self.repos_path(&project_id), &repositories)?;
        Ok(repository)
    }

    fn create_logical_repository(
        &self,
        input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.idempotency_key)?;
        let canonical_path = canonicalize_repo_path(&input.path)?;
        let source_identity = resolve_repository_source(&canonical_path)?;
        let receipts = RepositoryCommandReceiptStore::new(self.paths.clone());
        let receipt = receipts.find_create(&input.project_id, &input.idempotency_key)?;
        if let Some(receipt) = receipt.as_ref() {
            receipt.validate_input(&input, &canonical_path)?;
        }

        let registry = IdentityRegistryStore::new(self.paths.clone());
        let authority = LogicalCodebaseStore::new(self.paths.clone());
        let identity = match registry.find_by_source(&input.project_id, &source_identity)? {
            Some(entry) if entry.state == IdentityRegistryState::Active => {
                if entry.created_by_key != input.idempotency_key {
                    return Err(ProductStoreError::Conflict {
                        kind: "repository_already_registered",
                        id: entry.physical_repository_id,
                    });
                }
                if let Some(receipt) = receipt {
                    return Ok(receipt.repository);
                }
                self.existing_authority_identity(
                    &authority,
                    &input,
                    &canonical_path,
                    &source_identity,
                    &entry,
                )?
            }
            Some(entry) => {
                return Err(ProductStoreError::Conflict {
                    kind: "repository_source_tombstoned",
                    id: entry.physical_repository_id,
                });
            }
            None => {
                if let Some(receipt) = receipt {
                    return Ok(receipt.repository);
                }
                match self.find_incomplete_authority_identity(
                    &authority,
                    &input,
                    &canonical_path,
                    &source_identity,
                )? {
                    Some(identity) => identity,
                    None => RepositoryIdentityAllocation::new(),
                }
            }
        };

        let repository = identity.repository_record(&input, canonical_path.clone());
        self.ensure_authority_records(
            &authority,
            &registry,
            &input,
            &source_identity,
            &identity,
            &repository,
        )?;
        self.ensure_compatibility_projection(&repository)?;

        let receipt = RepositoryCreateReceipt::new(&input, &canonical_path, repository.clone());
        receipts.save_create(&input.project_id, &receipt)?;
        Ok(repository)
    }

    fn existing_authority_identity(
        &self,
        authority: &LogicalCodebaseStore,
        input: &CreateRepositoryInput,
        canonical_path: &Path,
        source_identity: &RepositorySourceIdentity,
        entry: &IdentityRegistryEntry,
    ) -> Result<RepositoryIdentityAllocation, ProductStoreError> {
        let created_at = authority
            .load_member(&input.project_id, entry.logical_repository_id)?
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: entry.logical_repository_id.0.to_string(),
            })?
            .created_at;
        let identity = RepositoryIdentityAllocation {
            physical_repository_id: entry.physical_repository_id.clone(),
            logical_repository_id: entry.logical_repository_id,
            primary_checkout_id: entry.primary_checkout_id,
            created_at,
        };
        self.validate_authority_identity(
            authority,
            input,
            canonical_path,
            source_identity,
            &identity,
        )?;
        Ok(identity)
    }

    fn find_incomplete_authority_identity(
        &self,
        authority: &LogicalCodebaseStore,
        input: &CreateRepositoryInput,
        canonical_path: &Path,
        source_identity: &RepositorySourceIdentity,
    ) -> Result<Option<RepositoryIdentityAllocation>, ProductStoreError> {
        let matching_members = authority
            .list_members(&input.project_id)?
            .into_iter()
            .filter(|member| member.source_identity == *source_identity)
            .collect::<Vec<_>>();
        if matching_members.len() > 1 {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: source_identity.key_digest.clone(),
            });
        }
        let Some(member) = matching_members.into_iter().next() else {
            return Ok(None);
        };
        let [primary_checkout_id] = member.checkout_ids.as_slice() else {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: member.logical_repository_id.0.to_string(),
            });
        };
        let identity = RepositoryIdentityAllocation {
            physical_repository_id: member.physical_repository_id.clone(),
            logical_repository_id: member.logical_repository_id,
            primary_checkout_id: *primary_checkout_id,
            created_at: member.created_at.clone(),
        };
        validate_relative_id(&identity.physical_repository_id)?;
        let expected_member =
            identity.member_record(input, source_identity, member.ordinal, &member.created_at);
        if member != expected_member {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: identity.logical_repository_id.0.to_string(),
            });
        }
        if let Some(checkout) =
            authority.load_checkout(&input.project_id, identity.primary_checkout_id)?
        {
            let expected_checkout =
                identity.checkout_record(canonical_path, source_identity, &checkout.created_at);
            if checkout != expected_checkout {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "repository_checkout",
                    id: identity.primary_checkout_id.0.to_string(),
                });
            }
        }
        Ok(Some(identity))
    }

    fn validate_authority_identity(
        &self,
        authority: &LogicalCodebaseStore,
        input: &CreateRepositoryInput,
        canonical_path: &Path,
        source_identity: &RepositorySourceIdentity,
        identity: &RepositoryIdentityAllocation,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&identity.physical_repository_id)?;
        let member = authority
            .load_member(&input.project_id, identity.logical_repository_id)?
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: identity.logical_repository_id.0.to_string(),
            })?;
        let expected_member =
            identity.member_record(input, source_identity, member.ordinal, &member.created_at);
        if member != expected_member {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: identity.logical_repository_id.0.to_string(),
            });
        }
        let checkout = authority
            .load_checkout(&input.project_id, identity.primary_checkout_id)?
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "repository_checkout",
                id: identity.primary_checkout_id.0.to_string(),
            })?;
        let expected_checkout =
            identity.checkout_record(canonical_path, source_identity, &checkout.created_at);
        if checkout != expected_checkout {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "repository_checkout",
                id: identity.primary_checkout_id.0.to_string(),
            });
        }
        Ok(())
    }

    fn ensure_authority_records(
        &self,
        authority: &LogicalCodebaseStore,
        registry: &IdentityRegistryStore,
        input: &CreateRepositoryInput,
        source_identity: &RepositorySourceIdentity,
        identity: &RepositoryIdentityAllocation,
        repository: &RepositoryRecord,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&identity.physical_repository_id)?;
        let member = match authority
            .load_member(&input.project_id, identity.logical_repository_id)?
        {
            Some(member) => {
                let expected = identity.member_record(
                    input,
                    source_identity,
                    member.ordinal,
                    &member.created_at,
                );
                if member != expected {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "logical_codebase_member",
                        id: identity.logical_repository_id.0.to_string(),
                    });
                }
                member
            }
            None => {
                let ordinal = authority
                    .list_members(&input.project_id)?
                    .iter()
                    .map(|member| member.ordinal)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| ProductStoreError::InvalidRecord {
                        kind: "logical_codebase_member",
                        reason: "repository ordinal overflow".to_string(),
                    })?;
                let member =
                    identity.member_record(input, source_identity, ordinal, &repository.created_at);
                authority.save_member(&input.project_id, &member)?;
                member
            }
        };

        let checkout =
            match authority.load_checkout(&input.project_id, identity.primary_checkout_id)? {
                Some(checkout) => {
                    let expected = identity.checkout_record(
                        &repository.path,
                        source_identity,
                        &checkout.created_at,
                    );
                    if checkout != expected {
                        return Err(ProductStoreError::IdentityMismatch {
                            kind: "repository_checkout",
                            id: identity.primary_checkout_id.0.to_string(),
                        });
                    }
                    checkout
                }
                None => {
                    let checkout = identity.checkout_record(
                        &repository.path,
                        source_identity,
                        &repository.created_at,
                    );
                    authority.save_checkout(&input.project_id, &checkout)?;
                    checkout
                }
            };

        let expected_registry = IdentityRegistryEntry::active(
            source_identity.clone(),
            identity.logical_repository_id,
            identity.physical_repository_id.clone(),
            identity.primary_checkout_id,
            input.idempotency_key.clone(),
        );
        match registry.find_by_source(&input.project_id, source_identity)? {
            Some(existing) if existing == expected_registry => {}
            Some(_) => {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "identity_registry",
                    id: identity.physical_repository_id.clone(),
                });
            }
            None => registry.upsert_active(&input.project_id, expected_registry)?,
        }

        self.ensure_manifest_membership(
            authority,
            &input.project_id,
            member.logical_repository_id,
        )?;
        debug_assert_eq!(checkout.checkout_id, identity.primary_checkout_id);
        Ok(())
    }

    fn ensure_manifest_membership(
        &self,
        authority: &LogicalCodebaseStore,
        project_id: &str,
        member_id: LogicalRepositoryId,
    ) -> Result<(), ProductStoreError> {
        let mut manifest =
            authority
                .load_manifest(project_id)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "logical_codebase_manifest",
                    id: project_id.to_string(),
                })?;
        if manifest.member_ids.contains(&member_id) {
            return Ok(());
        }
        manifest.member_ids.push(member_id);
        manifest.membership_revision =
            manifest.membership_revision.checked_add(1).ok_or_else(|| {
                ProductStoreError::InvalidRecord {
                    kind: "logical_codebase_manifest",
                    reason: "membership_revision overflow".to_string(),
                }
            })?;
        manifest.updated_at = Utc::now().to_rfc3339();
        authority.save_manifest(project_id, &manifest)
    }

    fn ensure_compatibility_projection(
        &self,
        repository: &RepositoryRecord,
    ) -> Result<(), ProductStoreError> {
        let mut repositories = self.list_compatibility_projection(&repository.project_id)?;
        match repositories
            .iter()
            .find(|existing| existing.id == repository.id)
        {
            Some(existing) if existing == repository => return Ok(()),
            Some(_) => {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "repository_projection",
                    id: repository.id.clone(),
                });
            }
            None => {}
        }
        repositories.push(repository.clone());
        write_json(&self.repos_path(&repository.project_id), &repositories)
    }

    fn list_compatibility_projection(
        &self,
        project_id: &str,
    ) -> Result<Vec<RepositoryRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        let path = self.repos_path(project_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        read_json(&path)
    }

    pub fn find_by_path(
        &self,
        project_id: &str,
        path: &Path,
    ) -> Result<Option<RepositoryRecord>, ProductStoreError> {
        let canonical_path = canonicalize_repo_path(path)?;
        let canonical_text = canonical_path.to_string_lossy();
        let target_hash = repo_hash_for_path(canonical_text.as_ref());

        Ok(self.list(project_id)?.into_iter().find(|record| {
            if record.repo_hash == target_hash {
                return true;
            }

            fs::canonicalize(&record.path)
                .map(|record_path| record_path == canonical_path)
                .unwrap_or_else(|_| record.path.to_string_lossy() == canonical_text)
        }))
    }

    pub fn delete(&self, project_id: &str, repository_id: &str) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(repository_id)?;
        let mut repositories = self.list(project_id)?;
        let initial_len = repositories.len();
        repositories.retain(|record| record.id != repository_id);
        if repositories.len() == initial_len {
            return Err(ProductStoreError::NotFound {
                kind: "repository",
                id: repository_id.to_string(),
            });
        }
        write_json(&self.repos_path(project_id), &repositories)
    }

    fn repos_path(&self, project_id: &str) -> PathBuf {
        self.paths.project_root(project_id).join("repos.json")
    }
}

#[derive(Debug, Clone)]
struct RepositoryIdentityAllocation {
    physical_repository_id: String,
    logical_repository_id: LogicalRepositoryId,
    primary_checkout_id: RepositoryCheckoutId,
    created_at: String,
}

impl RepositoryIdentityAllocation {
    fn new() -> Self {
        let physical_repository_id = format!("repository_{}", Uuid::new_v4().simple());
        // This physical ID is later persisted in the authority records before it
        // is used by any compatibility-projection path.
        debug_assert!(validate_relative_id(&physical_repository_id).is_ok());
        Self {
            physical_repository_id,
            logical_repository_id: LogicalRepositoryId(Uuid::new_v4()),
            primary_checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
            created_at: Utc::now().to_rfc3339(),
        }
    }

    fn repository_record(
        &self,
        input: &CreateRepositoryInput,
        canonical_path: PathBuf,
    ) -> RepositoryRecord {
        let repo_path_text = canonical_path.to_string_lossy();
        RepositoryRecord {
            id: self.physical_repository_id.clone(),
            project_id: input.project_id.clone(),
            name: input.name.clone(),
            repo_hash: repo_hash_for_path(repo_path_text.as_ref()),
            runtime_root: canonical_path.join(".aria/runtime"),
            path: canonical_path,
            default_policy_preset: input
                .default_policy_preset
                .clone()
                .unwrap_or_else(|| "manual-write".to_string()),
            default_provider_mode: input
                .default_provider_mode
                .clone()
                .unwrap_or_else(|| "fake".to_string()),
            created_at: self.created_at.clone(),
            updated_at: self.created_at.clone(),
            logical_repository_id: Some(self.logical_repository_id),
            primary_checkout_id: Some(self.primary_checkout_id),
            identity_schema_version: 1,
        }
    }

    fn member_record(
        &self,
        input: &CreateRepositoryInput,
        source_identity: &RepositorySourceIdentity,
        ordinal: u32,
        created_at: &str,
    ) -> CodebaseMemberRecord {
        CodebaseMemberRecord {
            logical_repository_id: self.logical_repository_id,
            physical_repository_id: self.physical_repository_id.clone(),
            alias: input.name.clone(),
            role: "repository".to_string(),
            ordinal,
            source_identity: source_identity.clone(),
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![self.primary_checkout_id],
            status: MemberStatus::Active,
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
        }
    }

    fn checkout_record(
        &self,
        canonical_path: &Path,
        source_identity: &RepositorySourceIdentity,
        created_at: &str,
    ) -> RepositoryCheckoutRecord {
        RepositoryCheckoutRecord {
            checkout_id: self.primary_checkout_id,
            logical_repository_id: self.logical_repository_id,
            physical_repository_id: self.physical_repository_id.clone(),
            kind: CheckoutKind::Main,
            canonical_path: canonical_path.to_path_buf(),
            checkout_path_hash: repo_hash_for_path(canonical_path.to_string_lossy().as_ref()),
            git_dir_identity: source_identity.git_dir_identity(),
            revision: None,
            availability: CheckoutAvailability::Available,
            observed_at: created_at.to_string(),
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RepositoryCreateReceipt {
    idempotency_key: String,
    input_digest: String,
    repository: RepositoryRecord,
}

impl RepositoryCreateReceipt {
    fn new(
        input: &CreateRepositoryInput,
        canonical_path: &Path,
        repository: RepositoryRecord,
    ) -> Self {
        Self {
            idempotency_key: input.idempotency_key.clone(),
            input_digest: create_input_digest(input, canonical_path),
            repository,
        }
    }

    fn validate_input(
        &self,
        input: &CreateRepositoryInput,
        canonical_path: &Path,
    ) -> Result<(), ProductStoreError> {
        if self.input_digest != create_input_digest(input, canonical_path) {
            return Err(ProductStoreError::Conflict {
                kind: "idempotency_key_reused",
                id: input.idempotency_key.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct RepositoryCommandReceiptStore {
    paths: ProductAppPaths,
}

impl RepositoryCommandReceiptStore {
    fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    fn find_create(
        &self,
        project_id: &str,
        idempotency_key: &str,
    ) -> Result<Option<RepositoryCreateReceipt>, ProductStoreError> {
        let path = self.create_receipt_path(project_id, idempotency_key)?;
        if !path.exists() {
            return Ok(None);
        }
        let receipt: RepositoryCreateReceipt = read_json(&path)?;
        if receipt.idempotency_key != idempotency_key {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "repository_create_receipt",
                id: idempotency_key.to_string(),
            });
        }
        Ok(Some(receipt))
    }

    fn save_create(
        &self,
        project_id: &str,
        receipt: &RepositoryCreateReceipt,
    ) -> Result<(), ProductStoreError> {
        let path = self.create_receipt_path(project_id, &receipt.idempotency_key)?;
        if path.exists() {
            let existing: RepositoryCreateReceipt = read_json(&path)?;
            if existing == *receipt {
                return Ok(());
            }
            return Err(ProductStoreError::Conflict {
                kind: "idempotency_key_reused",
                id: receipt.idempotency_key.clone(),
            });
        }
        write_json(&path, receipt)
    }

    fn create_receipt_path(
        &self,
        project_id: &str,
        idempotency_key: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(idempotency_key)?;
        Ok(self
            .paths
            .logical_codebase_root(project_id)
            .join("command-receipts")
            .join(format!("create-{idempotency_key}.json")))
    }
}

struct RepositorySourceResolver;

impl RepositorySourceResolver {
    fn resolve(canonical_path: &Path) -> Result<RepositorySourceIdentity, ProductStoreError> {
        let git_dir_output = run_git(canonical_path, &["rev-parse", "--git-dir"])?;
        let git_dir = PathBuf::from(git_dir_output.trim());
        let git_dir = if git_dir.is_absolute() {
            git_dir
        } else {
            canonical_path.join(git_dir)
        };
        let canonical_git_dir = canonicalize_repo_path(&git_dir)?;
        let canonical_origin = match run_git(canonical_path, &["remote", "get-url", "origin"]) {
            Ok(value) => {
                let origin = value.trim();
                (!origin.is_empty()).then(|| origin.to_string())
            }
            Err(ProductStoreError::Io(message)) if message.contains("git exited") => None,
            Err(error) => return Err(error),
        };
        Ok(RepositorySourceIdentity::from_git_parts(
            canonical_path,
            canonical_git_dir,
            canonical_origin,
        ))
    }
}

fn resolve_repository_source(
    canonical_path: &Path,
) -> Result<RepositorySourceIdentity, ProductStoreError> {
    RepositorySourceResolver::resolve(canonical_path)
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

fn create_input_digest(input: &CreateRepositoryInput, canonical_path: &Path) -> String {
    let payload = format!(
        "{}\\0{}\\0{}\\0{}\\0{}\\0{}",
        input.project_id,
        input.name,
        canonical_path.to_string_lossy(),
        input.default_policy_preset.as_deref().unwrap_or("<none>"),
        input.default_provider_mode.as_deref().unwrap_or("<none>"),
        "repository_create_v1",
    );
    format!("sha256:{:x}", Sha256::digest(payload.as_bytes()))
}

fn canonicalize_repo_path(path: &Path) -> Result<PathBuf, ProductStoreError> {
    fs::canonicalize(path)
        .map_err(|error| ProductStoreError::Io(format!("canonicalize {}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::product::logical_codebase::LogicalCodebaseFeature;
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    struct RepositoryStoreFixture {
        _root: tempfile::TempDir,
        store: RepositoryStore,
        git_root: PathBuf,
    }

    fn repository_store_fixture_with_feature_enabled() -> RepositoryStoreFixture {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path());
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        let git_root = root.path().join("api");
        fs::create_dir_all(&git_root).unwrap();
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&git_root)
            .status()
            .unwrap();
        assert!(status.success());

        RepositoryStoreFixture {
            _root: root,
            store: RepositoryStore::with_logical_codebase_feature(
                paths,
                LogicalCodebaseFeature::enabled(),
            ),
            git_root,
        }
    }

    #[test]
    fn create_is_idempotent_uses_uuid_physical_id_and_rejects_same_source() {
        let fixture = repository_store_fixture_with_feature_enabled();
        let input = CreateRepositoryInput {
            project_id: "project_0001".into(),
            name: "api".into(),
            path: fixture.git_root.clone(),
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: "register-api-1".into(),
        };

        let first = fixture.store.create(input.clone()).unwrap();
        let replay = fixture.store.create(input).unwrap();

        assert_eq!(first, replay);
        assert!(first.id.starts_with("repository_"));
        assert!(uuid::Uuid::parse_str(first.id.strip_prefix("repository_").unwrap()).is_ok());
        assert!(matches!(
            fixture.store.create(CreateRepositoryInput {
                project_id: "project_0001".into(),
                name: "api".into(),
                path: fixture.git_root.clone(),
                default_policy_preset: Some("automatic".into()),
                default_provider_mode: None,
                idempotency_key: "register-api-1".into(),
            }),
            Err(ProductStoreError::Conflict {
                kind: "idempotency_key_reused",
                ..
            })
        ));
        assert!(matches!(
            fixture.store.create(CreateRepositoryInput {
                project_id: "project_0001".into(),
                name: "api".into(),
                path: fixture.git_root,
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: "register-api-2".into(),
            }),
            Err(ProductStoreError::Conflict {
                kind: "repository_already_registered",
                ..
            })
        ));
    }

    #[test]
    fn repository_initialization_launch_is_nameable_through_repository_store() {
        let _: Option<crate::product::repository_store::RepositoryInitializationLaunch> = None;
    }
}
