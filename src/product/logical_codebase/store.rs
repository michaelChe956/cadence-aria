use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::{
    CodebaseMemberRecord, LogicalRepositoryId, RepositoryCheckoutId, RepositoryCheckoutRecord,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalCodebaseLayout {
    CommonNonGitParent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalCodebaseManifest {
    pub schema_version: u16,
    pub project_id: String,
    pub logical_codebase_id: Uuid,
    pub provider_context_root: PathBuf,
    pub layout: LogicalCodebaseLayout,
    pub membership_revision: u64,
    #[serde(default)]
    pub member_ids: Vec<LogicalRepositoryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_aggregate_index_id: Option<String>,
    #[serde(default)]
    pub context_policy_digest: String,
    pub created_at: String,
    pub updated_at: String,
}

impl LogicalCodebaseManifest {
    pub fn new(
        project_id: &str,
        provider_context_root: PathBuf,
        member_ids: Vec<LogicalRepositoryId>,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            schema_version: 1,
            project_id: project_id.to_string(),
            logical_codebase_id: Uuid::new_v4(),
            provider_context_root,
            layout: LogicalCodebaseLayout::CommonNonGitParent,
            membership_revision: 1,
            member_ids,
            active_aggregate_index_id: None,
            context_policy_digest: String::new(),
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalCodebaseRecord {
    pub id: String,
    pub name: String,
    pub aggregate_root: PathBuf,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalCodebaseCreateInput {
    pub name: String,
    pub aggregate_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LogicalCodebaseTombstone {
    deleted_at: String,
}

/// Resolves the durable subtree root a logical-codebase-scoped store must
/// use: the legacy alias codebase keeps its authoritative data under the old
/// `logical-codebase/` subtree, every other codebase lives under
/// `logical-codebases/{lc_id}/` (v1.3 layout). `None` keeps the legacy
/// project-scoped behavior unchanged for pre-R3 call sites.
pub(crate) fn lc_scope_root(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &Option<String>,
) -> Result<PathBuf, ProductStoreError> {
    validate_relative_id(project_id)?;
    match lc_id {
        Some(lc_id) if *lc_id != legacy_logical_codebase_id(project_id) => {
            validate_relative_id(lc_id)?;
            Ok(paths.logical_codebases_root(project_id).join(lc_id))
        }
        _ => Ok(paths.logical_codebase_root(project_id)),
    }
}

#[derive(Debug, Clone)]
pub struct LogicalCodebaseStore {
    paths: ProductAppPaths,
    lc_id: Option<String>,
}

impl LogicalCodebaseStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths, lc_id: None }
    }

    /// Scopes every manifest/member/checkout/registration path of this store
    /// to one logical codebase subtree. The legacy alias codebase resolves to
    /// the legacy project-scoped root, so its behavior is byte-identical to an
    /// unscoped store.
    pub fn for_lc(paths: ProductAppPaths, lc_id: impl Into<String>) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id.into()),
        }
    }

    /// Exposes the product paths backing this store for sibling services that
    /// rebuild a scoped view from the same roots.
    pub(crate) fn paths(&self) -> &ProductAppPaths {
        &self.paths
    }

    fn scope_root(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        lc_scope_root(&self.paths, project_id, &self.lc_id)
    }

    pub fn create(
        &self,
        project_id: &str,
        input: LogicalCodebaseCreateInput,
    ) -> Result<LogicalCodebaseRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        if input.name.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "logical_codebase_record",
                reason: "name must not be empty".to_string(),
            });
        }
        self.migrate_legacy(project_id)?;
        if self
            .list(project_id)?
            .iter()
            .any(|record| record.name == input.name)
        {
            return Err(ProductStoreError::Conflict {
                kind: "logical_codebase_name",
                id: input.name,
            });
        }
        let record = LogicalCodebaseRecord {
            id: format!("logical_codebase_{}", Uuid::new_v4().simple()),
            name: input.name,
            aggregate_root: input.aggregate_root,
            created_at: Utc::now().to_rfc3339(),
        };
        self.create_subtree(project_id, &record.id)?;
        write_json(&self.record_path(project_id, &record.id)?, &record)?;
        Ok(record)
    }

    pub fn get(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<Option<LogicalCodebaseRecord>, ProductStoreError> {
        self.migrate_legacy(project_id)?;
        self.get_existing(project_id, logical_codebase_id)
    }

    /// R2 统一 codebases 列表与 LC 详情的 per-LC manifest 读取：优先读
    /// `logical-codebases/{lc_id}/manifest.json`（v1.3 存储布局）；legacy 别名 LC
    /// 回退到旧 `logical-codebase/manifest.json`（迁移时子树副本可能随后续登记过期）。
    pub fn load_lc_manifest(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<Option<LogicalCodebaseManifest>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(logical_codebase_id)?;
        let subtree_manifest = self
            .paths
            .logical_codebase_record_root(project_id, logical_codebase_id)
            .join("manifest.json");
        if path_exists(&subtree_manifest)? {
            let manifest: LogicalCodebaseManifest = read_json(&subtree_manifest)?;
            self.validate_manifest_project(project_id, &manifest)?;
            return Ok(Some(manifest));
        }
        if logical_codebase_id == legacy_logical_codebase_id(project_id) {
            return self.load_manifest(project_id);
        }
        Ok(None)
    }

    /// per-LC 成员读取，回退语义与 [`LogicalCodebaseStore::load_lc_manifest`] 一致。
    /// 成员 record 暂不携带 lc_id（R3 登记端点换形后归属键落位）。
    pub fn list_lc_members(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<Vec<CodebaseMemberRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(logical_codebase_id)?;
        let subtree_members = self
            .paths
            .logical_codebase_record_root(project_id, logical_codebase_id)
            .join("members");
        if subtree_members.exists() {
            let mut members = Vec::new();
            for entry in std::fs::read_dir(&subtree_members).map_err(|error| {
                ProductStoreError::Io(format!("read {}: {error}", subtree_members.display()))
            })? {
                let entry =
                    entry.map_err(|error| ProductStoreError::Io(format!("read entry: {error}")))?;
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("json") {
                    continue;
                }
                let member: CodebaseMemberRecord = read_json(&path)?;
                validate_member_id(member.logical_repository_id, &member)?;
                members.push(member);
            }
            members.sort_by_key(|left| left.ordinal);
            return Ok(members);
        }
        if logical_codebase_id == legacy_logical_codebase_id(project_id) {
            return self.list_members(project_id);
        }
        Ok(Vec::new())
    }

    pub fn list(&self, project_id: &str) -> Result<Vec<LogicalCodebaseRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        self.migrate_legacy(project_id)?;
        let root = self.paths.logical_codebases_root(project_id);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };

        let mut records = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            if !entry
                .file_type()
                .map_err(|error| {
                    ProductStoreError::Io(format!("stat {}: {error}", entry.path().display()))
                })?
                .is_dir()
            {
                continue;
            }
            let id = entry.file_name().into_string().map_err(|value| {
                ProductStoreError::InvalidRecord {
                    kind: "logical_codebase_record",
                    reason: format!(
                        "logical codebase directory name is not UTF-8: {}",
                        PathBuf::from(value).display()
                    ),
                }
            })?;
            if let Some(record) = self.get_existing(project_id, &id)? {
                records.push(record);
            }
        }
        records.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(records)
    }

    pub fn delete_soft(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<(), ProductStoreError> {
        if self.get(project_id, logical_codebase_id)?.is_none() {
            return Err(ProductStoreError::NotFound {
                kind: "logical_codebase",
                id: logical_codebase_id.to_string(),
            });
        }
        write_json(
            &self.tombstone_path(project_id, logical_codebase_id)?,
            &LogicalCodebaseTombstone {
                deleted_at: Utc::now().to_rfc3339(),
            },
        )
    }

    /// Migrates the v1.2 project-scoped logical-codebase subtree once. The old
    /// endpoints continue to use that subtree as the default first-codebase
    /// compatibility alias until R3/R5 switch their paths to a supplied LC id.
    pub fn migrate_legacy(
        &self,
        project_id: &str,
    ) -> Result<Option<LogicalCodebaseRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        let legacy_root = self.paths.logical_codebase_root(project_id);
        if !legacy_manifest_exists(&legacy_root)? {
            return Ok(None);
        }

        with_exact_exclusive_lock(
            &self.paths.logical_codebase_migration_lock_path(project_id),
            || {
                let id = legacy_logical_codebase_id(project_id);
                if path_exists(&self.record_path(project_id, &id)?)? {
                    return self.load_record(project_id, &id).map(Some);
                }

                let aggregate_root = self
                    .load_manifest(project_id)?
                    .map(|manifest| manifest.provider_context_root)
                    .unwrap_or_else(|| legacy_root.clone());
                let record = LogicalCodebaseRecord {
                    id: id.clone(),
                    name: aggregate_root
                        .file_name()
                        .filter(|value| !value.is_empty())
                        .map(|value| value.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "logical-codebase".to_string()),
                    aggregate_root,
                    created_at: Utc::now().to_rfc3339(),
                };
                let destination = self.paths.logical_codebase_record_root(project_id, &id);
                self.create_subtree(project_id, &id)?;
                copy_directory_contents(&legacy_root, &destination)?;
                write_json(&self.record_path(project_id, &id)?, &record)?;
                Ok(Some(record))
            },
        )
    }

    pub fn has_any_storage(&self, project_id: &str) -> Result<bool, ProductStoreError> {
        validate_relative_id(project_id)?;
        if legacy_manifest_exists(&self.paths.logical_codebase_root(project_id))? {
            return Ok(true);
        }

        let root = self.paths.logical_codebases_root(project_id);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            let candidate = entry.path();
            if entry
                .file_type()
                .map_err(|error| {
                    ProductStoreError::Io(format!("stat {}: {error}", candidate.display()))
                })?
                .is_dir()
                && path_exists(&candidate.join("record.json"))?
                && !path_exists(&candidate.join("tombstone.json"))?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn get_existing(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<Option<LogicalCodebaseRecord>, ProductStoreError> {
        if !path_exists(&self.record_path(project_id, logical_codebase_id)?)?
            || path_exists(&self.tombstone_path(project_id, logical_codebase_id)?)?
        {
            return Ok(None);
        }
        self.load_record(project_id, logical_codebase_id).map(Some)
    }

    fn load_record(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<LogicalCodebaseRecord, ProductStoreError> {
        let path = self.record_path(project_id, logical_codebase_id)?;
        let record: LogicalCodebaseRecord = read_json(&path)?;
        self.validate_record(project_id, logical_codebase_id, &record)?;
        Ok(record)
    }

    fn create_subtree(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<(), ProductStoreError> {
        let root = self
            .paths
            .logical_codebase_record_root(project_id, logical_codebase_id);
        for directory in [
            root.join("members"),
            root.join("checkouts"),
            root.join("aggregate-indexes"),
            root.join("preflights"),
            root.join("registration-batches"),
            root.join("pointer-publications"),
        ] {
            std::fs::create_dir_all(&directory).map_err(|error| {
                ProductStoreError::Io(format!("create {}: {error}", directory.display()))
            })?;
        }
        Ok(())
    }

    pub fn record_path(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(logical_codebase_id)?;
        Ok(self
            .paths
            .logical_codebase_record_root(project_id, logical_codebase_id)
            .join("record.json"))
    }

    pub fn load_manifest(
        &self,
        project_id: &str,
    ) -> Result<Option<LogicalCodebaseManifest>, ProductStoreError> {
        let path = self.manifest_path(project_id)?;
        if !path_exists(&path)? {
            return Ok(None);
        }

        let manifest: LogicalCodebaseManifest = read_json(&path)?;
        self.validate_manifest_project(project_id, &manifest)?;
        Ok(Some(manifest))
    }

    pub fn list_manifests(&self) -> Result<Vec<LogicalCodebaseManifest>, ProductStoreError> {
        let projects_root = self.paths.projects_root();
        let entries = match std::fs::read_dir(&projects_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    projects_root.display()
                )));
            }
        };

        let mut manifests = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", projects_root.display()))
            })?;
            let path = entry.path();
            if !entry
                .file_type()
                .map_err(|error| {
                    ProductStoreError::Io(format!("stat {}: {error}", path.display()))
                })?
                .is_dir()
            {
                continue;
            }
            let project_id = entry.file_name().into_string().map_err(|value| {
                ProductStoreError::InvalidRecord {
                    kind: "logical_codebase_manifest",
                    reason: format!(
                        "project directory name is not UTF-8: {}",
                        PathBuf::from(value).display()
                    ),
                }
            })?;
            validate_relative_id(&project_id)?;
            if let Some(manifest) = self.load_manifest(&project_id)? {
                manifests.push(manifest);
            }
        }
        manifests.sort_by(|left, right| left.project_id.cmp(&right.project_id));
        Ok(manifests)
    }

    pub fn save_manifest(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
    ) -> Result<(), ProductStoreError> {
        self.validate_manifest_project(project_id, manifest)?;
        if manifest.membership_revision == 0 {
            return Err(ProductStoreError::InvalidRecord {
                kind: "logical_codebase_manifest",
                reason: "membership_revision must start at 1".to_string(),
            });
        }

        if let Some(existing) = self.load_manifest(project_id)? {
            if existing.logical_codebase_id != manifest.logical_codebase_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "logical_codebase_manifest",
                    id: project_id.to_string(),
                });
            }
            self.validate_membership_revision(&existing, manifest)?;
        }

        write_json(&self.manifest_path(project_id)?, manifest)
    }

    /// Checks the provider root under the manifest writer lock without
    /// creating a manifest. Registration uses this before any member work so
    /// a conflicting batch cannot partially attach.
    pub fn validate_registration_root(
        &self,
        project_id: &str,
        provider_context_root: &Path,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        with_exact_exclusive_lock(
            &self
                .scope_root(project_id)?
                .join(".manifest-registration.lock"),
            || {
                if let Some(manifest) = self.load_manifest(project_id)?
                    && manifest.provider_context_root != provider_context_root
                {
                    return Err(ProductStoreError::Conflict {
                        kind: "aggregate_root_mismatch",
                        id: project_id.to_string(),
                    });
                }
                Ok(())
            },
        )
    }

    /// Runs first-member registration while holding the manifest single-writer
    /// lock. A true first batch gets its manifest and first member in one
    /// critical section; if the operation fails before membership is written,
    /// the provisional empty manifest is removed.
    pub fn with_registration_manifest_writer<T>(
        &self,
        project_id: &str,
        provider_context_root: &Path,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        validate_relative_id(project_id)?;
        with_exact_exclusive_lock(
            &self
                .scope_root(project_id)?
                .join(".manifest-registration.lock"),
            || {
                let created = if let Some(manifest) = self.load_manifest(project_id)? {
                    if manifest.provider_context_root != provider_context_root {
                        return Err(ProductStoreError::Conflict {
                            kind: "aggregate_root_mismatch",
                            id: project_id.to_string(),
                        });
                    }
                    false
                } else {
                    self.save_manifest(
                        project_id,
                        &LogicalCodebaseManifest::new(
                            project_id,
                            provider_context_root.to_path_buf(),
                            Vec::new(),
                        ),
                    )?;
                    true
                };

                match operation() {
                    Ok(value) => Ok(value),
                    Err(error) => {
                        if created
                            && self
                                .load_manifest(project_id)?
                                .is_some_and(|manifest| manifest.member_ids.is_empty())
                        {
                            let path = self.manifest_path(project_id)?;
                            match std::fs::remove_file(&path) {
                                Ok(()) => {}
                                Err(remove_error) if remove_error.kind() == ErrorKind::NotFound => {
                                }
                                Err(remove_error) => {
                                    return Err(ProductStoreError::Io(format!(
                                        "remove provisional manifest {}: {remove_error}",
                                        path.display()
                                    )));
                                }
                            }
                        }
                        Err(error)
                    }
                }
            },
        )
    }

    pub fn save_member(
        &self,
        project_id: &str,
        member: &CodebaseMemberRecord,
    ) -> Result<(), ProductStoreError> {
        write_json(
            &self.member_path(project_id, member.logical_repository_id)?,
            member,
        )
    }

    pub fn load_member(
        &self,
        project_id: &str,
        id: LogicalRepositoryId,
    ) -> Result<Option<CodebaseMemberRecord>, ProductStoreError> {
        let path = self.member_path(project_id, id)?;
        if !path_exists(&path)? {
            return Ok(None);
        }

        let member: CodebaseMemberRecord = read_json(&path)?;
        validate_member_id(id, &member)?;
        Ok(Some(member))
    }

    pub fn list_members(
        &self,
        project_id: &str,
    ) -> Result<Vec<CodebaseMemberRecord>, ProductStoreError> {
        let mut members = Vec::new();
        for (id, path) in
            self.uuid_record_paths(project_id, "members", "logical_codebase_member")?
        {
            let member: CodebaseMemberRecord = read_json(&path)?;
            validate_member_id(LogicalRepositoryId(id), &member)?;
            members.push(member);
        }
        Ok(members)
    }

    pub fn save_checkout(
        &self,
        project_id: &str,
        checkout: &RepositoryCheckoutRecord,
    ) -> Result<(), ProductStoreError> {
        write_json(
            &self.checkout_path(project_id, checkout.checkout_id)?,
            checkout,
        )
    }

    pub fn load_checkout(
        &self,
        project_id: &str,
        id: RepositoryCheckoutId,
    ) -> Result<Option<RepositoryCheckoutRecord>, ProductStoreError> {
        let path = self.checkout_path(project_id, id)?;
        if !path_exists(&path)? {
            return Ok(None);
        }

        let checkout: RepositoryCheckoutRecord = read_json(&path)?;
        validate_checkout_id(id, &checkout)?;
        Ok(Some(checkout))
    }

    pub fn list_checkouts(
        &self,
        project_id: &str,
    ) -> Result<Vec<RepositoryCheckoutRecord>, ProductStoreError> {
        let mut checkouts = Vec::new();
        for (id, path) in self.uuid_record_paths(project_id, "checkouts", "repository_checkout")? {
            let checkout: RepositoryCheckoutRecord = read_json(&path)?;
            validate_checkout_id(RepositoryCheckoutId(id), &checkout)?;
            checkouts.push(checkout);
        }
        Ok(checkouts)
    }

    fn validate_manifest_project(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(&manifest.project_id)?;
        if manifest.project_id != project_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_manifest",
                id: project_id.to_string(),
            });
        }
        Ok(())
    }

    fn validate_membership_revision(
        &self,
        existing: &LogicalCodebaseManifest,
        next: &LogicalCodebaseManifest,
    ) -> Result<(), ProductStoreError> {
        if next.membership_revision < existing.membership_revision {
            return Err(ProductStoreError::InvalidRecord {
                kind: "logical_codebase_manifest",
                reason: format!(
                    "membership_revision cannot decrease from {} to {}",
                    existing.membership_revision, next.membership_revision
                ),
            });
        }

        let maximum_next_revision =
            existing.membership_revision.checked_add(1).ok_or_else(|| {
                ProductStoreError::InvalidRecord {
                    kind: "logical_codebase_manifest",
                    reason: "membership_revision overflow".to_string(),
                }
            })?;
        if next.membership_revision > maximum_next_revision {
            return Err(ProductStoreError::InvalidRecord {
                kind: "logical_codebase_manifest",
                reason: format!(
                    "membership_revision must advance at most one step from {} to {maximum_next_revision}",
                    existing.membership_revision
                ),
            });
        }

        if existing.member_ids != next.member_ids
            && next.membership_revision != maximum_next_revision
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "logical_codebase_manifest",
                reason: format!(
                    "member_ids changed but membership_revision must advance from {} to {maximum_next_revision}",
                    existing.membership_revision
                ),
            });
        }

        Ok(())
    }

    fn tombstone_path(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(logical_codebase_id)?;
        Ok(self
            .paths
            .logical_codebase_record_root(project_id, logical_codebase_id)
            .join("tombstone.json"))
    }

    fn validate_record(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        record: &LogicalCodebaseRecord,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(logical_codebase_id)?;
        validate_relative_id(&record.id)?;
        if record.id != logical_codebase_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_record",
                id: logical_codebase_id.to_string(),
            });
        }
        Ok(())
    }

    fn manifest_path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        Ok(self.scope_root(project_id)?.join("manifest.json"))
    }

    fn member_path(
        &self,
        project_id: &str,
        id: LogicalRepositoryId,
    ) -> Result<PathBuf, ProductStoreError> {
        let file_name = id.0.to_string();
        validate_relative_id(project_id)?;
        validate_relative_id(&file_name)?;
        Ok(self
            .scope_root(project_id)?
            .join("members")
            .join(format!("{file_name}.json")))
    }

    fn checkout_path(
        &self,
        project_id: &str,
        id: RepositoryCheckoutId,
    ) -> Result<PathBuf, ProductStoreError> {
        let file_name = id.0.to_string();
        validate_relative_id(project_id)?;
        validate_relative_id(&file_name)?;
        Ok(self
            .scope_root(project_id)?
            .join("checkouts")
            .join(format!("{file_name}.json")))
    }

    fn uuid_record_paths(
        &self,
        project_id: &str,
        directory: &str,
        kind: &'static str,
    ) -> Result<Vec<(Uuid, PathBuf)>, ProductStoreError> {
        validate_relative_id(project_id)?;
        let root = self.scope_root(project_id)?.join(directory);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };

        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if !entry
                .file_type()
                .map_err(|error| {
                    ProductStoreError::Io(format!("stat {}: {error}", path.display()))
                })?
                .is_file()
            {
                return Err(ProductStoreError::InvalidRecord {
                    kind,
                    reason: format!("record path is not a regular file: {}", path.display()),
                });
            }

            let file_name = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| ProductStoreError::InvalidRecord {
                    kind,
                    reason: format!("record file name is not UTF-8: {}", path.display()),
                })?;
            validate_relative_id(file_name)?;
            let id =
                Uuid::parse_str(file_name).map_err(|error| ProductStoreError::InvalidRecord {
                    kind,
                    reason: format!("record file name is not a UUID: {file_name}: {error}"),
                })?;
            if file_name != id.to_string() {
                return Err(ProductStoreError::InvalidRecord {
                    kind,
                    reason: format!("record file name is not a canonical UUID: {file_name}"),
                });
            }
            paths.push((id, path));
        }
        paths.sort_by_key(|(id, _)| *id);
        Ok(paths)
    }
}

fn legacy_logical_codebase_id(project_id: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(project_id.as_bytes()));
    format!("logical_codebase_{}", &digest[..32])
}

/// A legacy logical codebase is identified by its manifest, not by any stray
/// file (gateway bootstrap artifacts such as `capabilities.json` or
/// `aggregate-policy.json` alone do not constitute a logical codebase).
fn legacy_manifest_exists(root: &Path) -> Result<bool, ProductStoreError> {
    path_exists(&root.join("manifest.json"))
}

fn copy_directory_contents(source: &Path, destination: &Path) -> Result<(), ProductStoreError> {
    std::fs::create_dir_all(destination).map_err(|error| {
        ProductStoreError::Io(format!("create {}: {error}", destination.display()))
    })?;
    for entry in std::fs::read_dir(source)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", source.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", source.display()))
        })?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!("stat {}: {error}", source_path.display()))
        })?;
        if file_type.is_dir() {
            copy_directory_contents(&source_path, &destination_path)?;
        } else if file_type.is_file() && !path_exists(&destination_path)? {
            std::fs::copy(&source_path, &destination_path).map_err(|error| {
                ProductStoreError::Io(format!(
                    "copy {} to {}: {error}",
                    source_path.display(),
                    destination_path.display()
                ))
            })?;
        }
    }
    Ok(())
}

fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    path.try_exists()
        .map_err(|error| ProductStoreError::Io(format!("try_exists {}: {error}", path.display())))
}

fn validate_member_id(
    expected: LogicalRepositoryId,
    member: &CodebaseMemberRecord,
) -> Result<(), ProductStoreError> {
    if member.logical_repository_id != expected {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "logical_codebase_member",
            id: expected.0.to_string(),
        });
    }
    Ok(())
}

fn validate_checkout_id(
    expected: RepositoryCheckoutId,
    checkout: &RepositoryCheckoutRecord,
) -> Result<(), ProductStoreError> {
    if checkout.checkout_id != expected {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "repository_checkout",
            id: expected.0.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
include!("store_tests.inc.rs");
