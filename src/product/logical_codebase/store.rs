use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
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

#[derive(Debug, Clone)]
pub struct LogicalCodebaseStore {
    paths: ProductAppPaths,
}

impl LogicalCodebaseStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
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

    fn manifest_path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        Ok(self
            .paths
            .logical_codebase_root(project_id)
            .join("manifest.json"))
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
            .paths
            .logical_codebase_root(project_id)
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
            .paths
            .logical_codebase_root(project_id)
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
        let root = self.paths.logical_codebase_root(project_id).join(directory);
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
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalRepositoryId,
        RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity,
    };

    #[test]
    fn authority_store_roundtrips_manifest_member_and_checkout_by_uuid_file_name() {
        let temp = tempfile::tempdir().unwrap();
        let store = LogicalCodebaseStore::new(ProductAppPaths::new(temp.path()));
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            temp.path().to_path_buf(),
            vec![member_id],
        );

        store.save_manifest("project_0001", &manifest).unwrap();
        store
            .save_member("project_0001", &member_fixture(member_id, checkout_id))
            .unwrap();
        store
            .save_checkout("project_0001", &checkout_fixture(member_id, checkout_id))
            .unwrap();

        assert_eq!(
            store
                .load_manifest("project_0001")
                .unwrap()
                .unwrap()
                .member_ids,
            vec![member_id]
        );
        assert_eq!(
            store.list_members("project_0001").unwrap()[0].logical_repository_id,
            member_id
        );
        assert_eq!(
            store.list_checkouts("project_0001").unwrap()[0].checkout_id,
            checkout_id
        );
        assert!(
            temp.path()
                .join("projects/project_0001/logical-codebase/members")
                .join(format!("{}.json", member_id.0))
                .exists()
        );
        assert!(
            temp.path()
                .join("projects/project_0001/logical-codebase/checkouts")
                .join(format!("{}.json", checkout_id.0))
                .exists()
        );
    }

    #[test]
    fn changed_members_require_membership_revision_to_increase_by_one() {
        let temp = tempfile::tempdir().unwrap();
        let store = LogicalCodebaseStore::new(ProductAppPaths::new(temp.path()));
        let first_member_id = LogicalRepositoryId(Uuid::new_v4());
        let second_member_id = LogicalRepositoryId(Uuid::new_v4());
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            temp.path().to_path_buf(),
            vec![first_member_id],
        );
        store.save_manifest("project_0001", &manifest).unwrap();

        let mut changed = manifest.clone();
        changed.member_ids.push(second_member_id);
        assert!(matches!(
            store.save_manifest("project_0001", &changed),
            Err(ProductStoreError::InvalidRecord {
                kind: "logical_codebase_manifest",
                ..
            })
        ));

        changed.membership_revision += 1;
        store.save_manifest("project_0001", &changed).unwrap();
        assert_eq!(
            store
                .load_manifest("project_0001")
                .unwrap()
                .unwrap()
                .membership_revision,
            2
        );
    }

    fn member_fixture(
        logical_repository_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
    ) -> CodebaseMemberRecord {
        let now = "2026-08-08T00:00:00Z".to_string();
        CodebaseMemberRecord {
            logical_repository_id,
            physical_repository_id: "repository_0001".to_string(),
            alias: "api".to_string(),
            role: "service".to_string(),
            ordinal: 1,
            source_identity: RepositorySourceIdentity::from_git_parts(
                Path::new("/workspace/api"),
                PathBuf::from("/workspace/api/.git"),
                Some("ssh://git@example.test/acme/api.git".to_string()),
            ),
            repo_type: Default::default(),
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![checkout_id],
            status: Default::default(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    fn checkout_fixture(
        logical_repository_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
    ) -> RepositoryCheckoutRecord {
        let now = "2026-08-08T00:00:00Z".to_string();
        RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id,
            physical_repository_id: "repository_0001".to_string(),
            kind: CheckoutKind::Main,
            canonical_path: PathBuf::from("/workspace/api"),
            checkout_path_hash: "sha256:checkout".to_string(),
            git_dir_identity: "sha256:git-dir".to_string(),
            revision: Some("abc123".to_string()),
            availability: CheckoutAvailability::Available,
            observed_at: now.clone(),
            created_at: now.clone(),
            updated_at: now,
        }
    }
}
