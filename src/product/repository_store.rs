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
    IdentityMigrationJournal, IdentityMigrationJournalStore, IdentityRegistryEntry,
    IdentityRegistryState, IdentityRegistryStore, LogicalCodebaseFeature, LogicalCodebaseStore,
    LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
    RepositoryReferenceScanner, RepositorySourceIdentity, RepositoryType,
};
use crate::product::models::{ProjectRecord, RepositoryRecord};

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

/// A deletion command must always use a caller-owned stable operation id.
/// `allow_tombstone_reactivation` is deliberately retained as an explicit
/// rejected input so callers cannot introduce a force-delete side channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteRepositoryCommand {
    pub operation_id: String,
    pub expected_updated_at: Option<String>,
    pub allow_tombstone_reactivation: bool,
}

/// The durable outcome of a repository delete command.
///
/// A feature-disabled legacy deletion has no logical identity or tombstone;
/// those fields are `None` and `legacy_delete` is `true`. Enabled logical
/// codebase deletes always return all identity fields and `legacy_delete=false`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryDeletionReceipt {
    pub physical_repository_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_repository_id: Option<LogicalRepositoryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_id: Option<RepositoryCheckoutId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tombstone_operation_id: Option<String>,
    pub deleted_at: String,
    #[serde(default)]
    pub legacy_delete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionSource {
    LogicalAuthority,
    LegacyProjection,
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

    /// Transitional project-scoped feature detection. It is enabled only when
    /// this project has logical-codebase storage (new or legacy/migrated).
    /// R3/R5 must replace this with per-codebase detection from the request or
    /// issue; a project itself no longer has a repository-mode attribute.
    pub fn for_project(paths: ProductAppPaths, project: &ProjectRecord) -> Self {
        let feature = LogicalCodebaseStore::new(paths.clone())
            .has_any_storage(&project.id)
            .map(|has_storage| {
                if has_storage {
                    LogicalCodebaseFeature::enabled()
                } else {
                    LogicalCodebaseFeature::disabled()
                }
            })
            .unwrap_or_else(|_| LogicalCodebaseFeature::disabled());
        Self::with_logical_codebase_feature(paths, feature)
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
        if self.logical_codebase_feature.is_enabled()
            && LogicalCodebaseStore::new(self.paths.clone())
                .load_manifest(project_id)?
                .is_none()
        {
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
}

include!("repository_store_parts/create.inc.rs");
include!("repository_store_parts/find.inc.rs");
include!("repository_store_parts/delete.inc.rs");
include!("repository_store_parts/resolve.inc.rs");
include!("repository_store_parts/helpers.inc.rs");

impl RepositoryStore {
    fn repos_path(&self, project_id: &str) -> PathBuf {
        self.paths.project_root(project_id).join("repos.json")
    }
}
include!("repository_store_parts/tests.inc.rs");
