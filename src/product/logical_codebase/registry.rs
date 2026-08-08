use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::{
    LogicalRepositoryId, RepositoryCheckoutId, RepositorySourceIdentity,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityRegistryState {
    Active,
    Tombstoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRegistryEntry {
    pub source_identity: RepositorySourceIdentity,
    pub logical_repository_id: LogicalRepositoryId,
    pub physical_repository_id: String,
    pub primary_checkout_id: RepositoryCheckoutId,
    pub state: IdentityRegistryState,
    pub created_by_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reactivated_at: Option<String>,
}

impl IdentityRegistryEntry {
    pub fn active(
        source_identity: RepositorySourceIdentity,
        logical_repository_id: LogicalRepositoryId,
        physical_repository_id: String,
        primary_checkout_id: RepositoryCheckoutId,
        created_by_key: String,
    ) -> Self {
        Self {
            source_identity,
            logical_repository_id,
            physical_repository_id,
            primary_checkout_id,
            state: IdentityRegistryState::Active,
            created_by_key,
            deleted_at: None,
            delete_operation_id: None,
            reactivated_at: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRegistry {
    pub schema_version: u16,
    #[serde(default)]
    pub entries: Vec<IdentityRegistryEntry>,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct IdentityRegistryStore {
    paths: ProductAppPaths,
}

impl IdentityRegistryStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn load(&self, project_id: &str) -> Result<Option<IdentityRegistry>, ProductStoreError> {
        let path = self.path(project_id)?;
        if !path.exists() {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    pub fn find_by_source(
        &self,
        project_id: &str,
        source_identity: &RepositorySourceIdentity,
    ) -> Result<Option<IdentityRegistryEntry>, ProductStoreError> {
        let registry = match self.load(project_id)? {
            Some(registry) => registry,
            None => return Ok(None),
        };

        match registry
            .entries
            .iter()
            .find(|entry| entry.source_identity.key_digest == source_identity.key_digest)
        {
            Some(entry) if entry.source_identity != *source_identity => {
                Err(source_identity_collision(source_identity))
            }
            Some(entry) => Ok(Some(entry.clone())),
            None => Ok(None),
        }
    }

    pub fn upsert_active(
        &self,
        project_id: &str,
        entry: IdentityRegistryEntry,
    ) -> Result<(), ProductStoreError> {
        let mut registry = self.load(project_id)?.unwrap_or_else(|| IdentityRegistry {
            schema_version: 1,
            entries: Vec::new(),
            updated_at: String::new(),
        });
        if let Some(existing) = registry
            .entries
            .iter_mut()
            .find(|value| value.source_identity.key_digest == entry.source_identity.key_digest)
        {
            if existing.source_identity != entry.source_identity {
                return Err(source_identity_collision(&entry.source_identity));
            }
            if existing.state == IdentityRegistryState::Tombstoned {
                return Err(ProductStoreError::Conflict {
                    kind: "repository_source_tombstoned",
                    id: existing.physical_repository_id.clone(),
                });
            }
            if existing != &entry {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "identity_registry",
                    id: existing.physical_repository_id.clone(),
                });
            }
            return Ok(());
        }

        registry.entries.push(entry);
        registry.updated_at = Utc::now().to_rfc3339();
        write_json(&self.path(project_id)?, &registry)
    }

    pub fn tombstone(
        &self,
        project_id: &str,
        source_identity: &RepositorySourceIdentity,
        delete_operation_id: &str,
        deleted_at: &str,
    ) -> Result<(), ProductStoreError> {
        let mut registry = self.registry_for_mutation(project_id)?;
        let entry = find_entry_mut(&mut registry, source_identity)?;
        if entry.state == IdentityRegistryState::Tombstoned {
            return Err(ProductStoreError::Conflict {
                kind: "repository_source_tombstoned",
                id: entry.physical_repository_id.clone(),
            });
        }

        entry.state = IdentityRegistryState::Tombstoned;
        entry.deleted_at = Some(deleted_at.to_string());
        entry.delete_operation_id = Some(delete_operation_id.to_string());
        registry.updated_at = deleted_at.to_string();
        write_json(&self.path(project_id)?, &registry)
    }

    pub fn reactivate_tombstoned_source(
        &self,
        project_id: &str,
        source_identity: &RepositorySourceIdentity,
        _reactivate_operation_id: &str,
        reactivated_at: &str,
    ) -> Result<(), ProductStoreError> {
        let mut registry = self.registry_for_mutation(project_id)?;
        let entry = find_entry_mut(&mut registry, source_identity)?;
        if entry.state == IdentityRegistryState::Active {
            return Err(ProductStoreError::Conflict {
                kind: "repository_source_active",
                id: entry.physical_repository_id.clone(),
            });
        }

        entry.state = IdentityRegistryState::Active;
        entry.reactivated_at = Some(reactivated_at.to_string());
        registry.updated_at = reactivated_at.to_string();
        write_json(&self.path(project_id)?, &registry)
    }

    fn path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        Ok(self
            .paths
            .logical_codebase_root(project_id)
            .join("identity-registry.json"))
    }

    fn registry_for_mutation(
        &self,
        project_id: &str,
    ) -> Result<IdentityRegistry, ProductStoreError> {
        self.load(project_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "identity_registry",
                id: project_id.to_string(),
            })
    }
}

fn find_entry_mut<'a>(
    registry: &'a mut IdentityRegistry,
    source_identity: &RepositorySourceIdentity,
) -> Result<&'a mut IdentityRegistryEntry, ProductStoreError> {
    match registry
        .entries
        .iter_mut()
        .find(|entry| entry.source_identity.key_digest == source_identity.key_digest)
    {
        Some(entry) if entry.source_identity != *source_identity => {
            Err(source_identity_collision(source_identity))
        }
        Some(entry) => Ok(entry),
        None => Err(ProductStoreError::NotFound {
            kind: "repository_source",
            id: source_identity.key_digest.clone(),
        }),
    }
}

fn source_identity_collision(source_identity: &RepositorySourceIdentity) -> ProductStoreError {
    ProductStoreError::Conflict {
        kind: "source_identity_collision",
        id: source_identity.key_digest.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        LogicalRepositoryId, RepositoryCheckoutId, RepositorySourceIdentity,
    };
    use uuid::Uuid;

    #[test]
    fn tombstoned_source_requires_explicit_reactivation_and_detects_evidence_collision() {
        let temp = tempfile::tempdir().unwrap();
        let store = IdentityRegistryStore::new(ProductAppPaths::new(temp.path()));
        let source = RepositorySourceIdentity::from_git_parts(
            std::path::Path::new("/workspace/api"),
            "/workspace/api/.git".into(),
            Some("ssh://git@example.test/acme/api.git".into()),
        );
        let entry = IdentityRegistryEntry::active(
            source.clone(),
            LogicalRepositoryId(Uuid::new_v4()),
            "repository_test".into(),
            RepositoryCheckoutId(Uuid::new_v4()),
            "test:create".into(),
        );
        store.upsert_active("project_0001", entry).unwrap();
        store
            .tombstone("project_0001", &source, "delete-1", "2026-08-08T00:00:00Z")
            .unwrap();

        assert_eq!(
            store
                .find_by_source("project_0001", &source)
                .unwrap()
                .unwrap()
                .state,
            IdentityRegistryState::Tombstoned
        );
        assert!(matches!(
            store.upsert_active(
                "project_0001",
                IdentityRegistryEntry::active(
                    source.clone(),
                    LogicalRepositoryId(Uuid::new_v4()),
                    "repository_new".into(),
                    RepositoryCheckoutId(Uuid::new_v4()),
                    "test:create-2".into(),
                ),
            ),
            Err(ProductStoreError::Conflict {
                kind: "repository_source_tombstoned",
                ..
            })
        ));
        store
            .reactivate_tombstoned_source(
                "project_0001",
                &source,
                "reactivate-1",
                "2026-08-08T00:01:00Z",
            )
            .unwrap();
        assert_eq!(
            store
                .find_by_source("project_0001", &source)
                .unwrap()
                .unwrap()
                .state,
            IdentityRegistryState::Active
        );
    }
}
