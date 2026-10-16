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
    /// Bug3 症状2（2026-10-16）：所属逻辑代码库已软删（tombstone），source
    /// 已释放、可被新登记占用；LC restore 时按 (source, logical_repository_id)
    /// 复活回 Active。与 Tombstoned（repo 级硬删）语义互斥。
    Released,
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
    /// Bug3：LC 软删释放（Active→Released）的时间戳；复活/被新登记覆盖时清除。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released_at: Option<String>,
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
            released_at: None,
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

    pub fn find_by_logical_id(
        &self,
        project_id: &str,
        logical_repository_id: LogicalRepositoryId,
    ) -> Result<Vec<IdentityRegistryEntry>, ProductStoreError> {
        let registry = match self.load(project_id)? {
            Some(registry) => registry,
            None => return Ok(Vec::new()),
        };
        Ok(registry
            .entries
            .into_iter()
            .filter(|entry| entry.logical_repository_id == logical_repository_id)
            .collect())
    }

    pub fn find_by_physical_id(
        &self,
        project_id: &str,
        physical_repository_id: &str,
    ) -> Result<Vec<IdentityRegistryEntry>, ProductStoreError> {
        validate_relative_id(physical_repository_id)?;
        let registry = match self.load(project_id)? {
            Some(registry) => registry,
            None => return Ok(Vec::new()),
        };
        Ok(registry
            .entries
            .into_iter()
            .filter(|entry| entry.physical_repository_id == physical_repository_id)
            .collect())
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
        if let Some(index) = registry
            .entries
            .iter()
            .position(|value| value.source_identity.key_digest == entry.source_identity.key_digest)
        {
            let existing = &registry.entries[index];
            if existing.source_identity != entry.source_identity {
                return Err(source_identity_collision(&entry.source_identity));
            }
            if existing.state == IdentityRegistryState::Tombstoned {
                return Err(ProductStoreError::Conflict {
                    kind: "repository_source_tombstoned",
                    id: existing.physical_repository_id.clone(),
                });
            }
            // Bug3 症状2：Released（所属 LC 已软删释放）条目可被新登记按新身份
            // 覆盖——source 已释放；旧 LC 的成员 record 留在墓碑子树内，待其
            // restore 时按归属对账（届时归属已换则 fail-closed，见 reclaim）。
            if existing.state == IdentityRegistryState::Released {
                registry.entries[index] = entry;
                registry.updated_at = Utc::now().to_rfc3339();
                return write_json(&self.path(project_id)?, &registry);
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

    /// Bug3 症状2：LC 软删时释放其成员 source 命中的 Active 条目（→Released）。
    /// 条目缺席（登记链 crash-window 已知形态）与 Tombstoned（repo 级删除语义）
    /// 一律跳过；digest 命中但身份不同 → collision fail-closed。
    pub fn release_sources(
        &self,
        project_id: &str,
        sources: &[RepositorySourceIdentity],
        released_at: &str,
    ) -> Result<(), ProductStoreError> {
        if sources.is_empty() {
            return Ok(());
        }
        let Some(mut registry) = self.load(project_id)? else {
            return Ok(());
        };
        let mut changed = false;
        for source in sources {
            let Some(entry) = registry
                .entries
                .iter_mut()
                .find(|entry| entry.source_identity.key_digest == source.key_digest)
            else {
                continue;
            };
            if entry.source_identity != *source {
                return Err(source_identity_collision(source));
            }
            if entry.state == IdentityRegistryState::Active {
                entry.state = IdentityRegistryState::Released;
                entry.released_at = Some(released_at.to_string());
                changed = true;
            }
        }
        if changed {
            registry.updated_at = released_at.to_string();
            write_json(&self.path(project_id)?, &registry)?;
        }
        Ok(())
    }

    /// Bug3 修复①：LC restore 时按 (source, logical_repository_id) 复活 Released
    /// 条目（→Active，记 reactivated_at）。Active 且归属一致 → 幂等跳过；归属
    /// 已换（软删期间 source 被其他 LC/单仓重新登记占用）→ Conflict 整体失败，
    /// 墓碑保留由调用方裁决；Tombstoned / 条目缺席 → 跳过（repo 级删除与
    /// crash-window 语义不属于 LC restore 职责）。
    pub fn reclaim_released_sources(
        &self,
        project_id: &str,
        claims: &[(RepositorySourceIdentity, LogicalRepositoryId)],
        reclaimed_at: &str,
    ) -> Result<(), ProductStoreError> {
        if claims.is_empty() {
            return Ok(());
        }
        let Some(mut registry) = self.load(project_id)? else {
            return Ok(());
        };
        let mut changed = false;
        for (source, expected_logical_id) in claims {
            let Some(entry) = registry
                .entries
                .iter_mut()
                .find(|entry| entry.source_identity.key_digest == source.key_digest)
            else {
                continue;
            };
            if entry.source_identity != *source {
                return Err(source_identity_collision(source));
            }
            match entry.state {
                IdentityRegistryState::Released => {
                    if entry.logical_repository_id != *expected_logical_id {
                        return Err(ProductStoreError::Conflict {
                            kind: "logical_codebase_restore_source_taken",
                            id: source.key_digest.clone(),
                        });
                    }
                    entry.state = IdentityRegistryState::Active;
                    entry.reactivated_at = Some(reclaimed_at.to_string());
                    entry.released_at = None;
                    changed = true;
                }
                IdentityRegistryState::Active => {
                    if entry.logical_repository_id != *expected_logical_id {
                        return Err(ProductStoreError::Conflict {
                            kind: "logical_codebase_restore_source_taken",
                            id: source.key_digest.clone(),
                        });
                    }
                    // 幂等：已是目标态（前一次 restore 中途失败后重跑收敛）。
                }
                IdentityRegistryState::Tombstoned => {}
            }
        }
        if changed {
            registry.updated_at = reclaimed_at.to_string();
            write_json(&self.path(project_id)?, &registry)?;
        }
        Ok(())
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

    #[test]
    fn released_source_is_reclaimable_overwritable_and_conflicts_on_takeover() {
        // Bug3 症状2：LC 软删释放（Active→Released）→ 新登记可覆盖 →
        // 旧归属 reclaim 时 fail-closed；未换主的 Released 可复活回 Active。
        let temp = tempfile::tempdir().unwrap();
        let store = IdentityRegistryStore::new(ProductAppPaths::new(temp.path()));
        let source = RepositorySourceIdentity::from_git_parts(
            std::path::Path::new("/workspace/api"),
            "/workspace/api/.git".into(),
            None,
        );
        let old_logical_id = LogicalRepositoryId(Uuid::new_v4());
        store
            .upsert_active(
                "project_0001",
                IdentityRegistryEntry::active(
                    source.clone(),
                    old_logical_id,
                    "repository_old".into(),
                    RepositoryCheckoutId(Uuid::new_v4()),
                    "test:create-old".into(),
                ),
            )
            .unwrap();

        // 释放：Active→Released，记 released_at；未命中 source / 缺席条目跳过。
        store
            .release_sources("project_0001", &[source.clone()], "2026-10-16T00:00:00Z")
            .unwrap();
        let released = store
            .find_by_source("project_0001", &source)
            .unwrap()
            .unwrap();
        assert_eq!(released.state, IdentityRegistryState::Released);
        assert_eq!(
            released.released_at.as_deref(),
            Some("2026-10-16T00:00:00Z")
        );

        // 新 LC 重新登记：Released 条目被新身份覆盖（不再 IdentityMismatch）。
        let new_logical_id = LogicalRepositoryId(Uuid::new_v4());
        store
            .upsert_active(
                "project_0001",
                IdentityRegistryEntry::active(
                    source.clone(),
                    new_logical_id,
                    "repository_new".into(),
                    RepositoryCheckoutId(Uuid::new_v4()),
                    "test:create-new".into(),
                ),
            )
            .unwrap();

        // 旧归属 reclaim：归属已换 → Conflict（restore fail-closed，墓碑保留）。
        assert!(matches!(
            store.reclaim_released_sources(
                "project_0001",
                &[(source.clone(), old_logical_id)],
                "2026-10-16T00:01:00Z",
            ),
            Err(ProductStoreError::Conflict {
                kind: "logical_codebase_restore_source_taken",
                ..
            })
        ));

        // 新归属 reclaim：Active 且归属一致 → 幂等成功、无状态变化。
        store
            .reclaim_released_sources(
                "project_0001",
                &[(source.clone(), new_logical_id)],
                "2026-10-16T00:01:30Z",
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

        // 再次释放后按原归属复活：Released→Active，released_at 清空。
        store
            .release_sources("project_0001", &[source.clone()], "2026-10-16T00:02:00Z")
            .unwrap();
        store
            .reclaim_released_sources(
                "project_0001",
                &[(source.clone(), new_logical_id)],
                "2026-10-16T00:03:00Z",
            )
            .unwrap();
        let reclaimed = store
            .find_by_source("project_0001", &source)
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed.state, IdentityRegistryState::Active);
        assert_eq!(reclaimed.released_at, None);
        assert_eq!(
            reclaimed.reactivated_at.as_deref(),
            Some("2026-10-16T00:03:00Z")
        );
    }
}
