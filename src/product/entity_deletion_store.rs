//! 实体级联删除的最小删除事实与纯 ID 重入索引（fix-cascade-deletion Task D1.0，
//! plan §0.2/§0.5；REQ-DEL-02/06/07）。
//!
//! - 删除记录：`{issue lifecycle root}/entity-deletions/{kind}/{id}.json`，单条
//!   JSON 只含 scope/kind/id/phase/time（REQ-DEL-02：禁止内容/hash/ref/manifest
//!   正文）；`purging→deleted` 同条幂等更新，完成时间不被重试覆盖。
//! - pending manifest 与纯 ID owned-artifact 占用索引位于
//!   `{issue root}/entity-operations/`：plan §0.5 清单之外的顶层操作元数据
//!   目录，永不随实体子树 purge 删除。pending 仅用于失败重入，成功 complete
//!   前移除；owned-index 是长期准入事实（root/pending 均缺后仍可反查 exact
//!   IDs），只保 ID/scope，不保正文/hash。
//! - `begin`/`complete` 只接受合法 [`EntityDeletionLease`]（try-exclusive 持有者）。
//! - 读取错误不当 None：NotFound=未开始/已清，真实 IO/JSON 错误必报。

use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::entity_operation_store::EntityDeletionLease;
use crate::product::json_store::{ProductStoreError, write_json};

/// 被永久删除实体的种类（plan §0.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeletionEntityKind {
    Story,
    Design,
    Plan,
}

impl DeletionEntityKind {
    /// 稳定路径段：`entity-deletions/{segment}/{id}.json`。
    pub fn path_segment(&self) -> &'static str {
        match self {
            Self::Story => "story",
            Self::Design => "design",
            Self::Plan => "plan",
        }
    }
}

/// 实体 scope：`(project, issue, kind, entity_id)` 稳定操作键（plan §0.2）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityScope {
    pub project_id: String,
    pub issue_id: String,
    #[serde(rename = "entity_kind")]
    pub kind: DeletionEntityKind,
    pub entity_id: String,
}

impl EntityScope {
    pub fn new(
        project_id: impl Into<String>,
        issue_id: impl Into<String>,
        kind: DeletionEntityKind,
        entity_id: impl Into<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            issue_id: issue_id.into(),
            kind,
            entity_id: entity_id.into(),
        }
    }

    /// 校验各段为安全相对 ID（scope/path 逃逸拒，plan Task D1.0 Step 1）。
    pub fn validate(&self) -> Result<(), ProductStoreError> {
        crate::product::json_store::validate_relative_id(&self.project_id)?;
        crate::product::json_store::validate_relative_id(&self.issue_id)?;
        crate::product::json_store::validate_relative_id(&self.entity_id)?;
        Ok(())
    }
}

/// 删除阶段：`purging`（清理中，仅允许删除重入）→ `deleted`（完成的稳定事实）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeletionPhase {
    Purging,
    Deleted,
}

/// 单条无内容删除记录（REQ-DEL-02）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityDeletionRecord {
    #[serde(flatten)]
    pub scope: EntityScope,
    pub phase: DeletionPhase,
    pub started_at: String,
    pub deleted_at: Option<String>,
}

/// 外部引用事实（D1.1 `list_incoming_entity_references` 消费，plan §0.2）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityReference {
    pub kind: String,
    pub id: String,
}

/// 纯 ID 删除 manifest（plan §0.2）：只真实 ID，无内容/digest。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityDeletionManifest {
    #[serde(flatten)]
    pub scope: EntityScope,
    pub session_ids: Vec<String>,
    pub work_item_ids: Vec<String>,
    pub review_ids: Vec<String>,
    pub provider_run_ids: Vec<String>,
    pub verification_plan_ids: Vec<String>,
    pub repository_profile_ids: Vec<String>,
    pub target_repository_ids: Vec<String>,
    pub advance_ids: Vec<String>,
    pub advance_retry_command_ids: Vec<String>,
}

impl EntityDeletionManifest {
    pub fn empty(scope: EntityScope) -> Self {
        Self {
            scope,
            session_ids: Vec::new(),
            work_item_ids: Vec::new(),
            review_ids: Vec::new(),
            provider_run_ids: Vec::new(),
            verification_plan_ids: Vec::new(),
            repository_profile_ids: Vec::new(),
            target_repository_ids: Vec::new(),
            advance_ids: Vec::new(),
            advance_retry_command_ids: Vec::new(),
        }
    }

    /// 重入合并：ID 集合取并集（不丢已索引事实），scope 必须一致。
    pub fn merged_with(&self, other: &Self) -> Result<Self, ProductStoreError> {
        if self.scope != other.scope {
            return Err(ProductStoreError::InvalidRecord {
                kind: "entity_deletion_manifest",
                reason: "manifest scope mismatch".to_string(),
            });
        }
        let union = |left: &[String], right: &[String]| -> Vec<String> {
            let ids: BTreeSet<String> = left.iter().chain(right).cloned().collect();
            ids.into_iter().collect()
        };
        Ok(Self {
            scope: self.scope.clone(),
            session_ids: union(&self.session_ids, &other.session_ids),
            work_item_ids: union(&self.work_item_ids, &other.work_item_ids),
            review_ids: union(&self.review_ids, &other.review_ids),
            provider_run_ids: union(&self.provider_run_ids, &other.provider_run_ids),
            verification_plan_ids: union(&self.verification_plan_ids, &other.verification_plan_ids),
            repository_profile_ids: union(
                &self.repository_profile_ids,
                &other.repository_profile_ids,
            ),
            target_repository_ids: union(&self.target_repository_ids, &other.target_repository_ids),
            advance_ids: union(&self.advance_ids, &other.advance_ids),
            advance_retry_command_ids: union(
                &self.advance_retry_command_ids,
                &other.advance_retry_command_ids,
            ),
        })
    }
}

/// 单条删除事实 + pending/纯 ID 索引存储（plan §0.2/§0.5，D1.0）。
#[derive(Debug, Clone)]
pub struct EntityDeletionStore {
    paths: ProductAppPaths,
}

impl EntityDeletionStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    /// 读取删除事实（墓碑）：NotFound=None；真实 IO/JSON 错误不当 None。
    pub fn get(
        &self,
        scope: &EntityScope,
    ) -> Result<Option<EntityDeletionRecord>, ProductStoreError> {
        let path = entity_deletion_record_path(&self.paths, scope)?;
        read_json_if_exists(&path)
    }

    /// purging 开始（幂等）：同条记录 `purging→deleted`；既存记录原样返回
    /// （不重写 started_at/deleted_at）。只接受合法 delete lease。
    pub fn begin(
        &self,
        lease: &EntityDeletionLease,
    ) -> Result<EntityDeletionRecord, ProductStoreError> {
        let scope = lease.scope();
        let path = entity_deletion_record_path(&self.paths, scope)?;
        match read_json_if_exists::<EntityDeletionRecord>(&path)? {
            Some(record) => {
                ensure_record_scope(&record.scope, scope)?;
                Ok(record)
            }
            None => {
                let record = EntityDeletionRecord {
                    scope: scope.clone(),
                    phase: DeletionPhase::Purging,
                    started_at: Utc::now().to_rfc3339(),
                    deleted_at: None,
                };
                write_json(&path, &record)?;
                Ok(record)
            }
        }
    }

    /// 完成（幂等）：首次补 `deleted_at`；重试不覆盖完成时间；pending manifest
    /// 仍存在时拒绝（成功 complete 前必须移除）。
    pub fn complete(
        &self,
        lease: &EntityDeletionLease,
    ) -> Result<EntityDeletionRecord, ProductStoreError> {
        let scope = lease.scope();
        let path = entity_deletion_record_path(&self.paths, scope)?;
        if read_json_if_exists::<EntityDeletionManifest>(&pending_manifest_path(
            &self.paths,
            scope,
        )?)?
        .is_some()
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "entity_pending_manifest",
                reason: "pending manifest must be removed before completion".to_string(),
            });
        }
        match read_json_if_exists::<EntityDeletionRecord>(&path)? {
            None => Err(ProductStoreError::InvalidRecord {
                kind: "entity_deletion_record",
                reason: "complete requires an existing purging record (begin first)".to_string(),
            }),
            Some(record) => {
                ensure_record_scope(&record.scope, scope)?;
                if record.phase == DeletionPhase::Deleted {
                    return Ok(record);
                }
                let mut completed = record;
                completed.phase = DeletionPhase::Deleted;
                completed.deleted_at = Some(Utc::now().to_rfc3339());
                write_json(&path, &completed)?;
                Ok(completed)
            }
        }
    }

    /// purge 前落盘 pending manifest（仅重入用途）。
    pub fn save_pending_manifest(
        &self,
        lease: &EntityDeletionLease,
        manifest: &EntityDeletionManifest,
    ) -> Result<(), ProductStoreError> {
        let scope = lease.scope();
        if &manifest.scope != scope {
            return Err(ProductStoreError::InvalidRecord {
                kind: "entity_deletion_manifest",
                reason: "manifest scope must match the deletion lease".to_string(),
            });
        }
        write_json(&pending_manifest_path(&self.paths, scope)?, manifest)
    }

    pub fn load_pending_manifest(
        &self,
        lease: &EntityDeletionLease,
    ) -> Result<Option<EntityDeletionManifest>, ProductStoreError> {
        read_json_if_exists(&pending_manifest_path(&self.paths, lease.scope())?)
    }

    /// 幂等移除 pending manifest；NotFound=已移除。
    pub fn remove_pending_manifest(
        &self,
        lease: &EntityDeletionLease,
    ) -> Result<(), ProductStoreError> {
        let path = pending_manifest_path(&self.paths, lease.scope())?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ProductStoreError::Io(format!(
                "remove {}: {error}",
                path.display()
            ))),
        }
    }

    /// 纯 ID/scope 占用索引（§0.5）：root/pending 均缺后仍可反查 exact IDs；
    /// 重复索引按并集合并不丢事实。
    pub fn index_owned_artifact_ids(
        &self,
        lease: &EntityDeletionLease,
        manifest: &EntityDeletionManifest,
    ) -> Result<(), ProductStoreError> {
        let scope = lease.scope();
        if &manifest.scope != scope {
            return Err(ProductStoreError::InvalidRecord {
                kind: "entity_deletion_manifest",
                reason: "manifest scope must match the deletion lease".to_string(),
            });
        }
        let path = owned_artifact_index_path(&self.paths, scope)?;
        let merged = match read_json_if_exists::<EntityDeletionManifest>(&path)? {
            Some(existing) => existing.merged_with(manifest)?,
            None => manifest.clone(),
        };
        write_json(&path, &merged)
    }

    /// 读取纯 ID 占用索引；固定路径不存在=未索引/已清，返回空 manifest。
    pub fn list_owned_artifact_ids(
        &self,
        scope: &EntityScope,
    ) -> Result<EntityDeletionManifest, ProductStoreError> {
        let path = owned_artifact_index_path(&self.paths, scope)?;
        Ok(read_json_if_exists::<EntityDeletionManifest>(&path)?
            .unwrap_or_else(|| EntityDeletionManifest::empty(scope.clone())))
    }
}

/// 记录既存 scope 与请求 scope 一致性（同条幂等前提；路径已定 scope，防错档）。
fn ensure_record_scope(
    record_scope: &EntityScope,
    scope: &EntityScope,
) -> Result<(), ProductStoreError> {
    if record_scope == scope {
        return Ok(());
    }
    Err(ProductStoreError::IdentityMismatch {
        kind: "entity_deletion_record",
        id: scope.entity_id.clone(),
    })
}

/// 删除记录路径：`{issue lifecycle root}/entity-deletions/{kind}/{id}.json`。
pub(crate) fn entity_deletion_record_path(
    paths: &ProductAppPaths,
    scope: &EntityScope,
) -> Result<PathBuf, ProductStoreError> {
    scope.validate()?;
    Ok(paths
        .issue_lifecycle_root(&scope.project_id, &scope.issue_id)
        .join("entity-deletions")
        .join(scope.kind.path_segment())
        .join(format!("{}.json", scope.entity_id)))
}

/// 稳定操作元数据目录：`{issue root}/entity-operations/`（§0.5 清单外，永不 purge）。
pub(crate) fn issue_entity_operations_root(
    paths: &ProductAppPaths,
    scope: &EntityScope,
) -> Result<PathBuf, ProductStoreError> {
    scope.validate()?;
    Ok(paths
        .issue_lifecycle_root(&scope.project_id, &scope.issue_id)
        .join("entity-operations"))
}

/// pending manifest 路径：`{issue root}/entity-operations/pending/{kind}/{id}.json`。
fn pending_manifest_path(
    paths: &ProductAppPaths,
    scope: &EntityScope,
) -> Result<PathBuf, ProductStoreError> {
    Ok(issue_entity_operations_root(paths, scope)?
        .join("pending")
        .join(scope.kind.path_segment())
        .join(format!("{}.json", scope.entity_id)))
}

/// 纯 ID 占用索引路径：`{issue root}/entity-operations/owned-index/{kind}/{id}.json`。
fn owned_artifact_index_path(
    paths: &ProductAppPaths,
    scope: &EntityScope,
) -> Result<PathBuf, ProductStoreError> {
    Ok(issue_entity_operations_root(paths, scope)?
        .join("owned-index")
        .join(scope.kind.path_segment())
        .join(format!("{}.json", scope.entity_id)))
}

/// NotFound=None；其余 IO 错误与 JSON 解析错误必报（读取错误不当 None）。
pub(crate) fn read_json_if_exists<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>, ProductStoreError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "open {}: {error}",
                path.display()
            )));
        }
    };
    serde_json::from_reader(file)
        .map(Some)
        .map_err(|error| ProductStoreError::Json(format!("{}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths() -> (tempfile::TempDir, ProductAppPaths) {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ProductAppPaths::new(temp.path().join("aria"));
        (temp, paths)
    }

    fn manifest_for(scope: &EntityScope) -> EntityDeletionManifest {
        EntityDeletionManifest {
            session_ids: vec!["workspace_session_0001".to_string()],
            work_item_ids: vec!["work_item_a".to_string(), "work_item_b".to_string()],
            review_ids: vec!["review_0001".to_string()],
            provider_run_ids: vec!["run_0001".to_string()],
            verification_plan_ids: vec![],
            repository_profile_ids: vec![],
            target_repository_ids: vec![],
            advance_ids: vec![],
            advance_retry_command_ids: vec![],
            scope: scope.clone(),
        }
    }

    #[test]
    fn cascade_d1_0_tombstone_and_lease() {
        let (_temp, paths) = test_paths();
        let operations =
            crate::product::entity_operation_store::EntityOperationStore::new(paths.clone());
        let deletions = EntityDeletionStore::new(paths.clone());
        let scope = EntityScope::new(
            "project_0001",
            "issue_0001",
            DeletionEntityKind::Plan,
            "plan_0009",
        );

        // 未开始：无删除事实。
        assert_eq!(deletions.get(&scope).expect("get before begin"), None);

        // begin → purging（单条记录）。
        let lease = operations
            .try_acquire_delete(&scope)
            .expect("try_acquire_delete");
        let begun = deletions.begin(&lease).expect("begin");
        assert_eq!(begun.phase, DeletionPhase::Purging);
        assert!(begun.deleted_at.is_none());
        assert_eq!(
            deletions
                .get(&scope)
                .expect("get after begin")
                .map(|record| record.phase),
            Some(DeletionPhase::Purging)
        );

        // begin 幂等重入：同条记录、started_at 不变。
        let reentered = deletions.begin(&lease).expect("begin reentry");
        assert_eq!(reentered, begun);

        // pending manifest 落盘/读取/损坏必报。
        let manifest = manifest_for(&scope);
        deletions
            .save_pending_manifest(&lease, &manifest)
            .expect("save pending manifest");
        assert_eq!(
            deletions
                .load_pending_manifest(&lease)
                .expect("load pending"),
            Some(manifest.clone())
        );
        let pending_path = pending_manifest_path(&paths, &scope).expect("pending path");
        std::fs::write(&pending_path, "{ not json").expect("corrupt pending manifest");
        assert!(deletions.load_pending_manifest(&lease).is_err());

        // 纯 ID 占用索引：并集合并不丢事实。
        deletions
            .save_pending_manifest(&lease, &manifest)
            .expect("restore pending manifest");
        deletions
            .index_owned_artifact_ids(&lease, &manifest)
            .expect("index owned artifacts");
        let mut extended = manifest.clone();
        extended.work_item_ids.push("work_item_c".to_string());
        extended.review_ids.push("review_0002".to_string());
        deletions
            .index_owned_artifact_ids(&lease, &extended)
            .expect("reindex owned artifacts");
        let indexed = deletions
            .list_owned_artifact_ids(&scope)
            .expect("list owned artifacts");
        assert_eq!(
            indexed.work_item_ids,
            vec![
                "work_item_a".to_string(),
                "work_item_b".to_string(),
                "work_item_c".to_string()
            ]
        );
        assert_eq!(
            indexed.review_ids,
            vec!["review_0001".to_string(), "review_0002".to_string()]
        );
        assert_eq!(
            indexed.session_ids,
            vec!["workspace_session_0001".to_string()]
        );

        // pending 未移除时 complete 拒绝。
        assert!(matches!(
            deletions.complete(&lease),
            Err(ProductStoreError::InvalidRecord { .. })
        ));

        // 收口：移除 pending → complete → deleted；重试不覆盖完成时间。
        deletions
            .remove_pending_manifest(&lease)
            .expect("remove pending manifest");
        deletions
            .remove_pending_manifest(&lease)
            .expect("remove pending manifest idempotent");
        assert_eq!(
            deletions
                .load_pending_manifest(&lease)
                .expect("load after remove"),
            None
        );
        let completed = deletions.complete(&lease).expect("complete");
        assert_eq!(completed.phase, DeletionPhase::Deleted);
        let deleted_at = completed.deleted_at.clone().expect("deleted_at");
        let recompleted = deletions.complete(&lease).expect("complete idempotent");
        assert_eq!(recompleted.deleted_at.as_deref(), Some(deleted_at.as_str()));
        // begin 在完成后仍幂等返回 deleted 原记录。
        let re_begun = deletions.begin(&lease).expect("begin after complete");
        assert_eq!(re_begun.phase, DeletionPhase::Deleted);
        assert_eq!(re_begun.deleted_at.as_deref(), Some(deleted_at.as_str()));

        // 恰一条记录文件。
        let records_root = paths
            .issue_lifecycle_root("project_0001", "issue_0001")
            .join("entity-deletions")
            .join("plan");
        let mut record_files: Vec<String> = std::fs::read_dir(&records_root)
            .expect("read entity-deletions dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        record_files.sort();
        assert_eq!(record_files, vec!["plan_0009.json".to_string()]);

        // 记录只含 scope/kind/id/phase/time，无任何业务内容字段。
        let raw: serde_json::Value =
            crate::product::json_store::read_json(&records_root.join("plan_0009.json"))
                .expect("read raw record");
        let mut keys: Vec<&str> = raw
            .as_object()
            .expect("record object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "deleted_at",
                "entity_id",
                "entity_kind",
                "issue_id",
                "phase",
                "project_id",
                "started_at",
            ]
        );
    }

    #[test]
    fn cascade_d1_0_scope_path_escape_rejected() {
        let (_temp, paths) = test_paths();
        let operations =
            crate::product::entity_operation_store::EntityOperationStore::new(paths.clone());
        let deletions = EntityDeletionStore::new(paths.clone());

        for scope in [
            EntityScope::new(
                "../evil",
                "issue_0001",
                DeletionEntityKind::Plan,
                "plan_0001",
            ),
            EntityScope::new(
                "project_0001",
                "issue/0001",
                DeletionEntityKind::Story,
                "story_0001",
            ),
            EntityScope::new(
                "project_0001",
                "issue_0001",
                DeletionEntityKind::Design,
                "../escape",
            ),
            EntityScope::new("", "issue_0001", DeletionEntityKind::Plan, "plan_0001"),
        ] {
            assert!(matches!(
                deletions.get(&scope),
                Err(ProductStoreError::PathEscape(_))
            ));
            assert!(matches!(
                operations.try_acquire_delete(&scope),
                Err(
                    crate::product::entity_operation_store::EntityOperationError::Store(
                        ProductStoreError::PathEscape(_)
                    )
                )
            ));
        }
    }
}
